/**
 * ============================================================================
 * ModelScout —— HTTP 客户端封装与探测引擎
 * ============================================================================
 *
 * 【本文件包含三块内容】
 *   1. `ApiClient`      —— reqwest 客户端封装：URL 拼装、鉴权头、统一错误分类
 *   2. 探测原语          —— 单模型的工具调用 / 附件 / 推理 / 上限探测
 *   3. 并发引擎          —— tokio Semaphore + futures buffer_unordered + 中止控制
 *
 * 【安全约束（任务书第五节）】
 *   - 任何日志、事件 payload、错误信息中都**不得**出现完整 API Key。
 *     所有上游返回的正文在对外暴露前必须过 `error::scrub_secrets`。
 *   - Key 只出现在 `ApiClient` 构造的 `Authorization` 头里。
 *
 * 【超时策略（任务书第五节）】
 *   reqwest 客户端统一设置 30s 默认超时（可被 ProbeConfig.timeout_secs 覆盖）。
 *   超时归类为 `ProbeErrorKind::NetworkTimeout`。
 * ============================================================================
 */

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::stream::{self, StreamExt};
use serde_json::{json, Value};
// `Runtime` 用于把本模块的事件发射泛化到任意运行时：
// 生产用 Wry，集成测试用 `tauri::test::MockRuntime`（见 D-37 的测试）。
use tauri::{AppHandle, Emitter, Runtime};
use tokio::sync::{Mutex, Semaphore};

use crate::dictionary;
use crate::error::{redact_key, scrub_secrets, ProbeError, ProbeErrorKind};
use crate::events;
use crate::max_tokens;
use crate::models::{
    DeclaredCaps, LimitSource, ModelBrief, ModelInfo, ProbeConfig, ProbeFatal, ProbeFinished,
    ProbeLog, ProbeModelDone, ProbeProgress, ProbeRunSummary, ProbeSnapshot, ProbeStage,
    ProbeStatus, TriState,
};

/* ===========================================================================
 * 事件端口（依赖倒置）
 * ========================================================================= */

/// 事件发射端口。
///
/// **为什么要这层抽象**：`run_engine` 原本直接依赖 `AppHandle`，
/// 导致它**无法在没有 Tauri 应用的场景下被测试** ——
/// D-37 那个「界面永远 0/0」的 bug 正因如此逃过了全部检查。
///
/// 引入本 trait 后：
///   - 生产环境传 [`TauriSink`]（内部就是 `AppHandle::emit`）；
///   - 测试传一个「记录型 sink」，把事件收进 `Vec` 便于断言。
///
/// 这样引擎与 Tauri 运行时解耦，集成测试**不必启用 `tauri` 的 `test` feature** ——
/// 而正是那个 feature 会拉进完整 GUI 运行时（comctl32 / user32 / dwmapi…），
/// 触发本机的 `STATUS_ENTRYPOINT_NOT_FOUND` 问题（详见 D-37）。
pub trait EventSink: Clone + Send + Sync + 'static {
    /// 推送进度更新
    fn progress(&self, payload: ProbeProgress);
    /// 推送一条日志
    fn log(&self, payload: ProbeLog);
    /// 推送「单模型完成」
    fn model_done(&self, payload: ProbeModelDone);
}

/// 生产实现：把事件转发给 Tauri 应用。
///
/// 手写 `Clone` 而不是 `derive`，避免给 `R` 加上不必要的 `Clone` 约束。
pub struct TauriSink<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> Clone for TauriSink<R> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<R: Runtime> EventSink for TauriSink<R> {
    fn progress(&self, payload: ProbeProgress) {
        let _ = self.0.emit(events::EVENT_PROGRESS, payload);
    }
    fn log(&self, payload: ProbeLog) {
        let _ = self.0.emit(events::EVENT_LOG, payload);
    }
    fn model_done(&self, payload: ProbeModelDone) {
        let _ = self.0.emit(events::EVENT_MODEL_DONE, payload);
    }
}

/* ===========================================================================
 * 常量
 * ========================================================================= */

/// 探测「超大 max_tokens」时使用的值。
/// 取 999999 是因为它超过当前所有已知模型的输出上限，
/// 但又不至于大到被网关按「异常请求」直接 400 掉而丢失有用信息。
const PROBE_MAX_TOKENS: u64 = 999_999;

/// 1x1 像素 PNG 的 Base64（透明像素）。
///
/// 这是附件/视觉能力探测的载荷。选择 1x1 而非更大图片的原因：
///   - token 消耗接近 0
///   - 不会因为图片尺寸触发服务商的图片预处理错误
///   - 只要模型支持 image_url 就会通过；不支持则会明确报参数错误
const PIXEL_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

/// 探测请求里使用的用户消息。极短，降低 token 消耗。
const PROBE_USER_MESSAGE: &str = "hi";

/// 单次探测请求允许的最大输出（避免真实生成消耗额度）。
/// 注意：工具调用探测会临时替换成 PROBE_MAX_TOKENS，见下方实现说明。
const SMALL_MAX_TOKENS: u64 = 16;

/// 拉取 /models 列表时允许的最大超时（列表不消耗生成额度，给足时间）。
const MODELS_LIST_TIMEOUT_SECS: u64 = 60;

/* ===========================================================================
 * 一、ApiClient：reqwest 封装
 * ========================================================================= */

/// OpenAI 兼容 API 的客户端封装。
///
/// 一个 `ApiClient` 实例绑定一份配置（base_url + key + 超时），
/// 内部持有一个 `reqwest::Client`（连接池可复用，是并发探测高效的关键）。
///
/// 克隆代价很低（reqwest::Client 内部是 Arc），因此并发任务各自持有克隆即可。
#[derive(Clone)]
pub struct ApiClient {
    /// reqwest 客户端（内置连接池）
    http: reqwest::Client,
    /// 规范化后的 Base URL（无尾部斜杠）
    base_url: String,
    /// API Key 明文。仅用于构造 Authorization 头，**绝不**打印。
    api_key: String,
    /// 单请求超时
    timeout: Duration,
}

impl ApiClient {
    /// 构造客户端。
    ///
    /// # 参数
    /// - `config`: 已经过 `normalized()` 处理的配置。
    ///
    /// # 返回
    /// 构造失败（TLS 后端初始化异常等）时返回 `ProbeError`。
    pub fn new(config: &ProbeConfig) -> Result<Self, ProbeError> {
        let timeout = Duration::from_secs(config.timeout_secs.max(5));

        let http = reqwest::Client::builder()
            // 总超时（覆盖连接 + 读取）
            .timeout(timeout)
            // 连接阶段单独限制：DNS 出错时能更快失败，提升整体体验
            .connect_timeout(Duration::from_secs(15))
            // 为并发探测留足连接池
            .pool_max_idle_per_host(16)
            .user_agent(concat!("ModelScout/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| {
                ProbeError::new(
                    ProbeErrorKind::Unknown,
                    format!("HTTP 客户端初始化失败：{err}"),
                    0,
                    ProbeStage::Pending,
                )
            })?;

        Ok(Self {
            http,
            base_url: config.base_url.clone(),
            api_key: config.api_key.clone(),
            timeout,
        })
    }

    /// 当前客户端使用的超时（供事件展示）。
    pub fn timeout_secs(&self) -> u64 {
        self.timeout.as_secs()
    }

    /// 拼装完整 URL。
    ///
    /// `path` 应以 `/` 开头，例如 `/chat/completions`。
    /// 结果形如 `https://api.openai.com/v1/chat/completions`。
    pub fn url(&self, path: &str) -> String {
        let p = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        format!("{}{}", self.base_url, p)
    }

    /// 构造一个带鉴权头的 POST 请求。
    ///
    /// **本方法是全项目唯一把 Key 写入请求的地方。**
    fn post(&self, path: &str, body: &Value) -> reqwest::RequestBuilder {
        self.http
            .post(self.url(path))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            // 部分中转网关会因缺少 Accept 头而返回 406
            .header("Accept", "application/json")
            .json(body)
    }

    /// 构造一个带鉴权头的 GET 请求。
    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.http
            .get(self.url(path))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Accept", "application/json")
    }

    /// 构造一个**覆盖单请求超时**的 GET 请求。
    ///
    /// 【为什么需要单独一个方法】
    /// `reqwest::Client` 上设置的 `.timeout()` 是「默认超时」，
    /// 可以被单个 `RequestBuilder` 上的 `.timeout()` 覆盖。
    /// 拉取 `/models` 列表时，某些网关（尤其是聚合了几百个模型的
    /// 中转服务）响应可能远超用户配置的 30s；而列表请求不消耗生成
    /// 额度，给足时间更合理。因此这里以 `MODELS_LIST_TIMEOUT_SECS`
    /// 与用户配置值中的**较大者**作为该请求的超时。
    fn get_with_timeout(&self, path: &str, timeout: Duration) -> reqwest::RequestBuilder {
        self.get(path).timeout(timeout)
    }

    /// 拉取列表类请求应采用的超时（取用户配置与列表专用值的较大者）。
    fn list_request_timeout(&self) -> Duration {
        self.timeout.max(Duration::from_secs(MODELS_LIST_TIMEOUT_SECS))
    }

    /// 发送请求并把响应规约成 `Result<(status, body_text), ProbeError>`。
    ///
    /// 这是所有探测原语的公共底座。它**不把非 2xx 当作 Err**，
    /// 而是把状态码与正文原样返回给调用方 —— 因为很多探测
    /// （尤其是上限探测）**就是要看错误信息**。
    ///
    /// # 返回
    /// - `Ok((status, text))`: 成功拿到 HTTP 响应（无论状态码）
    /// - `Err(err)`: 传输层失败（超时、DNS、TLS 等），已分类
    async fn send(
        &self,
        builder: reqwest::RequestBuilder,
        stage: ProbeStage,
    ) -> Result<(u16, String), ProbeError> {
        match builder.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                match resp.text().await {
                    Ok(text) => Ok((status, text)),
                    Err(err) => Err(ProbeError::from_reqwest_error(&err, stage)),
                }
            }
            Err(err) => Err(ProbeError::from_reqwest_error(&err, stage)),
        }
    }

    /// 发送请求并**要求 2xx**。非 2xx 时把已分类的错误连同正文一起返回。
    ///
    /// 这是需要「必须成功」语义的调用方（如拉取模型列表）使用的入口。
    async fn send_strict(
        &self,
        builder: reqwest::RequestBuilder,
        stage: ProbeStage,
    ) -> Result<(u16, String), ProbeError> {
        let (status, text) = self.send(builder, stage).await?;
        if (200..300).contains(&status) {
            Ok((status, text))
        } else {
            let mut err = ProbeError::from_http_status_with_body(status, stage, &text);
            // 把上游正文摘要附上（已脱敏），帮助用户排障
            let scrubbed = scrub_secrets(&text, &self.api_key);
            let snippet = summarize_text(&scrubbed, 300);
            if !snippet.is_empty() {
                err.message = format!("{} 上游响应：{}", err.message, snippet);
            }
            Err(err)
        }
    }
}

/// 把一段可能很长的文本压成一行摘要。
///
/// 用于错误信息摘要：去掉换行、截断、避免超长文本污染 UI。
fn summarize_text(text: &str, max_chars: usize) -> String {
    let collapsed: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c })
        .collect();
    let trimmed = collapsed.trim();
    // 连续空白合并
    let mut out = String::with_capacity(trimmed.len().min(max_chars));
    let mut prev_space = false;
    for ch in trimmed.chars() {
        if ch == ' ' {
            if prev_space {
                continue;
            }
            prev_space = true;
        } else {
            prev_space = false;
        }
        if out.chars().count() >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

/* ===========================================================================
 * 二、模型列表拉取
 * ========================================================================= */

/// 拉取 `/models` 并解析为轻量条目列表。
///
/// 兼容三种常见返回形态：
///   1. `{ "data": [ { "id": "...", ... } ] }`  ← OpenAI 标准
///   2. `{ "models": [ ... ] }`                  ← 部分网关
///   3. `[ { "id": "..." } ]`                    ← 裸数组（Ollama 某些版本）
pub async fn fetch_model_list(
    client: &ApiClient,
) -> Result<(Vec<ModelBrief>, u64), ProbeError> {
    let started = Instant::now();
    let list_timeout = client.list_request_timeout();
    let (status, text) = client
        .send_strict(
            client.get_with_timeout("/models", list_timeout),
            ProbeStage::Listing,
        )
        .await
        .map_err(|mut err| {
            // 模型列表失败时的状态码补充（404 在列表阶段更可能是 URL 错误）
            if err.kind == ProbeErrorKind::BaseUrlInvalid && status_is_404_hint(&err) {
                err.message = format!(
                    "拉取模型列表返回 404：Base URL（{}）下不存在 /models 接口。\
                     请确认填写的是 API 根地址（通常以 /v1 结尾）。",
                    client.base_url
                );
            }
            err
        })?;

    let elapsed_ms = started.elapsed().as_millis() as u64;

    let parsed: Value = serde_json::from_str(&text).map_err(|err| {
        ProbeError::new(
            ProbeErrorKind::ParseError,
            format!(
                "模型列表不是合法 JSON（HTTP {status}）：{err}。\
                 请确认 Base URL 指向的是 OpenAI 兼容端点。"
            ),
            status,
            ProbeStage::Listing,
        )
    })?;

    let items: Vec<Value> = if let Some(arr) = parsed.as_array() {
        arr.clone()
    } else if let Some(arr) = parsed.get("data").and_then(|v| v.as_array()) {
        arr.clone()
    } else if let Some(arr) = parsed.get("models").and_then(|v| v.as_array()) {
        arr.clone()
    } else {
        return Err(ProbeError::new(
            ProbeErrorKind::ParseError,
            "模型列表 JSON 中未找到 data / models 数组字段。",
            status,
            ProbeStage::Listing,
        ));
    };

    let mut briefs: Vec<ModelBrief> = Vec::with_capacity(items.len());
    for item in items {
        // id 字段是必需的；有些网关用 name 代替
        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("name").and_then(|v| v.as_str()))
            .or_else(|| item.get("model").and_then(|v| v.as_str()))
            .unwrap_or("")
            .trim()
            .to_string();

        if id.is_empty() {
            continue; // 无 id 的条目直接跳过，不污染结果表
        }

        let display_name = item
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| id.clone());

        let owned_by = item
            .get("owned_by")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("ownedBy").and_then(|v| v.as_str()))
            .or_else(|| item.get("publisher").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();

        let created = item
            .get("created")
            .and_then(|v| v.as_i64())
            .or_else(|| item.get("created_at").and_then(|v| v.as_i64()))
            .unwrap_or(0);

        briefs.push(ModelBrief {
            id,
            display_name,
            owned_by,
            created,
            declared: parse_declared_caps(&item),
        });
    }

    // 按 id 排序，保证每次运行顺序稳定（便于对比两次探测结果）
    briefs.sort_by(|a, b| a.id.cmp(&b.id));

    Ok((briefs, elapsed_ms))
}

/// 从 `/models` 的单个条目里解析「接口自报的能力」。
///
/// 兼容 OpenRouter 风格的扩展 schema（详见 [`DeclaredCaps`]）。
/// 任何字段缺失都保持 `None`，交由真实请求探测补足。
///
/// **为什么值得做**：这类网关会把上下文长度、输出上限、是否支持 tools / 图片
/// 直接写在 `/models` 里。直接采信可以：① 省掉每个模型 3~4 次真实请求（省额度）；
/// ② 规避探测请求在这类网关上「不按常理出牌」的响应（例如 502 / 429）。
fn parse_declared_caps(item: &Value) -> DeclaredCaps {
    let mut caps = DeclaredCaps::default();

    // schema 版本（仅排障展示）
    if let Some(v) = item.get("schema_version").and_then(|v| v.as_str()) {
        caps.schema_version = v.to_string();
    }
    // 就绪标记
    caps.is_ready = item.get("is_ready").and_then(|v| v.as_bool());

    // ---------- 输入模态：text → 上下文上限；image → 视觉支持 ----------
    if let Some(mods) = item.get("input_modalities").and_then(|v| v.as_array()) {
        // 只要声明了 input_modalities，就能对「是否支持图片」下结论：
        // 有 image 条目 = 支持；没有 = 不支持。避免留下 None 去发无谓的探测请求。
        let mut has_image = false;
        for m in mods {
            match m.get("type").and_then(|v| v.as_str()) {
                Some("text") => {
                    if let Some(n) = m
                        .get("supported_inputs")
                        .and_then(|v| v.get("max_context_length"))
                        .and_then(|v| v.get("value"))
                        .and_then(|v| v.as_u64())
                    {
                        caps.context_limit = Some(n);
                    }
                }
                Some("image") => has_image = true,
                _ => {}
            }
        }
        caps.supports_vision = Some(has_image);
    }

    // ---------- 输出模态：text → 输出上限 + 工具调用 ----------
    if let Some(mods) = item.get("output_modalities").and_then(|v| v.as_array()) {
        for m in mods {
            if m.get("type").and_then(|v| v.as_str()) != Some("text") {
                continue;
            }
            if let Some(n) = m
                .get("max_length")
                .and_then(|v| v.get("value"))
                .and_then(|v| v.as_u64())
            {
                caps.max_output_limit = Some(n);
            }
            // 关键：`supported_parameters.tools` / `reasoning` 的形态都是
            // `{ "type": "boolean" }`，因此判断依据是「**该键是否存在**」，
            // 而不是它的值。
            // 一旦声明了 supported_parameters：
            //   - 缺失 tools     → 不支持工具调用
            //   - 存在 reasoning → 是推理模型（实测与官方文档 100% 吻合）
            if let Some(params) = m.get("supported_parameters").and_then(|v| v.as_object()) {
                caps.supports_tools = Some(params.contains_key("tools"));
                caps.supports_reasoning = Some(params.contains_key("reasoning"));
            }
        }
    }

    // ---------- 回退：部分网关把上限放在顶层字段 ----------
    if caps.context_limit.is_none() {
        caps.context_limit = item
            .get("context_length")
            .or_else(|| item.get("context_window"))
            .or_else(|| item.get("max_context_length"))
            .and_then(|v| v.as_u64());
    }
    if caps.max_output_limit.is_none() {
        caps.max_output_limit = item
            .get("max_output_length")
            .or_else(|| item.get("max_completion_tokens"))
            .and_then(|v| v.as_u64());
    }

    caps
}

/// 辅助判断：错误信息里是否提到 404。
/// 用于给「列表阶段 404」补充更精确的中文指引。
fn status_is_404_hint(err: &ProbeError) -> bool {
    err.http_status == 404
}

/* ===========================================================================
 * 三、能力探测原语
 * ========================================================================= */

/// 工具调用探测的结果。
#[derive(Debug, Clone)]
pub struct ToolProbeOutcome {
    /// 三态判定
    pub state: TriState,
    /// 判定说明
    pub evidence: String,
    /// 探测过程中产生的错误（若整体失败）
    pub error: Option<ProbeError>,
}

/// 探测模型是否支持工具调用（Function Calling）。
///
/// 【策略】
/// 发送一个带 `tools` 参数的最小请求，然后分三种情况判定：
///
///   1. **HTTP 2xx**：检查响应 JSON 中是否出现 `tool_calls`
///      或 `function_call`。注意：模型**不一定会真的调用**工具
///      （对它来说 "hi" 不需要工具），因此「2xx 但没有 tool_calls」
///      **不能**判定为不支持 —— 能够接受 tools 参数并正常返回，
///      本身就是强证据。故判为 `Supported`，并在 evidence 里说明。
///
///   2. **HTTP 4xx 且报错提到 tools / function**：明确不支持 → `Unsupported`。
///
///   3. **HTTP 4xx/5xx 但报错与 tools 无关**（例如 429 限流、
///      401 Key 失效、model 不存在）：**无法判定** → `Unknown`，
///      并保留错误让前端展示。这正是引入三态而不是 bool 的原因。
pub async fn probe_tools(client: &ApiClient, model_id: &str) -> ToolProbeOutcome {
    let body = json!({
        "model": model_id,
        "messages": [{ "role": "user", "content": PROBE_USER_MESSAGE }],
        // 给足输出空间，便于模型真的吐出 tool_calls
        "max_tokens": SMALL_MAX_TOKENS,
        "tools": [{
            "type": "function",
            "function": {
                "name": "modelscout_ping",
                "description": "A no-op probe function used to detect tool-calling support.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "echo": { "type": "string", "description": "Echo back the value." }
                    },
                    "required": ["echo"]
                }
            }
        }],
        // 显式要求可调用，部分实现默认 auto 也够，但显式更稳
        "tool_choice": "auto"
    });

    let (status, text) = match client.send(client.post("/chat/completions", &body), ProbeStage::ToolProbe).await {
        Ok(pair) => pair,
        Err(err) => {
            return ToolProbeOutcome {
                state: TriState::Unknown,
                evidence: format!("请求失败：{}", err.message),
                error: Some(err),
            }
        }
    };

    if (200..300).contains(&status) {
        // 优先找显式证据：真的返回了 tool_calls
        let (has_tool_calls, has_reasoning) = inspect_response(&text);
        if has_tool_calls {
            return ToolProbeOutcome {
                state: TriState::Supported,
                evidence: "响应中包含 tool_calls，明确支持工具调用。".to_string(),
                error: None,
            };
        }
        if has_reasoning {
            // 顺带记录：模型把内容放进了 reasoning_content
            return ToolProbeOutcome {
                state: TriState::Supported,
                evidence: "端点接受了 tools 参数并正常返回（响应含 reasoning_content，属推理模型）。"
                    .to_string(),
                error: None,
            };
        }
        return ToolProbeOutcome {
            state: TriState::Supported,
            evidence: "端点接受了 tools 参数并成功返回。模型本次未实际调用工具属正常行为，\
                       但能正常处理 tools 请求即为支持。"
                .to_string(),
            error: None,
        };
    }

    // 非 2xx：判断是否与 tools 参数有关
    let lower = text.to_lowercase();
    let mentions_tools = lower.contains("tool")
        || lower.contains("function")
        || lower.contains("tool_choice")
        || text.contains("工具");

    let mut err = ProbeError::from_http_status_with_body(status, ProbeStage::ToolProbe, &text);
    let scrubbed = scrub_secrets(&text, &client.api_key);
    let snippet = summarize_text(&scrubbed, 200);
    if !snippet.is_empty() {
        err.message = format!("{} 上游响应：{}", err.message, snippet);
    }

    if mentions_tools && (400..500).contains(&status) {
        ToolProbeOutcome {
            state: TriState::Unsupported,
            evidence: format!(
                "服务端明确拒绝 tools 参数（HTTP {status}）：{}",
                summarize_text(&scrubbed, 120)
            ),
            error: Some(err),
        }
    } else {
        ToolProbeOutcome {
            state: TriState::Unknown,
            evidence: format!("HTTP {status}，报错与 tools 参数无关，无法判定是否支持。"),
            error: Some(err),
        }
    }
}

/// 解析响应 JSON，返回 (是否含 tool_calls, 是否含 reasoning_content)。
///
/// 同时覆盖两种形态：
///   - OpenAI 标准：`choices[].message.tool_calls` / `.reasoning_content`
///   - 流式残片或非常规实现：在整段文本里做子串回退
fn inspect_response(text: &str) -> (bool, bool) {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        let choices = value.get("choices").and_then(|v| v.as_array());

        let mut has_tool_calls = false;
        let mut has_reasoning = false;

        if let Some(choices) = choices {
            for choice in choices {
                let message = choice.get("message").or_else(|| choice.get("delta"));

                if let Some(msg) = message {
                    // tool_calls 可能为 null，需判非空数组
                    if let Some(tc) = msg.get("tool_calls") {
                        if tc.as_array().map(|a| !a.is_empty()).unwrap_or(false) {
                            has_tool_calls = true;
                        }
                    }
                    // 部分实现把函数调用放在 message.function_call
                    if msg
                        .get("function_call")
                        .map(|v| !v.is_null())
                        .unwrap_or(false)
                    {
                        has_tool_calls = true;
                    }
                    // 推理内容：reasoning_content（DeepSeek）或 thinking（部分网关）
                    if msg
                        .get("reasoning_content")
                        .and_then(|v| v.as_str())
                        .map(|s| !s.trim().is_empty())
                        .unwrap_or(false)
                    {
                        has_reasoning = true;
                    }
                    if msg
                        .get("reasoning")
                        .and_then(|v| v.as_str())
                        .map(|s| !s.trim().is_empty())
                        .unwrap_or(false)
                    {
                        has_reasoning = true;
                    }
                }
            }
        }

        // 顶层也可能直接带 reasoning_content（少见但存在）
        if !has_reasoning {
            if value
                .get("reasoning_content")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
            {
                has_reasoning = true;
            }
        }

        // 注意：JSON 解析成功时**必须直接返回结构化结论**，绝不能落到下面的
        //    子串回退。
        //
        // 【之前为什么错】原实现在这里写的是
        //        if has_tool_calls || has_reasoning { return ...; }
        //    也就是说：当两者都为 false 时会继续往下走，落入子串回退，
        //    于是 `{"...","reasoning_content":""}` 这种「字段存在但为空」
        //    的响应会因为 `text.contains("reasoning_content")` 命中，
        //    被误判成「是推理模型」。
        //
        //    这与探测语义直接冲突：字段存在 ≠ 有推理内容。服务端返回空串
        //    恰恰说明该模型**没有**产出推理，必须判定为 false。
        //
        //    子串回退的本意是「仅在 JSON 完全解析不出来时使用」，
        //    只要 JSON 解析成功，结构化信息永远比子串匹配可信。
        return (has_tool_calls, has_reasoning);
    }

    // ---- 仅在 JSON 解析失败 / 结构完全不符时，才做保守的子串回退 ----
    let lower = text.to_lowercase();
    (
        lower.contains("\"tool_calls\"") && !lower.contains("\"tool_calls\":null"),
        lower.contains("reasoning_content"),
    )
}

/* ---------------------------------------------------------------------------
 * 二之二、推理能力「轻量采样」（当接口已自报其他能力时使用）
 * ------------------------------------------------------------------------- */

/// 轻量推理采样的结果。
#[derive(Debug, Clone)]
pub struct ReasoningSampleOutcome {
    /// 响应中是否出现 `reasoning_content`
    pub has_reasoning: bool,
    /// 判定说明
    pub evidence: String,
    /// 采样过程中的错误（不致命，仅记录）
    pub error: Option<ProbeError>,
}

/// 只发一条最短对话，检查响应里有没有 `reasoning_content`。
///
/// **什么时候用**：当接口已经在 `/models` 里声明了「工具调用 / 视觉 / 上限」时，
/// 就没必要再发带 `tools` 或图片的请求去验证它们 —— 那些请求在这类聚合网关上
/// 常常返回 502 / 429 之类的「非标准」响应，既浪费额度又拿不到有用信息。
/// 此时只保留一次最廉价的对话采样，专门用于判定「是否推理模型」。
pub async fn probe_reasoning_sample(
    client: &ApiClient,
    model_id: &str,
) -> ReasoningSampleOutcome {
    let body = json!({
        "model": model_id,
        "messages": [{ "role": "user", "content": "hi" }],
        "max_tokens": 16
    });

    let (status, text) = match client
        .send(
            client.post("/chat/completions", &body),
            ProbeStage::ReasoningProbe,
        )
        .await
    {
        Ok(pair) => pair,
        Err(err) => {
            return ReasoningSampleOutcome {
                has_reasoning: false,
                evidence: String::new(),
                error: Some(err),
            }
        }
    };

    if !(200..300).contains(&status) {
        let mut err = ProbeError::from_http_status_with_body(status, ProbeStage::ReasoningProbe, &text);
        let scrubbed = scrub_secrets(&text, &client.api_key);
        let snippet = summarize_text(&scrubbed, 200);
        if !snippet.is_empty() {
            err.message = format!("{} 上游响应：{}", err.message, snippet);
        }
        return ReasoningSampleOutcome {
            has_reasoning: false,
            evidence: String::new(),
            error: Some(err),
        };
    }

    // 复用与工具探测相同的响应解析逻辑
    let (_has_tool_calls, has_reasoning) = inspect_response(&text);
    ReasoningSampleOutcome {
        has_reasoning,
        evidence: if has_reasoning {
            "响应中包含 reasoning_content，判定为推理模型。".to_string()
        } else {
            String::new()
        },
        error: None,
    }
}

/// 附件/视觉探测的结果。
#[derive(Debug, Clone)]
pub struct VisionProbeOutcome {
    /// 三态判定
    pub state: TriState,
    /// 判定说明
    pub evidence: String,
    /// 探测过程中的错误
    pub error: Option<ProbeError>,
}

/// 探测模型是否支持附件（图片）上传。
///
/// 【策略】
/// 发送一条包含 `image_url`（1x1 PNG 的 Base64 data URI）的多模态消息。
///
///   - **HTTP 2xx** → 明确支持
///   - **4xx 且报错提到 image / vision / content type / multimodal**
///     → 明确不支持（服务端看懂了请求但拒绝）
///   - **其他 4xx/5xx**（限流、鉴权、模型不存在） → 无法判定
///
/// 【为什么用 1x1 像素】
/// token 消耗接近 0；同时避免因图片尺寸过大触发的服务端预处理错误，
/// 让「拒绝」一定是因为不支持多模态，而不是因为图片本身有问题。
pub async fn probe_vision(client: &ApiClient, model_id: &str) -> VisionProbeOutcome {
    let data_uri = format!("data:image/png;base64,{PIXEL_PNG_BASE64}");

    let body = json!({
        "model": model_id,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": "ok?" },
                { "type": "image_url", "image_url": { "url": data_uri, "detail": "low" } }
            ]
        }],
        "max_tokens": SMALL_MAX_TOKENS
    });

    let (status, text) =
        match client.send(client.post("/chat/completions", &body), ProbeStage::VisionProbe).await {
            Ok(pair) => pair,
            Err(err) => {
                return VisionProbeOutcome {
                    state: TriState::Unknown,
                    evidence: format!("请求失败：{}", err.message),
                    error: Some(err),
                }
            }
        };

    if (200..300).contains(&status) {
        return VisionProbeOutcome {
            state: TriState::Supported,
            evidence: "端点接受了 image_url 多模态消息并成功返回，支持图片附件。".to_string(),
            error: None,
        };
    }

    let scrubbed = scrub_secrets(&text, &client.api_key);
    let lower = scrubbed.to_lowercase();
    let mentions_vision = lower.contains("image")
        || lower.contains("vision")
        || lower.contains("multimodal")
        || lower.contains("content type")
        || lower.contains("content_type")
        || lower.contains("image_url")
        || lower.contains("modality")
        || lower.contains("unsupported content")
        || scrubbed.contains("图片")
        || scrubbed.contains("多模态");

    let mut err = ProbeError::from_http_status_with_body(status, ProbeStage::VisionProbe, &text);
    let snippet = summarize_text(&scrubbed, 200);
    if !snippet.is_empty() {
        err.message = format!("{} 上游响应：{}", err.message, snippet);
    }

    if mentions_vision && (400..500).contains(&status) {
        VisionProbeOutcome {
            state: TriState::Unsupported,
            evidence: format!(
                "服务端拒绝了 image_url 参数（HTTP {status}）：{}",
                summarize_text(&scrubbed, 120)
            ),
            error: Some(err),
        }
    } else {
        VisionProbeOutcome {
            state: TriState::Unknown,
            evidence: format!("HTTP {status}，报错与图片参数无关，无法判定是否支持视觉。"),
            error: Some(err),
        }
    }
}

/// 推理模型名称正则。
///
/// 只匹配**明确的**推理模型标识，避免误伤。
/// 例如不能简单地用 `*o1*` 做子串匹配 —— 那会把 `gpt-4o1`、
/// `llama-3-8b-o1-like` 这类无关名字也判成推理模型。
///
/// 因此全部使用「分隔符边界」或「完整词」形式。
static REASONING_NAME_PATTERNS: &[&str] = &[
    // ---- DeepSeek R1 系列 ----
    // r1 作为独立词段（边界匹配，避免 "mr1x" 之类误命中）
    r"(?i)(^|[/\-_:])r1($|[/\-_:.])",
    r"(?i)deepseek[-_]?reasoner",
    r"(?i)deepseek[-_]?r1",
    // ---- OpenAI o 系列 ----
    //
    // 【为什么 o1/o3 必须限定在开头或紧跟在厂商前缀之后】
    // 之前的写法是 `(^|[/\-_:])o1($|[/\-_:.])`，它会让
    // `vision-o1-like` 命中 —— 因为 `-o1-` 两侧都有分隔符。
    // 但 "vision-o1-like" 语义上是「一个叫 o1 的视觉模型变体」，
    // 并非 OpenAI 的推理模型。这类误判会直接污染探测结果。
    //
    // 精确写法：o 系列标识必须出现在模型 ID 的**起始位置**，
    // 或紧跟在 `vendor/`、`vendor-` 这种明确的厂商命名空间之后。
    // 例：`o1-mini` ✓  `openai/o1` ✓  `azure-o1-preview` ✓
    //     `vision-o1-like` ✗  `my-o3-clone` ✗
    r"(?i)^o1($|[/\-_:.])",
    r"(?i)^o3($|[/\-_:.])",
    r"(?i)^o4[-_]?mini($|[/\-_:.])",
    // 厂商命名空间前缀形态：openai/o1 、msft-o3 等
    r"(?i)^(?:openai|azure|msft|microsoft)[/\-_]+o(?:1|3|4[-_]?mini)($|[/\-_:.])",
    // ---- 通用推理标识 ----
    r"(?i)[\-_/]thinking([\-_/]|$)",
    r"(?i)[\-_/]reason(er|ing)([\-_/]|$)",
    r"(?i)[\-_/]cot([\-_/]|$)",
    // ---- 阿里 QwQ / Qwen3 的思考模式 ----
    r"(?i)^qwq",
    r"(?i)[\-_/]qwen3[\-_]?(think|thinking)",
    // ---- 智谱 ----
    //
    // 【修正】原写法 `[\-_/]glm[\-_]?z1` 要求 glm 前面有分隔符，
    // 但真实模型 ID 是 `glm-z1-air` —— `glm` 位于字符串开头，故永不命中。
    // 正确写法：glm 可在开头（^glm-z1）或在词段边界（/glm-z1）。
    r"(?i)(^|[\-_/])glm[\-_]?z1",
    // ---- 其他 ----
    r"(?i)[\-_/]marco[\-_]?o1",
    r"(?i)[\-_/]exaone[\-_]?deep",
];

/// 惰性编译的推理模式正则集合。
static REASONING_REGEXES: once_cell::sync::Lazy<Vec<regex::Regex>> =
    once_cell::sync::Lazy::new(|| {
        REASONING_NAME_PATTERNS
            .iter()
            .filter_map(|p| regex::Regex::new(p).ok())
            .collect()
    });

/// 依据模型 ID 做推理模型名称判定。
///
/// 返回命中说明；未命中返回 None。
pub fn reasoning_by_name(model_id: &str) -> Option<String> {
    for re in REASONING_REGEXES.iter() {
        if re.is_match(model_id) {
            return Some(format!("模型名称匹配推理标识 /{}/", re.as_str()));
        }
    }
    None
}

/* ===========================================================================
 * 四、上限探测
 * ========================================================================= */

/// 上限探测的结果。
#[derive(Debug, Clone)]
pub struct LimitProbeOutcome {
    /// 上下文窗口上限
    pub context_limit: u64,
    /// 输出上限
    pub max_output_limit: u64,
    /// 来源
    pub source: LimitSource,
    /// 说明（来源依据）
    pub evidence: String,
    /// 探测过程中的错误
    pub error: Option<ProbeError>,
}

/// 通过发送超大 `max_tokens` 触发服务端报错，从报错里解析真实上限。
///
/// 【流程】
///   1. 先用本地字典匹配（本函数调用方已做，这里只处理未命中的情况）。
///   2. 发送 `max_tokens = 999999` 的最小请求。
///   3. 若服务端**直接接受了**（2xx）—— 说明该端点不做上限校验，
///      无法由此反推，返回 Unknown 并说明原因。
///   4. 若报错，则用 `max_tokens::parse_limits` 解析正文。
///      解析到 → `LimitSource::ErrorParse`；解析不到 → `Unknown`。
///
/// 【为什么这是必要的手段】
/// OpenAI 兼容规范里并没有「查询模型上限」的接口，各实现也普遍不提供。
/// 主动触发一次参数校验失败是成本最低、覆盖率最高的做法。
pub async fn probe_limits_by_error(client: &ApiClient, model_id: &str) -> LimitProbeOutcome {
    let body = json!({
        "model": model_id,
        "messages": [{ "role": "user", "content": PROBE_USER_MESSAGE }],
        "max_tokens": PROBE_MAX_TOKENS
    });

    let (status, text) =
        match client.send(client.post("/chat/completions", &body), ProbeStage::LimitProbe).await {
            Ok(pair) => pair,
            Err(err) => {
                return LimitProbeOutcome {
                    context_limit: 0,
                    max_output_limit: 0,
                    source: LimitSource::Unknown,
                    evidence: format!("上限探测请求失败：{}", err.message),
                    error: Some(err),
                }
            }
        };

    let scrubbed = scrub_secrets(&text, &client.api_key);

    if (200..300).contains(&status) {
        // 端点不校验 max_tokens，无法反推。这是**正常**结果，不算错误。
        return LimitProbeOutcome {
            context_limit: 0,
            max_output_limit: 0,
            source: LimitSource::Unknown,
            evidence: format!(
                "端点接受了 max_tokens={PROBE_MAX_TOKENS}（HTTP {status}），\
                 说明其不做上限校验，无法通过报错反推，请手工填写。"
            ),
            error: None,
        };
    }

    let parsed = max_tokens::parse_limits(&scrubbed);

    if parsed.is_empty() {
        // 报错了但没解析出数字
        let mut err =
            ProbeError::from_http_status_with_body(status, ProbeStage::LimitProbe, &scrubbed);
        let snippet = summarize_text(&scrubbed, 200);
        if !snippet.is_empty() {
            err.message = format!("{} 上游响应：{}", err.message, snippet);
        }
        return LimitProbeOutcome {
            context_limit: 0,
            max_output_limit: 0,
            source: LimitSource::Unknown,
            evidence: format!(
                "服务端返回 HTTP {status} 但未能从报错中解析出上限数值，需手工核对。"
            ),
            error: Some(err),
        };
    }

    let ctx = parsed.context_limit.unwrap_or(0);
    let out = parsed.max_output_limit.unwrap_or(0);

    // 一致性回退：只解析到输出上限而没解析到上下文时，
    // 用「输出上限」作为上下文的保守下界（上下文必定 >= 输出上限）。
    let (context_limit, evidence) = if ctx == 0 && out > 0 {
        (
            out,
            format!(
                "{}；未直接解析到上下文窗口，已用输出上限 {} 作为保守下界。",
                parsed.evidence, out
            ),
        )
    } else {
        (ctx, parsed.evidence.clone())
    };

    LimitProbeOutcome {
        context_limit,
        max_output_limit: out,
        source: LimitSource::ErrorParse,
        evidence,
        error: None,
    }
}

/* ===========================================================================
 * 五、并发探测引擎
 * ========================================================================= */

/// 并发引擎的运行期共享状态。
///
/// 使用原子类型而非 Mutex 的原因：这些计数器在每个模型的每个阶段都会更新，
/// 用原子量避免锁竞争，让并发数拉到 10 时也不产生瓶颈。
pub struct RunState {
    /// 中止标志。`stop_probe` 命令置为 true，各任务在每个阶段前检查。
    pub cancelled: AtomicBool,
    /// 已完成模型数
    pub completed: AtomicUsize,
    /// 模型总数
    pub total: AtomicUsize,
    /// 已完成模型的结果缓存（供 `get_probe_state` 快照使用）
    pub results: Mutex<Vec<ModelInfo>>,
    /// 运行 ID
    pub run_id: String,
    /// 运行开始时间（用于计算总耗时）
    pub started: Instant,
    /// 最近一次心跳时间（Unix 毫秒），用于判断任务是否卡死
    pub last_heartbeat_ms: AtomicU64,
}

impl RunState {
    /// 创建一份新的运行状态。
    pub fn new(run_id: String, total: usize) -> Arc<Self> {
        Arc::new(Self {
            cancelled: AtomicBool::new(false),
            completed: AtomicUsize::new(0),
            total: AtomicUsize::new(total),
            results: Mutex::new(Vec::with_capacity(total)),
            run_id,
            started: Instant::now(),
            last_heartbeat_ms: AtomicU64::new(now_ms()),
        })
    }

    /// 是否已被请求中止。
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// 请求中止。
    pub fn request_cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// 刷新心跳。
    pub fn touch(&self) {
        self.last_heartbeat_ms
            .store(now_ms(), Ordering::SeqCst);
    }

    /// 已耗时（毫秒）。
    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }
}

/// 取当前 Unix 毫秒时间戳。
///
/// 刻意不引入 `chrono` 依赖：标准库 SystemTime 足够，
/// 少一个依赖就少一份版本维护负担。
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/* ---------------------------------------------------------------------------
 * 事件发射工具
 * ------------------------------------------------------------------------- */

/// 统一的进度发射函数。
///
/// 所有进度事件都经由此处，保证：
///   1. payload 字段完整（前端无需做空值回退）
///   2. percent 由后端统一计算，避免前端与后端算出不同进度
#[allow(clippy::too_many_arguments)]
fn emit_progress<S: EventSink>(
    app: &S,
    state: &RunState,
    model_id: &str,
    stage: ProbeStage,
    elapsed_ms: u64,
) {
    let total = state.total.load(Ordering::SeqCst);
    let completed = state.completed.load(Ordering::SeqCst);
    let percent = compute_percent(completed, total, stage);

    let payload = ProbeProgress {
        run_id: state.run_id.clone(),
        model_id: model_id.to_string(),
        stage,
        stage_label: stage.label().to_string(),
        completed,
        total,
        percent,
        elapsed_ms,
    };

    // 发射失败（例如窗口已关闭）不应中断探测，由 sink 实现自行忽略
    app.progress(payload);
    state.touch();
}

/// 计算整体完成百分比。
///
/// 计入「当前正在处理的模型已推进的阶段数」，让进度条更平滑：
///   每个模型占 `100 / total` 的份额，模型内部的 5 个阶段再均分该份额。
fn compute_percent(completed: usize, total: usize, stage: ProbeStage) -> u8 {
    if total == 0 {
        return 0;
    }
    let per_model = 1.0 / total as f64;

    // 各阶段在单个模型内部的完成比例
    let inner = match stage {
        ProbeStage::Pending => 0.0,
        ProbeStage::Listing => 0.0,
        ProbeStage::ToolProbe => 0.2,
        ProbeStage::VisionProbe => 0.4,
        ProbeStage::ReasoningProbe => 0.6,
        ProbeStage::LimitProbe => 0.8,
        ProbeStage::Done => 1.0,
        ProbeStage::Failed => 1.0,
    };

    let ratio = (completed as f64 * per_model) + (per_model * inner);
    let percent = (ratio * 100.0).round();
    percent.clamp(0.0, 100.0) as u8
}

/// 统一的日志发射函数。
///
/// 注意：调用方必须保证 `message` 已脱敏。本函数额外做一次保险：
/// 若检测到疑似 Key（`sk-` 开头且长度较长）则整体打码。
fn emit_log<S: EventSink>(
    app: &S,
    state: &RunState,
    level: crate::models::LogLevel,
    model_id: &str,
    message: impl Into<String>,
) {
    let raw = message.into();
    let payload = ProbeLog {
        run_id: state.run_id.clone(),
        level,
        model_id: model_id.to_string(),
        message: sanitize_log_line(&raw),
        timestamp: now_ms(),
    };
    app.log(payload);
    state.touch();
}

/// 对日志正文做最后一道防泄漏处理。
///
/// 此处不做正则全量扫描（成本高且可能误伤），
/// 只处理最典型的情况：出现 `sk-` / `Bearer ` 后跟长串时打码。
fn sanitize_log_line(input: &str) -> String {
    static BEARER_RE: once_cell::sync::Lazy<Option<regex::Regex>> =
        once_cell::sync::Lazy::new(|| {
            regex::Regex::new(r"(?i)(bearer\s+)([A-Za-z0-9\-_\.]{12,})").ok()
        });
    static SK_RE: once_cell::sync::Lazy<Option<regex::Regex>> = once_cell::sync::Lazy::new(|| {
        regex::Regex::new(r"\b(sk-[A-Za-z0-9\-_]{8,})\b").ok()
    });

    let mut out = input.to_string();
    if let Some(re) = BEARER_RE.as_ref() {
        out = re.replace_all(&out, "${1}***REDACTED***").to_string();
    }
    if let Some(re) = SK_RE.as_ref() {
        out = re.replace_all(&out, "***REDACTED***").to_string();
    }
    summarize_text(&out, 500)
}

/* ---------------------------------------------------------------------------
 * 单模型探测流水线
 * ------------------------------------------------------------------------- */

/// 对单个模型执行完整探测流水线，返回填好的 `ModelInfo`。
///
/// 【阶段顺序与理由】
///   1. `ToolProbe`      —— 最基础的兼容性验证。若这一步就 401/404，
///                          后续阶段毫无意义，可直接短路。
///   2. `VisionProbe`    —— 独立能力探测。
///   3. `ReasoningProbe` —— 名称正则 + 复用第 1 步响应里的 reasoning_content。
///                          不额外发请求，省额度。
///   4. `LimitProbe`     —— 字典优先；未命中且用户开启时才发超大 max_tokens。
///
/// 【短路规则】
/// 当第 1 步给出「Key 无效」或「Base URL 错误」这类**全局性**错误时，
/// 跳过后续阶段 —— 它们是端点级问题，不是模型级问题，
/// 对每个模型重复请求只会浪费时间并加重限流。
async fn probe_one_model<S: EventSink>(
    app: S,
    state: Arc<RunState>,
    client: ApiClient,
    brief: ModelBrief,
    probe_limits_enabled: bool,
    semaphore: Arc<Semaphore>,
) -> ModelInfo {
    let model_started = Instant::now();
    let mut info = ModelInfo::from_brief(&brief);
    let mut global_failure = false;

    let _permit = semaphore.acquire().await;

    // 若在排队期间已被中止，直接返回初始态
    if state.is_cancelled() {
        info.errors.push(ProbeError::cancelled(ProbeStage::Pending));
        info.final_stage = ProbeStage::Failed;
        info.elapsed_ms = model_started.elapsed().as_millis() as u64;
        return info;
    }

    // ★ 接口已在 /models 中「自报」能力时，优先采信声明，跳过对应的真实请求探测。
    //   三个好处：① 省额度；② 更快；③ 规避聚合网关上探测请求的「非标准」响应
    //   （本类网关对图片请求返回 502、对超大 max_tokens 返回 429，都拿不到有效信号）。
    let declared = brief.declared.clone();
    if let Some(v) = declared.supports_tools {
        info.supports_tools = TriState::from_bool(v);
    }
    if let Some(v) = declared.supports_vision {
        info.supports_vision = TriState::from_bool(v);
    }
    if let Some(v) = declared.supports_reasoning {
        info.is_reasoning = v;
        info.reasoning_evidence = if v {
            "由接口 /models 声明：该模型支持 reasoning（深度思考）。".to_string()
        } else {
            "由接口 /models 声明：该模型不支持 reasoning（非深度思考模型）。".to_string()
        };
    }
    if let Some(n) = declared.context_limit {
        info.context_limit = n;
        info.context_limit_source = LimitSource::Declared;
    }
    if let Some(n) = declared.max_output_limit {
        info.max_output_limit = n;
        info.max_output_limit_source = LimitSource::Declared;
    }
    if !declared.is_empty() {
        emit_log(
            &app,
            &state,
            crate::models::LogLevel::Info,
            &brief.id,
            format!(
                "接口已声明该模型能力（schema {}）：上下文 {} / 输出 {} / 工具 {} / 视觉 {}，直接采信声明值。",
                if declared.schema_version.is_empty() { "未知" } else { &declared.schema_version },
                declared.context_limit.map(|v| v.to_string()).unwrap_or_else(|| "未声明".into()),
                declared.max_output_limit.map(|v| v.to_string()).unwrap_or_else(|| "未声明".into()),
                declared.supports_tools.map(|v| if v { "支持" } else { "不支持" }).unwrap_or("未声明"),
                declared.supports_vision.map(|v| if v { "支持" } else { "不支持" }).unwrap_or("未声明"),
            ),
        );
    }

    // 汇总「能力数据来源」，供界面展示 —— 让用户知道哪些值是接口说的、哪些是实测的。
    // 这一点很重要：实测发现接口声明可能与官方文档不符（见 D‑34）。
    info.caps_source = if declared.is_empty() {
        "实测探测".to_string()
    } else {
        let mut parts: Vec<&str> = Vec::new();
        if declared.context_limit.is_some() {
            parts.push("上下文");
        }
        if declared.max_output_limit.is_some() {
            parts.push("输出上限");
        }
        if declared.supports_tools.is_some() {
            parts.push("工具调用");
        }
        if declared.supports_vision.is_some() {
            parts.push("视觉");
        }
        if declared.supports_reasoning.is_some() {
            parts.push("推理");
        }
        format!(
            "接口声明（schema {}）：{}",
            if declared.schema_version.is_empty() {
                "未知"
            } else {
                declared.schema_version.as_str()
            },
            parts.join("、")
        )
    };

    // 记录「响应体出现 reasoning_content」的证据：
    // - 走真实工具探测时，从探测响应里取；
    // - 走声明路径时，用一次最廉价的轻量采样取得。
    let mut body_reasoning_evidence: Option<String> = None;

    // ---------------- 阶段 1：工具调用 ----------------
    let stage_started = Instant::now();
    emit_progress(&app, &state, &brief.id, ProbeStage::ToolProbe, 0);

    if declared.supports_tools.is_some() {
        if declared.supports_reasoning.is_some() {
            // 工具调用与推理能力都已声明 → 该模型**一次请求都不用发**
            emit_log(
                &app,
                &state,
                crate::models::LogLevel::Info,
                &brief.id,
                "工具调用与推理能力均已由接口声明，跳过全部探测请求。",
            );
        } else {
            // 已声明工具、但未声明推理 → 发一次最短对话采样，专门判定推理能力。
            // （带 tools 的请求在聚合网关上常被拒，因此不用它）
            emit_log(
                &app,
                &state,
                crate::models::LogLevel::Info,
                &brief.id,
                "工具调用能力已由接口声明，跳过探测请求；改发一次轻量对话以判定推理能力。",
            );
            let sample = probe_reasoning_sample(&client, &brief.id).await;
            if sample.has_reasoning {
                body_reasoning_evidence = Some(sample.evidence.clone());
            }
            if let Some(err) = sample.error {
                info.errors.push(err);
            }
        }
    } else {
        emit_log(
            &app,
            &state,
            crate::models::LogLevel::Info,
            &brief.id,
            "开始探测工具调用支持…",
        );

        let tool_outcome = probe_tools(&client, &brief.id).await;
        info.supports_tools = tool_outcome.state;
        if tool_outcome.evidence.contains("reasoning_content") {
            body_reasoning_evidence = Some("响应中包含 reasoning_content。".to_string());
        }
        if let Some(err) = tool_outcome.error.as_ref() {
            // 全局性错误 → 短路
            if matches!(
                err.kind,
                ProbeErrorKind::KeyInvalid | ProbeErrorKind::BaseUrlInvalid
            ) {
                global_failure = true;
            }
            info.errors.push(err.clone());
        }
    }
    emit_progress(
        &app,
        &state,
        &brief.id,
        ProbeStage::ToolProbe,
        stage_started.elapsed().as_millis() as u64,
    );

    // 检查中止
    if state.is_cancelled() {
        info.errors.push(ProbeError::cancelled(ProbeStage::ToolProbe));
        info.final_stage = ProbeStage::Failed;
        info.elapsed_ms = model_started.elapsed().as_millis() as u64;
        return info;
    }

    if global_failure {
        // 全局错误：写入说明性证据并跳过后续阶段
        info.reasoning_evidence = "因端点在工具调用阶段即返回全局性错误（Key 或 Base URL 问题），\
                                   未继续探测其他能力。"
            .to_string();
        info.supports_vision = TriState::Unknown;
        info.supports_streaming = TriState::Unknown;
        info.context_limit_source = LimitSource::Unknown;
        info.max_output_limit_source = LimitSource::Unknown;
        info.final_stage = ProbeStage::Failed;
        info.elapsed_ms = model_started.elapsed().as_millis() as u64;
        return info;
    }

    // ---------------- 阶段 2：附件/视觉 ----------------
    let stage_started = Instant::now();
    emit_progress(&app, &state, &brief.id, ProbeStage::VisionProbe, 0);

    if declared.supports_vision.is_some() {
        emit_log(
            &app,
            &state,
            crate::models::LogLevel::Info,
            &brief.id,
            "视觉（图片输入）能力已由接口声明，跳过探测请求。",
        );
    } else {
        emit_log(
            &app,
            &state,
            crate::models::LogLevel::Info,
            &brief.id,
            "开始探测附件（图片）上传支持…",
        );

        let vision_outcome = probe_vision(&client, &brief.id).await;
        info.supports_vision = vision_outcome.state;
        if let Some(err) = vision_outcome.error.as_ref() {
            info.errors.push(err.clone());
        }
    }
    emit_progress(
        &app,
        &state,
        &brief.id,
        ProbeStage::VisionProbe,
        stage_started.elapsed().as_millis() as u64,
    );

    if state.is_cancelled() {
        info.errors.push(ProbeError::cancelled(ProbeStage::VisionProbe));
        info.final_stage = ProbeStage::Failed;
        info.elapsed_ms = model_started.elapsed().as_millis() as u64;
        return info;
    }

    // ---------------- 阶段 3：推理能力 ----------------
    let stage_started = Instant::now();
    emit_progress(&app, &state, &brief.id, ProbeStage::ReasoningProbe, 0);

    // 推理能力：接口已声明则直接采信（前面已写入 info.is_reasoning / evidence），
    // 未声明时才用「名称正则 + 响应体」判定。
    if declared.supports_reasoning.is_none() {
        // 3a. 名称正则
        let name_hit = reasoning_by_name(&brief.id);

        // 3b. 响应体内的 reasoning_content（来自工具探测响应，或声明路径下的轻量采样）
        let body_hit = body_reasoning_evidence.is_some();

        match (name_hit, body_hit) {
            (Some(reason), true) => {
                info.is_reasoning = true;
                info.reasoning_evidence = format!("{reason}；且响应中包含 reasoning_content。");
            }
            (Some(reason), false) => {
                info.is_reasoning = true;
                info.reasoning_evidence = reason;
            }
            (None, true) => {
                info.is_reasoning = true;
                info.reasoning_evidence =
                    "响应中包含 reasoning_content，判定为推理模型（名称不含常见推理标识）。"
                        .to_string();
            }
            (None, false) => {
                info.is_reasoning = false;
                info.reasoning_evidence = "名称与响应均未出现推理模型特征。".to_string();
            }
        }
    }

    // 3c. 流式支持：以「探测请求被接受」作为弱证据。
    // 说明：严格验证需要发起 SSE 流式请求，成本高且对部分网关不友好；
    // 这里采用保守推断并在界面标注为「推断」。
    info.supports_streaming = match info.supports_tools {
        TriState::Supported => TriState::Supported,
        TriState::Unsupported => TriState::Supported, // 不接受 tools 仍通常支持流式
        TriState::Unknown => TriState::Unknown,
    };

    emit_progress(
        &app,
        &state,
        &brief.id,
        ProbeStage::ReasoningProbe,
        stage_started.elapsed().as_millis() as u64,
    );

    if state.is_cancelled() {
        info.errors.push(ProbeError::cancelled(ProbeStage::ReasoningProbe));
        info.final_stage = ProbeStage::Failed;
        info.elapsed_ms = model_started.elapsed().as_millis() as u64;
        return info;
    }

    // ---------------- 阶段 4：上下文 / 输出上限 ----------------
    let stage_started = Instant::now();
    emit_progress(&app, &state, &brief.id, ProbeStage::LimitProbe, 0);

    // 4a. 接口已声明 → 直接采信，不发任何请求
    if declared.context_limit.is_some() || declared.max_output_limit.is_some() {
        emit_log(
            &app,
            &state,
            crate::models::LogLevel::Info,
            &brief.id,
            format!(
                "上下文/输出上限已由接口声明：上下文 {} / 输出 {}，跳过探测。",
                if declared.context_limit.is_some() {
                    info.context_limit.to_string()
                } else {
                    "未声明".to_string()
                },
                if declared.max_output_limit.is_some() {
                    info.max_output_limit.to_string()
                } else {
                    "未声明".to_string()
                },
            ),
        );
    } else if let Some(hit) = dictionary::lookup(&brief.id) {
        info.context_limit = hit.context_limit;
        info.max_output_limit = hit.max_output_limit;
        info.context_limit_source = LimitSource::Dictionary;
        info.max_output_limit_source = LimitSource::Dictionary;
        emit_log(
            &app,
            &state,
            crate::models::LogLevel::Info,
            &brief.id,
            format!(
                "命中本地参数字典（模式 {}，{}）：上下文 {} / 输出 {}。",
                hit.matched_pattern,
                if hit.note.is_empty() { "无备注" } else { &hit.note },
                hit.context_limit,
                hit.max_output_limit
            ),
        );
    } else if probe_limits_enabled {
        // 4b. 未命中 → 超大 max_tokens 探针
        emit_log(
            &app,
            &state,
            crate::models::LogLevel::Info,
            &brief.id,
            format!(
                "未命中本地字典，发送 max_tokens={PROBE_MAX_TOKENS} 探针以反推上限…"
            ),
        );

        let limit_outcome = probe_limits_by_error(&client, &brief.id).await;
        info.context_limit = limit_outcome.context_limit;
        info.max_output_limit = limit_outcome.max_output_limit;
        info.context_limit_source = limit_outcome.source;
        info.max_output_limit_source = if limit_outcome.max_output_limit > 0 {
            limit_outcome.source
        } else {
            LimitSource::Unknown
        };
        if let Some(err) = limit_outcome.error.as_ref() {
            info.errors.push(err.clone());
        }
        emit_log(
            &app,
            &state,
            if limit_outcome.source == LimitSource::Unknown {
                crate::models::LogLevel::Warn
            } else {
                crate::models::LogLevel::Info
            },
            &brief.id,
            limit_outcome.evidence.clone(),
        );
    } else {
        // 4c. 用户关闭了上限探测 → 使用保守默认值，并明确标注来源
        info.context_limit = 0;
        info.max_output_limit = 0;
        info.context_limit_source = LimitSource::Unknown;
        info.max_output_limit_source = LimitSource::Unknown;
        emit_log(
            &app,
            &state,
            crate::models::LogLevel::Warn,
            &brief.id,
            "未命中字典且已关闭上限探测，该模型的上下文/输出上限标记为未知。",
        );
    }

    emit_progress(
        &app,
        &state,
        &brief.id,
        ProbeStage::LimitProbe,
        stage_started.elapsed().as_millis() as u64,
    );

    // ---------------- 收尾 ----------------
    info.elapsed_ms = model_started.elapsed().as_millis() as u64;
    info.final_stage = if info.errors.iter().any(|e| e.kind != ProbeErrorKind::Cancelled) {
        ProbeStage::Failed
    } else {
        ProbeStage::Done
    };

    // 释放信号量（permits 会在 drop 时自动归还，这里显式 drop 使语义更清晰）
    drop(_permit);

    info
}

/* ---------------------------------------------------------------------------
 * 并发主引擎
 * ------------------------------------------------------------------------- */

/// 并发探测引擎的配置。
pub struct EngineOptions {
    /// 是否执行上限探测
    pub probe_limits: bool,
}

/// 引擎的执行结果。
pub struct EngineOutcome {
    /// 成功/失败模型的完整列表
    pub models: Vec<ModelInfo>,
    /// 全局错误
    pub errors: Vec<ProbeError>,
    /// 终态
    pub status: ProbeStatus,
    /// 总耗时（毫秒）
    pub elapsed_ms: u64,
}

/// 运行并发探测引擎。
///
/// 【并发实现】
/// 采用「Semaphore 限流 + buffer_unordered 流式调度」的组合：
///
///   - `Semaphore` 在 `probe_one_model` 内部获取许可，因此**真正的**
///     并发请求数被严格限制在 `concurrency` 以内。
///   - `buffer_unordered(concurrency)` 控制同时在飞的任务数，
///     使得内存占用与任务数无关（不会一次性 spawn 几百个 future）。
///
/// 为什么两者都要？
///   仅用 buffer_unordered 时，若某任务的 await 点在网络等待，
///   并发度由 buffer 大小决定，但无法阻止「已完成网络等待、
///   正在做 CPU 工作」的任务继续占用额度。
///   加一层 Semaphore 后，「网络请求」这一最稀缺的资源被精确限流。
///
/// 【中止】
/// 每个任务在阶段边界检查 `state.cancelled`，一旦置位立即返回。
/// 已发出的 HTTP 请求无法取消（reqwest 不提供 per-request cancel），
/// 但它们会在超时后自然结束，不会阻塞主流程返回。
/// 从运行状态中取出「已完成的模型」，作为结果的**唯一事实来源**。
///
/// 单独抽成函数有两个目的：
///   1. 让 `run_engine` 不再维护第二个累积变量 ——
///      D-37 那个「界面永远 0/0」的 bug 正是两份变量只写了一份导致的；
///   2. 本函数只依赖 `RunState`、**不需要 AppHandle**，
///      因此可以直接单元测试。这是 D-37 之后补上的、
///      真正覆盖「结果收集路径」的测试（此前 36 个单测全是纯函数，无一覆盖此处）。
async fn collect_results(state: &Arc<RunState>) -> Vec<ModelInfo> {
    let mut out = {
        let guard = state.results.lock().await;
        guard.clone()
    };
    // 按 id 排序，保证前端表格顺序稳定（便于对比两次探测结果）
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

pub async fn run_engine<S: EventSink>(
    app: S,
    state: Arc<RunState>,
    client: ApiClient,
    briefs: Vec<ModelBrief>,
    concurrency: usize,
    options: EngineOptions,
) -> EngineOutcome {
    let started = Instant::now();
    let total = briefs.len();

    emit_log(
        &app,
        &state,
        crate::models::LogLevel::Info,
        "",
        format!(
            "开始并发探测 {total} 个模型，并发数 {concurrency}，单请求超时 {}s，上限探测{}。",
            client.timeout_secs(),
            if options.probe_limits { "已开启" } else { "已关闭" }
        ),
    );

    if total == 0 {
        return EngineOutcome {
            models: Vec::new(),
            errors: vec![ProbeError::new(
                ProbeErrorKind::Unknown,
                "模型列表为空，没有可探测的模型。请确认 Base URL 与 API Key。",
                0,
                ProbeStage::Listing,
            )],
            status: ProbeStatus::Failed,
            elapsed_ms: started.elapsed().as_millis() as u64,
        };
    }

    // 并发度取 max(1)，防止 buffer_unordered(0) 导致流永不推进
    let concurrency = concurrency.max(1);
    let semaphore = Arc::new(Semaphore::new(concurrency));

    // 逐模型构造 future，交给 buffer_unordered 流式调度
    let futures_iter = briefs.into_iter().map(|brief| {
        let app = app.clone();
        let state = state.clone();
        let client = client.clone();
        let semaphore = semaphore.clone();
        let probe_limits = options.probe_limits;

        async move {
            probe_one_model(app, state, client, brief, probe_limits, semaphore).await
        }
    });

    let mut stream = stream::iter(futures_iter).buffer_unordered(concurrency);
    let mut global_errors: Vec<ProbeError> = Vec::new();

    while let Some(info) = stream.next().await {
        // ★★ 关键：把本次结果写入 `state.results` —— 它是结果的**唯一事实来源**。
        //
        // 历史上这里曾把结果只写进一个局部的 `collected` 变量，而 `state.results`
        // 另写一份，两份累积变量只写了一份，导致返回值恒为空、界面永远显示
        // 「0/0 暂无探测结果」（见 D-37）。
        // 现在统一为单一出口：循环只写这里，返回值由 `collect_results()` 派生。
        {
            let mut guard = state.results.lock().await;
            guard.push(info.clone());
        }
        state.completed.fetch_add(1, Ordering::SeqCst);

        // 收集全局性错误（去重）
        for err in info.errors.iter() {
            if matches!(
                err.kind,
                ProbeErrorKind::KeyInvalid | ProbeErrorKind::BaseUrlInvalid
            ) && !global_errors
                .iter()
                .any(|existing| existing.kind == err.kind)
            {
                global_errors.push(err.clone());
            }
        }

        let completed = state.completed.load(Ordering::SeqCst);

        // 推送单模型完成事件
        let done_payload = ProbeModelDone {
            run_id: state.run_id.clone(),
            model: info.clone(),
            completed,
            total,
        };
        app.model_done(done_payload);

        // 推送 100% 的进度（让进度条走满）
        emit_progress(&app, &state, &info.id, ProbeStage::Done, info.elapsed_ms);

        emit_log(
            &app,
            &state,
            crate::models::LogLevel::Info,
            &info.id,
            format!(
                "完成（{completed}/{total}）：工具={} 附件={} 推理={} 上下文={} 输出={}（耗时 {}ms）",
                tri_label(info.supports_tools),
                tri_label(info.supports_vision),
                if info.is_reasoning { "是" } else { "否" },
                if info.context_limit > 0 {
                    info.context_limit.to_string()
                } else {
                    "未知".to_string()
                },
                if info.max_output_limit > 0 {
                    info.max_output_limit.to_string()
                } else {
                    "未知".to_string()
                },
                info.elapsed_ms
            ),
        );

        // 中止：停止消费剩余 future。
        // drop(stream) 会取消尚未开始的任务，已在飞的请求随其超时自然结束。
        if state.is_cancelled() {
            emit_log(
                &app,
                &state,
                crate::models::LogLevel::Warn,
                "",
                format!("收到中止请求，停止派发剩余任务（已完成 {completed}/{total}）。"),
            );
            break;
        }
    }

    // 结果**唯一来源**：直接从 `state.results` 派生（见 D-37）。
    // 不再维护第二个累积变量 —— 那正是「只写一份、另一份恒空」bug 的温床。
    let collected = collect_results(&state).await;

    let elapsed_ms = started.elapsed().as_millis() as u64;
    let status = if state.is_cancelled() {
        ProbeStatus::Cancelled
    } else if collected.is_empty() {
        ProbeStatus::Failed
    } else {
        ProbeStatus::Done
    };

    EngineOutcome {
        models: collected,
        errors: global_errors,
        status,
        elapsed_ms,
    }
}

/// 三态标签的中文短名（用于日志）。
fn tri_label(state: TriState) -> &'static str {
    match state {
        TriState::Supported => "支持",
        TriState::Unsupported => "不支持",
        TriState::Unknown => "未知",
    }
}

/* ---------------------------------------------------------------------------
 * 快照构造
 * ------------------------------------------------------------------------- */

/// 依据运行状态构造一份快照（供 `get_probe_state` 使用）。
pub async fn build_snapshot(
    state: Option<&Arc<RunState>>,
    status: ProbeStatus,
) -> ProbeSnapshot {
    match state {
        Some(state) => {
            let (models, completed) = {
                let guard = state.results.lock().await;
                let mut sorted = guard.clone();
                sorted.sort_by(|a, b| a.id.cmp(&b.id));
                let done = sorted.len();
                (sorted, done)
            };
            ProbeSnapshot {
                is_running: status == ProbeStatus::Running || status == ProbeStatus::Cancelling,
                run_id: state.run_id.clone(),
                status,
                total_models: state.total.load(Ordering::SeqCst),
                completed_models: completed,
                models,
            }
        }
        None => ProbeSnapshot {
            is_running: false,
            run_id: String::new(),
            status: ProbeStatus::Idle,
            total_models: 0,
            completed_models: 0,
            models: Vec::new(),
        },
    }
}

/// 构造「致命错误」事件并发射。
pub fn emit_fatal<R: Runtime>(
    app: &AppHandle<R>,
    run_id: &str,
    error: ProbeError,
    stage: ProbeStage,
) {
    let payload = ProbeFatal {
        run_id: run_id.to_string(),
        error,
        stage,
    };
    let _ = app.emit(events::EVENT_ERROR, payload);
}

/// 构造「整轮结束」事件并发射。
pub fn emit_finished<R: Runtime>(
    app: &AppHandle<R>,
    run_id: &str,
    status: ProbeStatus,
    completed: usize,
    total: usize,
    elapsed_ms: u64,
    errors: Vec<ProbeError>,
) {
    let payload = ProbeFinished {
        run_id: run_id.to_string(),
        status,
        completed,
        total,
        elapsed_ms,
        errors,
    };
    let _ = app.emit(events::EVENT_FINISHED, payload);
}

/// 由引擎结果构造 `ProbeRunSummary`。
pub fn build_summary(run_id: &str, outcome: EngineOutcome) -> ProbeRunSummary {
    ProbeRunSummary {
        run_id: run_id.to_string(),
        status: outcome.status,
        total_models: outcome.models.len(),
        models: outcome.models,
        errors: outcome.errors,
        elapsed_ms: outcome.elapsed_ms,
    }
}

/* ---------------------------------------------------------------------------
 * 便于外部使用的辅助函数
 * ------------------------------------------------------------------------- */

/// 生成一个新的运行 ID。
///
/// 用 Unix 毫秒时间戳的字符串形式：单调性足够好，且人类可读，
/// 便于在日志里把一次运行的所有事件串起来。
pub fn new_run_id() -> String {
    now_ms().to_string()
}

/// 仅供调试：返回当前配置的 Key 脱敏形式（用于启动自检日志）。
pub fn describe_config(config: &ProbeConfig) -> String {
    format!(
        "baseUrl={} apiKey={} concurrency={} timeout={}s probeLimits={}",
        config.base_url,
        redact_key(&config.api_key),
        config.concurrency,
        config.timeout_secs,
        config.probe_limits
    )
}

/* ---------------------------------------------------------------------------
 * 单元测试
 * ------------------------------------------------------------------------- */

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归测试：接口自报能力必须被正确解析。
    /// 样本取自真实聚合网关（discovery-api）的 `/models` 返回。
    #[test]
    fn declared_caps_are_parsed_from_extended_schema() {
        let item = json!({
            "schema_version": "2.4",
            "id": "glm-5.3",
            "input_modalities": [
                { "type": "text",
                  "supported_inputs": { "max_context_length": { "value": 1048576, "unit": "token" } } },
                { "type": "image",
                  "supported_inputs": { "sources": { "type": "enum", "values": ["url", "base64"] } } }
            ],
            "output_modalities": [
                { "type": "text",
                  "max_length": { "value": 1048576, "unit": "token" },
                  "supported_parameters": {
                      "max_tokens": { "type": "integer", "min": 1, "max": 1048576 },
                      "reasoning": { "type": "boolean" },
                      "tools": { "type": "boolean" }
                  } }
            ],
            "is_ready": true
        });
        let caps = parse_declared_caps(&item);
        assert_eq!(caps.context_limit, Some(1048576));
        assert_eq!(caps.max_output_limit, Some(1048576));
        assert_eq!(caps.supports_tools, Some(true));
        assert_eq!(caps.supports_vision, Some(true));
        assert_eq!(caps.supports_reasoning, Some(true));
        assert_eq!(caps.is_ready, Some(true));
        assert_eq!(caps.schema_version, "2.4");
        assert!(!caps.is_empty());
    }

    /// 纯文本模型：无 image 模态 → 视觉应判定为「不支持」而非留空；
    /// `supported_parameters` 里没有 `tools` → 工具调用同样判定为「不支持」。
    /// 这样才不会再去发无谓的探测请求。
    #[test]
    fn declared_caps_mark_absent_capabilities_as_unsupported() {
        let item = json!({
            "id": "text-only",
            "input_modalities": [
                { "type": "text",
                  "supported_inputs": { "max_context_length": { "value": 131072 } } }
            ],
            "output_modalities": [
                { "type": "text",
                  "max_length": { "value": 8192 },
                  "supported_parameters": { "temperature": { "type": "range" } } }
            ]
        });
        let caps = parse_declared_caps(&item);
        assert_eq!(caps.context_limit, Some(131072));
        assert_eq!(caps.max_output_limit, Some(8192));
        assert_eq!(caps.supports_tools, Some(false));
        assert_eq!(caps.supports_vision, Some(false));
        // supported_parameters 存在但没有 reasoning 键 → 判定为非推理模型
        assert_eq!(caps.supports_reasoning, Some(false));
    }

    /// 标准 OpenAI 列表条目（无扩展字段）→ 全部为空，表示仍需真实请求探测。
    /// 这是保证「普通 OpenAI 兼容接口行为不退化」的关键测试。
    #[test]
    fn plain_openai_entry_yields_no_declared_caps() {
        let item = json!({ "id": "gpt-4o", "object": "model", "owned_by": "openai" });
        let caps = parse_declared_caps(&item);
        assert!(caps.is_empty(), "标准 OpenAI 条目不应产生任何声明");
        assert_eq!(caps.supports_tools, None);
        assert_eq!(caps.supports_vision, None);
        assert_eq!(caps.context_limit, None);
    }

    /// 构造一个最小可用的 `ModelInfo`，仅供测试。
    fn test_model_info(id: &str) -> ModelInfo {
        ModelInfo::from_brief(&ModelBrief {
            id: id.to_string(),
            display_name: id.to_string(),
            owned_by: "test".to_string(),
            created: 0,
            declared: DeclaredCaps::default(),
        })
    }

    /// **回归测试（D-37）：结果收集路径必须把「已完成模型」如实取出。**
    ///
    /// 这个测试存在的唯一理由：`run_engine` 曾经维护两个累积变量
    /// （局部的 `collected` 与共享的 `state.results`），却只写了一份，
    /// 导致命令返回值恒为空、界面永远显示「0/0 暂无探测结果」，
    /// 而日志里每个模型都显示「完成」。
    ///
    /// 此前 36 个单测全是纯函数测试（解析 / 正则 / 脱敏…），
    /// **没有任何一个覆盖这条路径**，所以那个 bug 才逃过了全部检查。
    #[tokio::test]
    async fn collect_results_returns_every_finished_model() {
        let state = RunState::new("run-test".to_string(), 2);

        // 尚未有结果时，应为空
        assert!(collect_results(&state).await.is_empty());

        // 故意乱序推入两个模型
        {
            let mut guard = state.results.lock().await;
            guard.push(test_model_info("zeta"));
            guard.push(test_model_info("alpha"));
        }

        let out = collect_results(&state).await;
        assert_eq!(out.len(), 2, "已完成的模型必须一个不少地被取出");
        assert_eq!(out[0].id, "alpha", "结果应按 id 排序");
        assert_eq!(out[1].id, "zeta");

        // 幂等：不应把 state.results 取空（快照 / 重新同步还要用它）
        let again = collect_results(&state).await;
        assert_eq!(
            again.len(),
            2,
            "collect_results 不应清空 state.results（快照路径仍在依赖它）"
        );
    }

    #[test]
    fn headers_and_url_are_built_correctly() {
        let config = ProbeConfig {
            base_url: "https://api.example.com/v1".to_string(),
            api_key: "sk-test-abcdefghijklmnop".to_string(),
            concurrency: 3,
            timeout_secs: 30,
            probe_limits: true,
            only_models: vec![],
            api_key_source: crate::models::ApiKeySource::Plain,
            api_key_env_var: String::new(),
            api_key_credential: String::new(),
        };
        let client = ApiClient::new(&config).expect("客户端应能构造");
        assert_eq!(
            client.url("/chat/completions"),
            "https://api.example.com/v1/chat/completions"
        );
        // 不带头部斜杠的路径也应正确拼接
        assert_eq!(client.url("models"), "https://api.example.com/v1/models");
        assert_eq!(client.timeout_secs(), 30);
    }

    #[test]
    fn reasoning_name_detection_is_precise() {
        // 应识别的推理模型
        assert!(reasoning_by_name("deepseek-r1").is_some());
        assert!(reasoning_by_name("deepseek-reasoner").is_some());
        assert!(reasoning_by_name("o1-mini").is_some());
        assert!(reasoning_by_name("o3-mini").is_some());
        assert!(reasoning_by_name("qwq-32b").is_some());
        assert!(reasoning_by_name("qwen3-235b-thinking").is_some());
        assert!(reasoning_by_name("glm-z1-air").is_some());
        assert!(reasoning_by_name("model-thinking-7b").is_some());

        // 不应误判的普通模型
        assert!(reasoning_by_name("gpt-4o").is_none(), "gpt-4o 不是推理模型");
        assert!(
            reasoning_by_name("gpt-4o-mini").is_none(),
            "gpt-4o-mini 不是推理模型"
        );
        assert!(
            reasoning_by_name("vision-o1-like").is_none(),
            "o1 出现在非边界位置不应命中"
        );
        assert!(reasoning_by_name("llama-3.3-70b").is_none());
        assert!(reasoning_by_name("qwen2.5-72b").is_none());
    }

    #[test]
    fn response_inspection_detects_tool_calls() {
        let with_tools = r#"{"choices":[{"message":{"role":"assistant","tool_calls":[{"id":"1","type":"function","function":{"name":"modelscout_ping","arguments":"{}"}}]}}]}"#;
        assert_eq!(inspect_response(with_tools).0, true);

        let empty_tools = r#"{"choices":[{"message":{"role":"assistant","content":"hi","tool_calls":null}}]}"#;
        assert_eq!(inspect_response(empty_tools).0, false);

        let plain = r#"{"choices":[{"message":{"role":"assistant","content":"hello"}}]}"#;
        assert_eq!(inspect_response(plain), (false, false));
    }

    #[test]
    fn response_inspection_detects_reasoning_content() {
        let with_reasoning = r#"{"choices":[{"message":{"role":"assistant","content":"","reasoning_content":"让我想想…"}}]}"#;
        assert_eq!(inspect_response(with_reasoning).1, true);

        let empty_reasoning = r#"{"choices":[{"message":{"role":"assistant","content":"hi","reasoning_content":""}}]}"#;
        assert_eq!(inspect_response(empty_reasoning).1, false);
    }

    #[test]
    fn percent_progress_is_monotonic_and_bounded() {
        assert_eq!(compute_percent(0, 10, ProbeStage::Pending), 0);
        assert_eq!(compute_percent(0, 10, ProbeStage::ToolProbe), 2);
        assert_eq!(compute_percent(0, 10, ProbeStage::LimitProbe), 8);
        assert_eq!(compute_percent(5, 10, ProbeStage::Done), 60);
        assert_eq!(compute_percent(10, 10, ProbeStage::Done), 100);
        // 边界：总数为 0 不应 panic
        assert_eq!(compute_percent(0, 0, ProbeStage::Done), 0);
    }

    #[test]
    fn log_sanitizer_removes_keys() {
        let line = "request failed with Authorization: Bearer sk-proj-1234567890abcdef";
        let sanitized = sanitize_log_line(line);
        assert!(!sanitized.contains("sk-proj-1234567890abcdef"));
        assert!(sanitized.contains("REDACTED"));

        let bare = "using key sk-abcdefghijklmnop1234 for request";
        let sanitized2 = sanitize_log_line(bare);
        assert!(!sanitized2.contains("sk-abcdefghijklmnop1234"));
    }

    #[test]
    fn describe_config_never_leaks_key() {
        let config = ProbeConfig {
            base_url: "https://api.example.com/v1".to_string(),
            api_key: "sk-super-secret-key-value-12345".to_string(),
            concurrency: 3,
            timeout_secs: 30,
            probe_limits: true,
            only_models: vec![],
            api_key_source: crate::models::ApiKeySource::Plain,
            api_key_env_var: String::new(),
            api_key_credential: String::new(),
        };
        let desc = describe_config(&config);
        assert!(!desc.contains("sk-super-secret-key-value-12345"));
        assert!(desc.contains("apiKey="));
    }

    #[test]
    fn summarize_text_collapses_and_truncates() {
        let long = "a\n\n\nb   c ".to_string() + &"x".repeat(500);
        let s = summarize_text(&long, 20);
        assert!(s.chars().count() <= 21, "摘要长度应受限：{}", s.chars().count());
        assert!(!s.contains('\n'));
    }
}
