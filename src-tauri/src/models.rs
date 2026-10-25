//! 数据契约模块：Rust 侧「唯一事实来源」。
//!
//! 核心约定：本文件中的每一个对外结构体都必须带
//! `#[derive(Debug, Clone, Serialize, Deserialize)]` 与
//! `#[serde(rename_all = "camelCase")]`——否则前端拿到的是 snake_case
//! 字段，与 `src/types/contracts.ts` 不匹配（Tauri v2 的常见陷阱）。
//! 字段顺序与 `contracts.ts` 中的接口保持一致，便于逐一交叉核对。

use serde::{Deserialize, Serialize};

use crate::error::ProbeError;

// ---- 基础枚举 ----

/// 能力探测的三态结果。
///
/// 与前端 `TriState` 对应。用三态而非 bool 的原因：
/// 探测失败（例如上游 500）时无法判定支持与否，
/// 强行记为 false 会误导用户，故保留 `Unknown`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TriState {
    /// 明确探测到支持
    Supported,
    /// 明确探测到不支持
    Unsupported,
    /// 无法判定（探测本身失败）
    Unknown,
}

impl TriState {
    /// 便捷构造：bool → TriState
    pub fn from_bool(value: bool) -> Self {
        if value {
            TriState::Supported
        } else {
            TriState::Unsupported
        }
    }
}

/// 上下文 / 输出上限的取值来源。
///
/// 与前端 `LimitSource` 对应。前端据此在表格中展示不同的可信度标签。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LimitSource {
    /// 接口在 `/models` 中直接声明（最可信：服务端自己说的）
    Declared,
    /// 命中本地参数字典，可信度高
    Dictionary,
    /// 通过超大 max_tokens 触发报错后解析得出
    ErrorParse,
    /// 使用回退值
    Fallback,
    /// 未能获取
    Unknown,
}

/// 探测任务整体状态机。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProbeStatus {
    /// 空闲，未启动
    Idle,
    /// 正在运行
    Running,
    /// 已收到中止请求，正在收敛
    Cancelling,
    /// 正常完成
    Done,
    /// 被用户中止
    Cancelled,
    /// 因致命错误失败
    Failed,
}

impl Default for ProbeStatus {
    /// 默认状态为「空闲」。
    ///
    /// 【为什么手写而不 derive(Default)】
    /// `#[derive(Default)]` 会把默认值绑定到枚举的第一个变体，
    /// 一旦将来有人在 `Idle` 之前插入新变体，默认值就会被静默改变。
    /// 显式实现可以在编译期锁定语义，避免这种脆弱的隐式依赖。
    fn default() -> Self {
        ProbeStatus::Idle
    }
}

/// 单个模型在探测流水线中的阶段。
///
/// 顺序即执行顺序，前端进度面板照此展示阶段名。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProbeStage {
    /// 排队等待
    Pending,
    /// 正在拉取模型列表
    Listing,
    /// 工具调用探测中
    ToolProbe,
    /// 附件/视觉探测中
    VisionProbe,
    /// 推理能力探测中
    ReasoningProbe,
    /// 上下文/输出上限探测中
    LimitProbe,
    /// 已完成
    Done,
    /// 已失败
    Failed,
}

impl ProbeStage {
    /// 阶段的中文显示名，用于进度条与日志。
    pub fn label(&self) -> &'static str {
        match self {
            ProbeStage::Pending => "排队等待",
            ProbeStage::Listing => "拉取模型列表",
            ProbeStage::ToolProbe => "探测工具调用",
            ProbeStage::VisionProbe => "探测附件上传",
            ProbeStage::ReasoningProbe => "探测推理能力",
            ProbeStage::LimitProbe => "探测上下文上限",
            ProbeStage::Done => "完成",
            ProbeStage::Failed => "失败",
        }
    }
}

/// API Key 的来源方式。
///
/// 与前端 `ApiKeySource` 对应。引入它的动机：让用户不必在界面上以明文
/// 粘贴 Key —— 可以只填一个环境变量名或系统凭据名，由后端在真正发请求前
/// 才把明文取出来。这样明文既不落盘，也不进入前端渲染层。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApiKeySource {
    /// 明文：直接使用 `api_key` 字段
    Plain,
    /// 环境变量：从 `api_key_env_var` 指定的变量读取
    Env,
    /// Windows 凭据管理器：从 `api_key_credential` 指定的通用凭据读取
    Credential,
}

impl Default for ApiKeySource {
    /// 默认明文 —— 与引入本功能之前的行为完全一致，保证向后兼容。
    ///
    /// 与 `ProbeStatus` 同样手写而不 derive：避免将来在枚举头部插入变体时
    /// 默认值被静默改变。
    fn default() -> Self {
        ApiKeySource::Plain
    }
}

// ---- 二、命令入参 / 出参结构体 ----

/// `start_probe` 的入参。
///
/// 安全提示：`api_key` 字段是唯一持有 Key 明文的载体。
/// 它绝不被序列化回前端（本结构体虽 derive 了 Serialize 以便日志调试，
/// 但全项目代码中没有任何一处把 ProbeConfig 直接 emit 出去），
/// 也绝不被打印。参见 `error::redact_key`。
///
/// 当 `api_key_source` 不是 `Plain` 时，`api_key` 字段应留空 ——
/// 明文由后端在 `resolve_api_key()` 里从环境变量 / 凭据管理器取出，
/// 前端自始至终不接触它。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeConfig {
    /// API 根地址，例如 `https://api.openai.com/v1`
    pub base_url: String,
    /// API Key（明文，仅用于构造 Authorization 头）
    pub api_key: String,
    /// 并发数，后端会 clamp 到 1..=10
    pub concurrency: usize,
    /// 单请求超时（秒），后端会 clamp 到 5..=300
    pub timeout_secs: u64,
    /// 是否执行「超大 max_tokens」探测
    pub probe_limits: bool,
    /// 可选模型白名单；空表示全量
    #[serde(default)]
    pub only_models: Vec<String>,
    /// API Key 的来源方式：明文 / 环境变量 / Windows 凭据管理器。
    ///
    /// `#[serde(default)]` 保证旧版前端（只发 apiKey）仍可正常调用。
    #[serde(default)]
    pub api_key_source: ApiKeySource,
    /// 当 `api_key_source = Env` 时要读取的环境变量名
    #[serde(default)]
    pub api_key_env_var: String,
    /// 当 `api_key_source = Credential` 时要读取的 Windows 凭据名（target）
    #[serde(default)]
    pub api_key_credential: String,
}

impl ProbeConfig {
    /// 规范化：裁剪空白、补全协议前缀、clamp 数值范围。
    ///
    /// 这是唯一允许修正用户输入的地方，后续所有代码都可以假定
    /// `base_url` 是合法且不含尾部斜杠的绝对 URL。
    pub fn normalized(&self) -> Self {
        let mut base = self.base_url.trim().to_string();

        // 只有当用户没写协议时才补 https://，
        // 避免把 "http://localhost:11434/v1" 误改成 "https://http://..."
        if !base.is_empty()
            && !base.starts_with("http://")
            && !base.starts_with("https://")
        {
            base = format!("https://{base}");
        }

        // 去掉全部尾部斜杠（可能有多条）
        while base.ends_with('/') {
            base.pop();
        }

        Self {
            base_url: base,
            api_key: self.api_key.trim().to_string(),
            concurrency: self.concurrency.clamp(1, 10),
            timeout_secs: self.timeout_secs.clamp(5, 300),
            probe_limits: self.probe_limits,
            only_models: self
                .only_models
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            api_key_source: self.api_key_source,
            api_key_env_var: self.api_key_env_var.trim().to_string(),
            api_key_credential: self.api_key_credential.trim().to_string(),
        }
    }

    /// 校验必填项。返回中文错误信息（面向用户）。
    ///
    /// 密钥按来源分别校验：明文模式要求 `api_key` 非空；
    /// 环境变量 / 凭据模式则只要求对应的「名字」非空 ——
    /// 真正的取值放在 `resolve_api_key()` 里做（可能失败，且失败原因
    /// 与「参数不合法」性质不同，需分开提示）。
    pub fn validate(&self) -> Result<(), String> {
        if self.base_url.trim().is_empty() {
            return Err("Base URL 不能为空。".to_string());
        }

        match self.api_key_source {
            ApiKeySource::Plain => {
                if self.api_key.trim().is_empty() {
                    return Err("API Key 不能为空。".to_string());
                }
            }
            ApiKeySource::Env => {
                if self.api_key_env_var.trim().is_empty() {
                    return Err("已选择从环境变量读取 API Key，但未填写环境变量名。".to_string());
                }
            }
            ApiKeySource::Credential => {
                if self.api_key_credential.trim().is_empty() {
                    return Err("已选择从 Windows 凭据管理器读取 API Key，但未填写凭据名。".to_string());
                }
            }
        }

        // 规范化之后的 URL 至少要有 "https://x" 这样 9 个字符
        if self.base_url.len() < 9 || !self.base_url.contains("://") {
            return Err(format!("Base URL 格式不合法：{}", self.base_url));
        }
        Ok(())
    }

    /// 依据 `api_key_source` 解析出最终用于请求的明文 Key。
    ///
    /// 这是唯一把「外部来源」转成明文的入口。返回值含敏感信息，
    /// 调用方只能把它交给 `ApiClient`，不得打印、不得回传前端。
    pub fn resolve_api_key(&self) -> Result<String, String> {
        match self.api_key_source {
            ApiKeySource::Plain => {
                let key = self.api_key.trim().to_string();
                if key.is_empty() {
                    return Err("API Key 不能为空。".to_string());
                }
                Ok(key)
            }
            ApiKeySource::Env => {
                let name = self.api_key_env_var.trim();
                if name.is_empty() {
                    return Err("未填写环境变量名。".to_string());
                }
                match std::env::var(name) {
                    Ok(value) if !value.trim().is_empty() => Ok(value.trim().to_string()),
                    Ok(_) => Err(format!("环境变量 {name} 已存在但内容为空，请检查后再试。")),
                    Err(_) => Err(format!(
                        "环境变量 {name} 未设置。请先在系统环境变量中配置它\
                        （可从凭据管理器取值后注入），或改回其它密钥来源。"
                    )),
                }
            }
            ApiKeySource::Credential => {
                let target = self.api_key_credential.trim();
                if target.is_empty() {
                    return Err("未填写凭据名。".to_string());
                }
                crate::credential::read_generic_credential(target)
            }
        }
    }

    /// 返回一份「Key 已解析为明文」的副本，供 `ApiClient` 等下游使用。
    ///
    /// 解析后把来源归为 `Plain` 并清空外部来源的名字字段，让下游只处理
    /// 一种形态，同时避免明文被二次解析。
    pub fn with_resolved_key(&self) -> Result<Self, String> {
        let key = self.resolve_api_key()?;
        let mut resolved = self.clone();
        resolved.api_key = key;
        resolved.api_key_source = ApiKeySource::Plain;
        resolved.api_key_env_var = String::new();
        resolved.api_key_credential = String::new();
        Ok(resolved)
    }
}

/// 单个模型探测完成后的全量信息。
///
/// 与前端 `ModelInfo` 接口字段一一对应（共 18 个字段）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    /// 模型 ID
    pub id: String,
    /// 显示名称（为空时回退为 id）
    pub display_name: String,
    /// 所属方
    pub owned_by: String,
    /// 创建时间（Unix 秒），0 表示未知
    pub created: i64,

    /// 是否支持工具调用
    pub supports_tools: TriState,
    /// 是否支持附件/图片上传
    pub supports_vision: TriState,
    /// 是否支持流式输出
    pub supports_streaming: TriState,

    /// 是否为推理模型
    pub is_reasoning: bool,
    /// 判定依据（中文，用于前端 tooltip）
    pub reasoning_evidence: String,

    /// 上下文窗口上限（token），0 表示未知
    pub context_limit: u64,
    /// 单次最大输出上限（token），0 表示未知
    pub max_output_limit: u64,
    /// 上下文上限来源
    pub context_limit_source: LimitSource,
    /// 输出上限来源
    pub max_output_limit_source: LimitSource,

    /// 能力数据的整体来源说明（中文），例如「接口声明（schema 2.4）」/「实测探测」/「混合」。
    ///
    /// 展示原因：部分网关的 `/models` 声明可能与官方文档不符
    /// （例如接口称某模型支持 tools、文档称不支持）。把来源显式标出来，
    /// 用户才能判断该采信谁，而不是盲信一个数字。
    #[serde(default)]
    pub caps_source: String,

    /// 该模型探测过程中的错误列表
    pub errors: Vec<ProbeError>,
    /// 探测耗时（毫秒）
    pub elapsed_ms: u64,
    /// 终态阶段
    pub final_stage: ProbeStage,
}

impl ModelInfo {
    /// 基于轻量列表条目创建一份「初始态」ModelInfo。
    /// 所有能力字段先置为 Unknown，待探测逐步回填。
    pub fn from_brief(brief: &ModelBrief) -> Self {
        Self {
            id: brief.id.clone(),
            display_name: brief.display_name.clone(),
            owned_by: brief.owned_by.clone(),
            created: brief.created,
            supports_tools: TriState::Unknown,
            supports_vision: TriState::Unknown,
            supports_streaming: TriState::Unknown,
            is_reasoning: false,
            reasoning_evidence: String::new(),
            context_limit: 0,
            max_output_limit: 0,
            context_limit_source: LimitSource::Unknown,
            max_output_limit_source: LimitSource::Unknown,
            caps_source: String::new(),
            errors: Vec::new(),
            elapsed_ms: 0,
            final_stage: ProbeStage::Pending,
        }
    }
}

/// 接口在 `/models` 响应中「自报」的能力（OpenRouter / 部分聚合网关的扩展 schema）。
///
/// 全部字段都是 `Option`，语义很关键：
/// - `Some(..)` = 接口已明确声明该能力 → 直接采信，无需再发请求探测
///   （省额度、更快、且比探测更准确）
/// - `None`     = 接口未声明 → 回退到真实请求探测（标准 OpenAI 接口走这条路）
///
/// 典型返回（某网关的扩展 schema）：
///
/// ```text
/// {
///   "id": "glm-5.3",
///   "input_modalities": [
///     { "type": "text",
///       "supported_inputs": { "max_context_length": { "value": 1048576, "unit": "token" } } },
///     { "type": "image",
///       "supported_inputs": { "sources": { "type": "enum", "values": ["url", "base64"] } } }
///   ],
///   "output_modalities": [
///     { "type": "text", "max_length": { "value": 1048576 },
///       "supported_parameters": { "tools": { "type": "boolean" } } }
///   ],
///   "is_ready": true
/// }
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclaredCaps {
    /// 上下文窗口上限（token）
    pub context_limit: Option<u64>,
    /// 单次输出上限（token）
    pub max_output_limit: Option<u64>,
    /// 是否支持工具调用
    pub supports_tools: Option<bool>,
    /// 是否支持图片输入（视觉/附件）
    pub supports_vision: Option<bool>,
    /// 是否为推理模型（`supported_parameters` 中存在 `reasoning`）
    ///
    /// 注意：这比「响应里出现 reasoning_content」更可靠：
    /// 部分网关对所有模型都会返回 `reasoning_content` 字段（可能为空），
    /// 因此靠响应体判断会把非推理模型误判为推理模型。
    pub supports_reasoning: Option<bool>,
    /// 接口是否标记为就绪（`is_ready`）
    pub is_ready: Option<bool>,
    /// 扩展 schema 版本，仅用于排障展示
    pub schema_version: String,
}

impl DeclaredCaps {
    /// 是否「一条声明都没有」——为 true 时需要走完整的真实请求探测。
    pub fn is_empty(&self) -> bool {
        self.context_limit.is_none()
            && self.max_output_limit.is_none()
            && self.supports_tools.is_none()
            && self.supports_vision.is_none()
            && self.supports_reasoning.is_none()
    }
}

/// 轻量模型条目（仅 /models 列表信息）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelBrief {
    /// 模型 ID
    pub id: String,
    /// 显示名称
    pub display_name: String,
    /// 所属方
    pub owned_by: String,
    /// 创建时间（Unix 秒）
    pub created: i64,
    /// 接口自报的能力；为空表示未声明，需要真实探测
    #[serde(default)]
    pub declared: DeclaredCaps,
}

/// `list_models` 的返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelListResult {
    /// 发现的模型条目
    pub models: Vec<ModelBrief>,
    /// 错误列表（空表示成功）
    pub errors: Vec<ProbeError>,
    /// 耗时（毫秒）
    pub elapsed_ms: u64,
}

/// `ping_connection` 的返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PingResult {
    /// 是否连通且鉴权通过
    pub ok: bool,
    /// HTTP 状态码，0 表示未拿到响应
    pub http_status: u16,
    /// 错误；ok 为 true 时该字段为 None，序列化后是 null
    pub error: Option<ProbeError>,
    /// 往返耗时（毫秒）
    pub elapsed_ms: u64,
    /// 发现的模型数量
    pub model_count: usize,
    /// 实际请求的 URL（不含 Key，可安全展示）
    pub resolved_url: String,
}

/// `start_probe` 的返回：整轮运行的汇总。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeRunSummary {
    /// 运行 ID（Unix 毫秒时间戳字符串）
    pub run_id: String,
    /// 终态
    pub status: ProbeStatus,
    /// 成功完成的模型数
    pub total_models: usize,
    /// 成功获取到全量信息的模型列表
    pub models: Vec<ModelInfo>,
    /// 全局错误
    pub errors: Vec<ProbeError>,
    /// 总耗时（毫秒）
    pub elapsed_ms: u64,
}

// ---- 三、事件 payload 结构体 ----

/// 事件 `probe://progress` 的 payload。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeProgress {
    /// 所属运行 ID
    pub run_id: String,
    /// 当前处理的模型 ID
    pub model_id: String,
    /// 当前阶段
    pub stage: ProbeStage,
    /// 阶段中文名
    pub stage_label: String,
    /// 已完成模型数
    pub completed: usize,
    /// 总模型数
    pub total: usize,
    /// 整体完成百分比 0-100
    pub percent: u8,
    /// 本阶段耗时（毫秒）
    pub elapsed_ms: u64,
}

/// 事件 `probe://log` 的 payload。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeLog {
    /// 所属运行 ID
    pub run_id: String,
    /// 日志级别
    pub level: LogLevel,
    /// 关联模型 ID；全局日志为空串
    pub model_id: String,
    /// 日志正文（已脱敏）
    pub message: String,
    /// Unix 毫秒时间戳
    pub timestamp: u64,
}

/// 日志级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// 事件 `probe://model-done` 的 payload。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeModelDone {
    /// 所属运行 ID
    pub run_id: String,
    /// 完成的模型全量信息
    pub model: ModelInfo,
    /// 已完成数
    pub completed: usize,
    /// 总数
    pub total: usize,
}

/// 事件 `probe://finished` 的 payload。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeFinished {
    /// 所属运行 ID
    pub run_id: String,
    /// 终态
    pub status: ProbeStatus,
    /// 完成数
    pub completed: usize,
    /// 总数
    pub total: usize,
    /// 总耗时（毫秒）
    pub elapsed_ms: u64,
    /// 全局错误
    pub errors: Vec<ProbeError>,
}

/// 事件 `probe://error` 的 payload。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeFatal {
    /// 所属运行 ID
    pub run_id: String,
    /// 分类后的错误
    pub error: ProbeError,
    /// 致命错误发生阶段
    pub stage: ProbeStage,
}

// ---- 四、快照结构体（供 get_probe_state 使用） ----

/// `get_probe_state` 的返回：任意时刻的实时状态。
///
/// 前端打开进度面板、或事件丢失需要重同步时使用。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeSnapshot {
    /// 是否有任务在运行
    pub is_running: bool,
    /// 当前运行 ID；无任务时为空串
    pub run_id: String,
    /// 当前状态
    pub status: ProbeStatus,
    /// 已发现的模型总数
    pub total_models: usize,
    /// 已完成探测的模型数
    pub completed_models: usize,
    /// 截至此刻已完成探测的模型全量信息
    pub models: Vec<ModelInfo>,
}

// ---- 五、配置导出 ----

/// `save_config_file` 的入参。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveConfigRequest {
    /// 建议的文件名（不含目录），例如 `opencode.json`
    pub file_name: String,
    /// 文件正文
    pub content: String,
}

/// `save_config_file` 的返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveConfigResult {
    /// 是否保存成功
    pub saved: bool,
    /// 实际写入的绝对路径；取消保存时为空串
    pub path: String,
}

// ---- 六、一键填写（读取/写入目标工具的配置文件） ----

/// `inspect_tool_config` 的返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfigFile {
    /// 解析后的目标文件绝对路径（已做 `~` 展开与默认路径推断）
    pub path: String,
    /// 文件是否存在
    pub exists: bool,
    /// 文件内容（UTF-8）；不存在时为空字符串
    pub content: String,
}

/// `write_tool_config` 的返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfigWriteResult {
    /// 实际写入的绝对路径
    pub path: String,
    /// 写入前创建的备份文件路径；原文件不存在（未备份）时为 None
    pub backup_path: Option<String>,
    /// 写入的字节数
    pub bytes_written: u64,
}

/// `scan_tool_configs` 的返回：一次扫出的全部配置文件。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfigScan {
    /// OpenCode 的配置文件（`opencode.json` / `opencode.jsonc`，存在几个给几个）
    pub opencode: Vec<ToolConfigFile>,
    /// dsh 各 profile 的 `cordis.patch.yml`（`web` 优先，按名称排序）
    pub dsh: Vec<ToolConfigFile>,
}

// ---- 七、环境变量密钥管理 ----

/// `env_secret_list` 的条目：一个疑似密钥的环境变量。
///
/// 出于脱敏考虑，后端**永不**返回完整值，只有掩码预览。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvSecretInfo {
    /// 变量名
    pub name: String,
    /// 掩码预览（如 `sk-1…abcd`；短值整体打码）
    pub preview: String,
    /// 值长度（字符数）
    pub length: usize,
    /// 是否已持久化到用户级环境变量（Windows：HKCU\Environment）
    pub persisted: bool,
}

/// `env_secret_set` 的返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvSecretWriteResult {
    /// 后端进程内已生效（本应用的探测/连通测试立即可用）
    pub process_set: bool,
    /// 已写入用户级持久环境（新开的终端/程序可读取；Windows 走 setx）
    pub persisted: bool,
    /// 面向用户的补充说明（空串 = 无）
    pub note: String,
}

/// `env_secret_status` 的返回：单个变量的精确存在性。
///
/// 用于联动校验：配置引用的变量名未必命中「疑似密钥」命名启发式
/// （如 `MY_KEY`），清单里看不到不代表机器上没有 —— 以此命令为准。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvSecretStatus {
    /// 本进程可见且值非空
    pub exists: bool,
    /// 是否已持久化到用户级环境变量
    pub persisted: bool,
    /// 掩码预览（不存在时为空串）
    pub preview: String,
    /// 值长度（不存在时为 0）
    pub length: u64,
}

// ---- 单元测试 ----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_normalization_trims_and_prefixes() {
        let cfg = ProbeConfig {
            base_url: "  api.openai.com/v1///  ".to_string(),
            api_key: "  sk-abc  ".to_string(),
            concurrency: 99,
            timeout_secs: 1,
            probe_limits: true,
            only_models: vec!["  ".to_string(), "gpt-4o".to_string()],
            api_key_source: ApiKeySource::Plain,
            api_key_env_var: String::new(),
            api_key_credential: String::new(),
        };
        let n = cfg.normalized();
        assert_eq!(n.base_url, "https://api.openai.com/v1");
        assert_eq!(n.api_key, "sk-abc");
        assert_eq!(n.concurrency, 10, "并发数应被 clamp 到上限 10");
        assert_eq!(n.timeout_secs, 5, "超时应被 clamp 到下限 5");
        assert_eq!(n.only_models, vec!["gpt-4o".to_string()]);
    }

    #[test]
    fn config_normalization_keeps_explicit_http_scheme() {
        // 本地 Ollama 场景：不应被强行改成 https
        let cfg = ProbeConfig {
            base_url: "http://localhost:11434/v1".to_string(),
            api_key: "ollama".to_string(),
            concurrency: 3,
            timeout_secs: 30,
            probe_limits: false,
            only_models: vec![],
            api_key_source: ApiKeySource::Plain,
            api_key_env_var: String::new(),
            api_key_credential: String::new(),
        };
        assert_eq!(cfg.normalized().base_url, "http://localhost:11434/v1");
    }

    #[test]
    fn api_key_source_defaults_to_plain_for_backward_compat() {
        // 旧版前端只发 baseUrl/apiKey/...，不带 apiKeySource 字段，
        // 反序列化后应回退为 Plain，且行为与改动前一致。
        let json = r#"{
            "baseUrl": "https://api.example.com/v1",
            "apiKey": "sk-legacy",
            "concurrency": 3,
            "timeoutSecs": 30,
            "probeLimits": true
        }"#;
        let cfg: ProbeConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.api_key_source, ApiKeySource::Plain);
        assert_eq!(cfg.resolve_api_key().unwrap(), "sk-legacy");
    }

    #[test]
    fn validate_requires_the_name_matching_the_selected_source() {
        // 环境变量模式：api_key 留空也合法，但变量名必填
        let env_cfg = ProbeConfig {
            base_url: "https://api.example.com/v1".to_string(),
            api_key: String::new(),
            concurrency: 3,
            timeout_secs: 30,
            probe_limits: true,
            only_models: vec![],
            api_key_source: ApiKeySource::Env,
            api_key_env_var: "MY_API_KEY".to_string(),
            api_key_credential: String::new(),
        };
        assert!(env_cfg.validate().is_ok());
        assert!(env_cfg.with_resolved_key().is_err(), "变量未设置时应解析失败");

        let env_no_name = ProbeConfig {
            api_key_env_var: String::new(),
            ..env_cfg.clone()
        };
        assert!(env_no_name.validate().is_err());

        // 凭据模式：凭据名必填
        let cred_cfg = ProbeConfig {
            api_key_source: ApiKeySource::Credential,
            api_key_env_var: String::new(),
            api_key_credential: "modelscout/openai".to_string(),
            ..env_cfg.clone()
        };
        assert!(cred_cfg.validate().is_ok());

        let cred_no_name = ProbeConfig {
            api_key_credential: String::new(),
            ..cred_cfg
        };
        assert!(cred_no_name.validate().is_err());

        // 明文模式：Key 必填
        let plain_no_key = ProbeConfig {
            api_key_source: ApiKeySource::Plain,
            api_key: String::new(),
            ..env_cfg
        };
        assert!(plain_no_key.validate().is_err());
    }

    #[test]
    fn resolve_api_key_reads_environment_variable() {
        // 用进程级唯一的名字，避免与其它测试相互干扰
        let var = "MODELSCOUT_TEST_ENV_KEY_9F3A";
        std::env::set_var(var, "  sk-from-env  ");

        let cfg = ProbeConfig {
            base_url: "https://api.example.com/v1".to_string(),
            api_key: String::new(),
            concurrency: 3,
            timeout_secs: 30,
            probe_limits: true,
            only_models: vec![],
            api_key_source: ApiKeySource::Env,
            api_key_env_var: var.to_string(),
            api_key_credential: String::new(),
        };

        let resolved = cfg.with_resolved_key().unwrap();
        assert_eq!(resolved.api_key, "sk-from-env", "应去掉首尾空白");
        // 解析后来源归一为明文，名字字段清空，避免二次解析
        assert_eq!(resolved.api_key_source, ApiKeySource::Plain);
        assert!(resolved.api_key_env_var.is_empty());

        std::env::remove_var(var);
    }

    #[test]
    fn camel_case_serialization_is_enforced() {
        // 关键回归测试：确认结构体序列化后是驼峰而非蛇形
        let brief = ModelBrief {
            id: "gpt-4o".to_string(),
            display_name: "GPT-4o".to_string(),
            owned_by: "openai".to_string(),
            created: 1715367049,
            declared: DeclaredCaps::default(),
        };
        let json = serde_json::to_string(&brief).unwrap();
        assert!(json.contains("\"displayName\""), "实际输出: {json}");
        assert!(json.contains("\"ownedBy\""), "实际输出: {json}");
        assert!(!json.contains("display_name"));

        let info = ModelInfo::from_brief(&brief);
        let json2 = serde_json::to_string(&info).unwrap();
        assert!(json2.contains("\"supportsTools\""));
        assert!(json2.contains("\"isReasoning\""));
        assert!(json2.contains("\"contextLimit\""));
        assert!(json2.contains("\"maxOutputLimit\""));
        assert!(json2.contains("\"finalStage\""));
        assert!(json2.contains("\"elapsedMs\""));
    }

    #[test]
    fn enum_serialization_is_camel_case() {
        assert_eq!(serde_json::to_string(&TriState::Supported).unwrap(), "\"supported\"");
        assert_eq!(serde_json::to_string(&TriState::Unknown).unwrap(), "\"unknown\"");
        assert_eq!(
            serde_json::to_string(&ProbeStage::ToolProbe).unwrap(),
            "\"toolProbe\""
        );
        assert_eq!(
            serde_json::to_string(&LimitSource::ErrorParse).unwrap(),
            "\"errorParse\""
        );
        assert_eq!(
            serde_json::to_string(&ProbeStatus::Cancelled).unwrap(),
            "\"cancelled\""
        );
    }
}
