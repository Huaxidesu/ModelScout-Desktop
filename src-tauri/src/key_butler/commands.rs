/**
 * ============================================================================
 * ModelScout —— Key 管家：命令层
 * ============================================================================
 *
 * 【职责】
 * 把 Key 管家的能力暴露为 13 个 `key_butler_*` 命令。
 * 命令名必须与前端 `COMMAND_NAMES.KEY_BUTLER_*` 的字符串值逐字一致
 * （`scripts/cross_check.py` 会把 `generate_handler!` 里的函数名与前端常量比对）。
 *
 * 【安全约定】
 *   - 任何日志 / 事件 / 错误信息里都不得出现 Key 明文；
 *     上游响应正文在进入错误信息前先过 `scrub_secrets`。
 *   - `key_butler_reveal` 的返回值是唯一一次明文外流，仅供前端一键复制，
 *     后端不记录任何日志。
 * ============================================================================
 */

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::stream::{self, StreamExt};
use regex::Regex;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

use crate::error::{scrub_secrets, CommandError, ProbeErrorKind};
use crate::events;
use crate::key_butler::balance;
use crate::key_butler::classify;
use crate::key_butler::models::{
    BalanceEndpointKind, CheckStage, KeyCheckEntryDone, KeyCheckFinished, KeyCheckProgress,
    KeyCheckRequest, KeyCheckSummary, KeyConfigPatch, KeyEntry, KeyImportResult, KeyStatus, Snapshot,
    VaultStatus,
};
use crate::key_butler::platform;
use crate::key_butler::vault::{mask_key, CheckRun, KeyButlerState, StoredKey};

/* ---------------------------------------------------------------------------
 * 时间与 id
 * ------------------------------------------------------------------------- */

/// 当前 Unix 毫秒时间戳。
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 生成新的运行 id（Unix 毫秒字符串，与探测模块同一形态）。
fn new_run_id() -> String {
    now_ms().to_string()
}

/// `KeyStatus` → 序列化字符串（用于快照与汇总）。
fn key_status_str(status: KeyStatus) -> &'static str {
    match status {
        KeyStatus::Active => "active",
        KeyStatus::Insufficient => "insufficient",
        KeyStatus::Invalid => "invalid",
        KeyStatus::Error => "error",
    }
}

/// `ProbeErrorKind` → 序列化字符串（`KeyEntry.error_category` 复用探测模块的 9 类）。
fn error_kind_str(kind: ProbeErrorKind) -> String {
    match kind {
        ProbeErrorKind::KeyInvalid => "keyInvalid",
        ProbeErrorKind::BaseUrlInvalid => "baseUrlInvalid",
        ProbeErrorKind::NetworkTimeout => "networkTimeout",
        ProbeErrorKind::RateLimited => "rateLimited",
        ProbeErrorKind::ServerError => "serverError",
        ProbeErrorKind::ModelNotFound => "modelNotFound",
        ProbeErrorKind::ParseError => "parseError",
        ProbeErrorKind::Cancelled => "cancelled",
        ProbeErrorKind::Unknown => "unknown",
    }
    .to_string()
}

/* ---------------------------------------------------------------------------
 * Key 文本提取
 * ------------------------------------------------------------------------- */

/// 提取规则（与前端 `src/utils/parseConnection.ts` 的正则对齐）。
///
/// 第二个元素表示「是否取第 1 个捕获组」。
type ExtractPattern = (Regex, bool);

/// 编译一次、全进程复用。
fn extract_patterns() -> &'static Vec<ExtractPattern> {
    static PATTERNS: OnceLock<Vec<ExtractPattern>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        // 正则字面量是编译期常量，构造失败只可能是写错了 —— 直接 panic 暴露问题
        let build = |pattern: &str, group1: bool| {
            (
                Regex::new(pattern).expect("Key 提取正则必须是合法表达式"),
                group1,
            )
        };

        let mut patterns: Vec<ExtractPattern> = Vec::new();

        // 规则 0：**已知平台前缀**（从前缀表派生，长前缀在前）。
        //
        // 为什么必须有这条：`gsk_` / `AIza` / `hf_` / `r8_` / `xai-` 这些前缀
        // 都不符合 `sk-` 的形态，靠下面几条固定正则根本提取不到。
        // 用前缀表派生之后，用户往 platform_prefixes.json 里加一条规则，
        // 批量粘贴就自动认识这种 Key。
        let prefixes = platform::prefixes();
        if !prefixes.is_empty() {
            let alternation = prefixes
                .iter()
                .map(|prefix| regex::escape(prefix))
                .collect::<Vec<_>>()
                .join("|");
            // 量词部分单独成常量，而不是在 format! 里写转义大括号 ——
            // 后者会让 scripts/cross_check.py 的「未替换占位符」扫描误报。
            const KEY_BODY: &str = "[A-Za-z0-9_\\-]{12,}";
            let pattern = format!("(?:{alternation}){KEY_BODY}");
            patterns.push(build(&pattern, false));
        }

        // 规则 1：带标签：apiKey: xxx / "api_key": "xxx" / 密钥：xxx
        patterns.push(build(
            r#"(?i)(?:api[_\-\s]?key|apikey|access[_\-\s]?token|auth[_\-\s]?token|secret[_\-\s]?key|token|密钥|秘钥|令牌|key)\s*[:=：]\s*["'`]?([A-Za-z0-9_\-.=]{12,})"#,
            true,
        ));

        // 规则 2：Bearer xxx
        patterns.push(build(r"(?i)\bBearer\s+([A-Za-z0-9_\-.=]{12,})", true));

        // 规则 3：纯十六进制长串（有些网关的 key 没有前缀）
        patterns.push(build(r"\b[a-fA-F0-9]{32,}\b", false));

        patterns
    })
}

/// 常见顶级域名后缀 —— 用于排除「其实是域名」的误判。
const TLDS: [&str; 18] = [
    ".com", ".cn", ".org", ".net", ".io", ".ai", ".dev", ".app", ".xyz", ".top", ".info", ".co",
    ".me", ".cc", ".tv", ".site", ".online", ".tech",
];

/// 判断候选是否更像域名而不是密钥（对齐前端 `looksLikeHostname` 的思路）。
fn looks_like_hostname(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if TLDS.iter().any(|tld| lower.ends_with(tld)) {
        return true;
    }
    value.contains('.')
        && value.contains('-')
        && !value.chars().any(|c| c.is_ascii_uppercase())
        && !value.contains('_')
}

/// 从一段文本里提取全部 Key 候选（去重、保序）。
fn extract_keys(text: &str) -> Vec<String> {
    let mut hits: Vec<(usize, usize, String)> = Vec::new();

    for (regex, group1) in extract_patterns() {
        for captures in regex.captures_iter(text) {
            let matched = if *group1 {
                captures.get(1)
            } else {
                captures.get(0)
            };
            let Some(matched) = matched else { continue };
            let value = matched.as_str().trim().to_string();
            if value.is_empty() {
                continue;
            }
            hits.push((matched.start(), matched.end(), value));
        }
    }

    // 按出现位置排序，保证「先出现的先导入」
    hits.sort_by_key(|(start, _, _)| *start);

    // 去掉「被更长候选完整包住」的冗余项。
    //
    // 典型例子：`sk-2ee1cb8a…82bf`（sk- 后面跟 64 位十六进制）会同时命中
    // 「已知前缀」规则与「纯十六进制」规则，后者其实是前者的**后缀** ——
    // 不过滤的话，一条 Key 会被导入成两条。
    //
    // 判据必须用**严格更长**（`>`）而不是「不相等」：同一个值常常被两条规则
    // 同时命中（如 `sk-ant-…` 同时符合前缀规则与 `sk-` 规则），
    // 用不等号会让两个候选互相认为「对方包含我」而双双被删。
    // 等长的重复项交给下面的 seen 去重。
    let values: Vec<String> = hits.iter().map(|(_, _, value)| value.clone()).collect();
    hits.retain(|(_, _, value)| {
        !values
            .iter()
            .any(|other| other.len() > value.len() && other.contains(value.as_str()))
    });

    let mut seen: HashSet<String> = HashSet::new();
    let mut result: Vec<String> = Vec::new();
    for (_, _, value) in hits {
        if value.contains("://") || value.starts_with("http") {
            continue;
        }
        if looks_like_hostname(&value) {
            continue;
        }
        if seen.insert(value.clone()) {
            result.push(value);
        }
    }
    result
}

/// 把余额适配器字符串解析成枚举。
fn parse_endpoint_kind(raw: &str) -> BalanceEndpointKind {
    match raw.trim().to_ascii_lowercase().as_str() {
        "builtin-oneapi" => BalanceEndpointKind::BuiltinOneapi,
        "custom" => BalanceEndpointKind::Custom,
        _ => BalanceEndpointKind::None,
    }
}

/// 由明文 Key 构造一条待导入的条目。
fn build_entry(key: &str) -> KeyEntry {
    let rule = platform::identify(key);
    KeyEntry {
        id: uuid::Uuid::new_v4().to_string(),
        masked_key: mask_key(key),
        platform: rule.platform,
        // note 留给用户写备注；规则里的说明不进这里，避免导入后满是机器文案
        note: String::new(),
        created_at: now_ms(),
        last_checked_at: None,
        status: KeyStatus::Error,
        key_valid: None,
        balance: None,
        balance_known: false,
        currency: None,
        error_category: None,
        balance_endpoint: parse_endpoint_kind(&rule.balance_endpoint),
        custom_endpoint: None,
        base_url: rule.default_base_url,
    }
}

/* ---------------------------------------------------------------------------
 * 事件推送
 * ------------------------------------------------------------------------- */

/// 推送一条进度事件。
fn emit_progress(app: &AppHandle, run: &CheckRun, entry_id: &str, masked: &str, stage: CheckStage) {
    let total = run.total();
    let completed = run.completed();
    let percent = if total == 0 {
        0
    } else {
        ((completed.saturating_mul(100)) / total) as u32
    };

    let payload = KeyCheckProgress {
        run_id: run.run_id.clone(),
        key_id: entry_id.to_string(),
        masked_key: masked.to_string(),
        stage,
        stage_label: stage.label().to_string(),
        completed,
        total,
        percent,
    };
    let _ = app.emit(events::EVENT_KEY_BUTLER_PROGRESS, payload);
}

/* ---------------------------------------------------------------------------
 * 有效性检测
 * ------------------------------------------------------------------------- */

/// 有效性检测的中间结果。
struct ValidityResult {
    /// Key 是否有效；`None` 表示未能判定
    key_valid: Option<bool>,
    /// 错误分类（复用 9 类字符串值）
    error_category: Option<String>,
    /// 「配额耗尽」短路：直接给定的余额，非 `None` 时不再请求余额接口
    forced_balance: Option<f64>,
}

impl ValidityResult {
    fn network_error(kind: &str) -> Self {
        Self {
            key_valid: None,
            error_category: Some(kind.to_string()),
            forced_balance: None,
        }
    }
}

/// 用**最小 chat 请求**判定 Key 是否有效。
///
/// 【为什么不用 `/models` 判定】
/// 部分中转站的 `/models` 不校验 Key，会把失效的 Key 判成有效（任务书第四节）。
/// 因此先用 `/models` 拿一个模型名（顺带能提前发现 401/403/404），
/// 真正的判定交给 `max_tokens=1` 的 chat 请求。
async fn check_validity(client: &reqwest::Client, base_url: &str, api_key: &str) -> ValidityResult {
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return ValidityResult {
            key_valid: None,
            error_category: Some("unknown".to_string()),
            forced_balance: None,
        };
    }

    // ---- 第 1 步：取一个可用的模型名 ----
    let mut model = "gpt-3.5-turbo".to_string();
    let models_url = format!("{base}/models");
    match client
        .get(&models_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status().as_u16();
            if status == 401 || status == 403 {
                return ValidityResult {
                    key_valid: Some(false),
                    error_category: Some("keyInvalid".to_string()),
                    forced_balance: None,
                };
            }
            if status == 404 {
                return ValidityResult {
                    key_valid: None,
                    error_category: Some("baseUrlInvalid".to_string()),
                    forced_balance: None,
                };
            }
            if response.status().is_success() {
                let text = response.text().await.unwrap_or_default();
                let text = scrub_secrets(&text, api_key);
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(id) = value
                        .get("data")
                        .and_then(|data| data.as_array())
                        .and_then(|list| list.first())
                        .and_then(|first| first.get("id"))
                        .and_then(|id| id.as_str())
                    {
                        if !id.trim().is_empty() {
                            model = id.trim().to_string();
                        }
                    }
                }
            }
            // 其它状态码先忽略，交给 chat 请求定夺
        }
        Err(err) => {
            let kind = if err.is_connect() {
                "baseUrlInvalid"
            } else {
                "networkTimeout"
            };
            return ValidityResult::network_error(kind);
        }
    }

    // ---- 第 2 步：最小 chat 请求 ----
    let chat_url = format!("{base}/chat/completions");
    let body = json!({
        "model": model,
        "messages": [{ "role": "user", "content": "hi" }],
        "max_tokens": 1
    });

    match client
        .post(&chat_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(response) => match response.status().as_u16() {
            200..=299 => ValidityResult {
                key_valid: Some(true),
                error_category: None,
                forced_balance: None,
            },
            401 | 403 => ValidityResult {
                key_valid: Some(false),
                error_category: Some("keyInvalid".to_string()),
                forced_balance: None,
            },
            // 402：服务端明确告知「没额度了」。Key 本身有效，余额直接记 0，
            // 不再去请求余额接口（那边多半也会失败，徒增噪声）。
            402 => ValidityResult {
                key_valid: Some(true),
                error_category: None,
                forced_balance: Some(0.0),
            },
            404 => ValidityResult {
                key_valid: None,
                error_category: Some("baseUrlInvalid".to_string()),
                forced_balance: None,
            },
            429 => ValidityResult {
                key_valid: None,
                error_category: Some("rateLimited".to_string()),
                forced_balance: None,
            },
            // 400 / 422：服务端已经完成了鉴权，只是不接受这次请求参数
            // （常见于「模型名不对」）。此时把 Key 判为有效，避免误杀。
            400 | 422 => ValidityResult {
                key_valid: Some(true),
                error_category: Some("unknown".to_string()),
                forced_balance: None,
            },
            status if status >= 500 => ValidityResult {
                key_valid: None,
                error_category: Some("serverError".to_string()),
                forced_balance: None,
            },
            _ => ValidityResult {
                key_valid: None,
                error_category: Some("unknown".to_string()),
                forced_balance: None,
            },
        },
        Err(err) => {
            let kind = if err.is_connect() {
                "baseUrlInvalid"
            } else {
                "networkTimeout"
            };
            ValidityResult::network_error(kind)
        }
    }
}

/* ---------------------------------------------------------------------------
 * 单条检测
 * ------------------------------------------------------------------------- */

/// 检测一条 Key：有效性 → 余额 → 分类 → 写回 → 快照 → 推送事件。
async fn check_one(
    app: AppHandle,
    client: reqwest::Client,
    run: Arc<CheckRun>,
    stored: StoredKey,
    key: Zeroizing<String>,
    threshold: f64,
) -> KeyEntry {
    let entry_id = stored.entry.id.clone();
    let masked = stored.entry.masked_key.clone();
    let base = stored.entry.base_url.clone().unwrap_or_default();
    let kind = stored.entry.balance_endpoint;
    let custom = stored.entry.custom_endpoint.clone();

    // 阶段 1：有效性
    emit_progress(&app, &run, &entry_id, &masked, CheckStage::Checking);
    let validity = check_validity(&client, &base, &key).await;

    // 该平台是否本应有余额接口：`none` 表示「本来就没有」，不算失败
    let balance_applicable = matches!(
        kind,
        BalanceEndpointKind::BuiltinOneapi | BalanceEndpointKind::Custom
    );

    // 阶段 2：余额
    let (balance_value, balance_known, currency, balance_error) =
        if let Some(forced) = validity.forced_balance {
            (Some(forced), true, None, None)
        } else if validity.key_valid == Some(true) && balance_applicable {
            emit_progress(&app, &run, &entry_id, &masked, CheckStage::Balance);
            let outcome =
                balance::query(&client, &base, &key, kind, custom.as_ref()).await;
            let known = outcome.is_known();
            let category = outcome
                .error
                .as_ref()
                .map(|(error_kind, _)| error_kind_str(*error_kind));
            (outcome.balance, known, outcome.currency, category)
        } else {
            (None, false, None, None)
        };

    let status = classify::classify(
        validity.key_valid,
        balance_applicable,
        balance_known,
        balance_value,
        threshold,
    );

    // 错误分类：有效性阶段的优先（更能解释「为什么没查成余额」）
    let error_category = validity.error_category.clone().or(balance_error);

    let checked_at = now_ms();
    let mut updated = stored.entry.clone();
    updated.last_checked_at = Some(checked_at);
    updated.status = status;
    updated.key_valid = validity.key_valid;
    updated.balance = balance_value;
    updated.balance_known = balance_known;
    updated.currency = currency;
    updated.error_category = error_category;

    // 写回内存表（vault 启用且已解锁时由命令末尾统一落盘）
    let state = app.state::<KeyButlerState>();
    let replacement = updated.clone();
    state
        .update(&entry_id, move |entry| {
            *entry = replacement;
        })
        .await;

    // 快照：失败也记一条，曲线才不会断。
    // 未启用加密保存时只进内存缓冲，不写盘（见 vault::append_snapshot）。
    let _ = state
        .append_snapshot(Snapshot {
            key_id: entry_id.clone(),
            ts: checked_at,
            balance: balance_value,
            status: key_status_str(status).to_string(),
        })
        .await;

    run.completed.fetch_add(1, Ordering::SeqCst);
    run.bump(status);

    let _ = app.emit(
        events::EVENT_KEY_BUTLER_ENTRY_DONE,
        KeyCheckEntryDone {
            run_id: run.run_id.clone(),
            entry: updated.clone(),
            completed: run.completed(),
            total: run.total(),
        },
    );

    updated
}

/* ---------------------------------------------------------------------------
 * 命令 1：导入
 * ------------------------------------------------------------------------- */

/// 批量文本 → 提取 Key → 前缀识别 → 入列（去重）。
#[tauri::command]
pub async fn key_butler_import(
    state: State<'_, KeyButlerState>,
    text: String,
) -> Result<KeyImportResult, CommandError> {
    state
        .ensure_writable()
        .await
        .map_err(CommandError::invalid_input)?;

    let candidates = extract_keys(&text);
    let existing: HashSet<String> = state.existing_keys().await.into_iter().collect();

    let mut seen: HashSet<String> = HashSet::new();
    let mut duplicates = 0usize;
    let mut items: Vec<StoredKey> = Vec::new();

    for key in candidates.iter() {
        if existing.contains(key) || !seen.insert(key.clone()) {
            duplicates += 1;
            continue;
        }
        items.push(StoredKey::new(key.clone(), build_entry(key)));
    }

    // 没能提取出 Key 的非空行数（给用户一个「这段文字里有多少是废话」的量）
    let ignored = text
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .filter(|line| !candidates.iter().any(|key| line.contains(key.as_str())))
        .count();

    let added = state.insert(items).await;
    state
        .persist_if_enabled()
        .await
        .map_err(CommandError::internal)?;

    Ok(KeyImportResult {
        entries: state.entries().await,
        added,
        duplicates,
        ignored,
    })
}

/* ---------------------------------------------------------------------------
 * 命令 2 / 3：检测与中止
 * ------------------------------------------------------------------------- */

/// 单/批量检测。进度与逐条结果走事件，返回值是整批汇总。
#[tauri::command]
pub async fn key_butler_check(
    app: AppHandle,
    state: State<'_, KeyButlerState>,
    request: KeyCheckRequest,
) -> Result<KeyCheckSummary, CommandError> {
    state
        .ensure_writable()
        .await
        .map_err(CommandError::invalid_input)?;

    let concurrency = request.concurrency.unwrap_or(3).clamp(1, 10);
    let timeout_secs = request.timeout_secs.unwrap_or(30).clamp(5, 300);
    let threshold = request.low_balance_threshold.unwrap_or(0.0);

    let targets = state.targets(&request.ids).await;
    let total = targets.len();
    if total == 0 {
        return Err(CommandError::invalid_input(
            "没有可检测的 Key —— 请先用「批量粘贴」导入。",
        ));
    }

    let run = match state.begin_run(new_run_id(), total).await {
        Some(run) => run,
        None => {
            return Err(CommandError::invalid_input(
                "已有检测任务在运行，请等待完成或点击「停止」。",
            ))
        }
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .connect_timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(16)
        .user_agent(concat!("ModelScout/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|err| CommandError::internal(format!("HTTP 客户端初始化失败：{err}")))?;

    let semaphore = Arc::new(Semaphore::new(concurrency));

    // 并发跑完；结果已经通过事件与内存表落地，这里不需要再收集返回值
    let _ = stream::iter(targets.into_iter())
        .map(|(stored, key)| {
            let app = app.clone();
            let client = client.clone();
            let run = run.clone();
            let semaphore = semaphore.clone();
            async move {
                // 中止是协作式的：每条开始前检查标志，已发出的请求等它自然结束
                if run.is_cancelled() {
                    return None;
                }
                let _permit = semaphore.acquire().await.ok()?;
                if run.is_cancelled() {
                    return None;
                }
                Some(check_one(app, client, run, stored, key, threshold).await)
            }
        })
        .buffer_unordered(concurrency)
        .collect::<Vec<Option<KeyEntry>>>()
        .await;

    let summary = run.summary();
    let finished_status = if run.is_cancelled() { "cancelled" } else { "done" };
    state.end_run().await;
    state
        .persist_if_enabled()
        .await
        .map_err(CommandError::internal)?;

    let _ = app.emit(
        events::EVENT_KEY_BUTLER_FINISHED,
        KeyCheckFinished {
            run_id: summary.run_id.clone(),
            status: finished_status.to_string(),
            checked: summary.checked,
            total: run.total(),
            elapsed_ms: summary.elapsed_ms,
            summary: summary.clone(),
        },
    );

    Ok(summary)
}

/// 请求中止当前批量检测。
#[tauri::command]
pub async fn key_butler_check_cancel(
    state: State<'_, KeyButlerState>,
) -> Result<KeyCheckSummary, CommandError> {
    Ok(state.cancel_run().await)
}

/* ---------------------------------------------------------------------------
 * 命令 4 ~ 8：列表 / 取明文 / 删除 / 备注 / 配置
 * ------------------------------------------------------------------------- */

/// 返回脱敏列表。
#[tauri::command]
pub async fn key_butler_list(
    state: State<'_, KeyButlerState>,
) -> Result<Vec<KeyEntry>, CommandError> {
    Ok(state.entries().await)
}

/// 返回单个完整 Key（仅供一键复制）。
#[tauri::command]
pub async fn key_butler_reveal(
    state: State<'_, KeyButlerState>,
    id: String,
) -> Result<String, CommandError> {
    state
        .reveal(&id)
        .await
        .ok_or_else(|| CommandError::invalid_input("找不到该 Key（可能已被删除）。"))
}

/// 删除一条 Key（连同它的余额快照）。
#[tauri::command]
pub async fn key_butler_delete(
    state: State<'_, KeyButlerState>,
    id: String,
) -> Result<Vec<KeyEntry>, CommandError> {
    state
        .ensure_writable()
        .await
        .map_err(CommandError::invalid_input)?;

    if !state.remove(&id).await {
        return Err(CommandError::invalid_input("找不到该 Key。"));
    }
    state.remove_snapshots(&id).await;
    state
        .persist_if_enabled()
        .await
        .map_err(CommandError::internal)?;

    Ok(state.entries().await)
}

/// 修改备注。
#[tauri::command]
pub async fn key_butler_update_note(
    state: State<'_, KeyButlerState>,
    id: String,
    note: String,
) -> Result<Vec<KeyEntry>, CommandError> {
    state
        .ensure_writable()
        .await
        .map_err(CommandError::invalid_input)?;

    let found = state
        .update(&id, move |entry| {
            entry.note = note;
        })
        .await;
    if !found {
        return Err(CommandError::invalid_input("找不到该 Key。"));
    }
    state
        .persist_if_enabled()
        .await
        .map_err(CommandError::internal)?;

    Ok(state.entries().await)
}

/// 修改 Base URL 与余额接口配置。
#[tauri::command]
pub async fn key_butler_set_config(
    state: State<'_, KeyButlerState>,
    id: String,
    patch: KeyConfigPatch,
) -> Result<Vec<KeyEntry>, CommandError> {
    state
        .ensure_writable()
        .await
        .map_err(CommandError::invalid_input)?;

    let found = state
        .update(&id, move |entry| {
            if let Some(base) = patch.base_url {
                let trimmed = base.trim().to_string();
                // 空字符串表示「清空」，归一为 None
                entry.base_url = if trimmed.is_empty() { None } else { Some(trimmed) };
            }
            if let Some(kind) = patch.balance_endpoint {
                entry.balance_endpoint = kind;
                // 切回非自定义时，清掉残留的自定义配置，避免下次误用
                if kind != BalanceEndpointKind::Custom {
                    entry.custom_endpoint = None;
                }
            }
            if let Some(custom) = patch.custom_endpoint {
                entry.custom_endpoint = Some(custom);
            }
        })
        .await;
    if !found {
        return Err(CommandError::invalid_input("找不到该 Key。"));
    }
    state
        .persist_if_enabled()
        .await
        .map_err(CommandError::internal)?;

    Ok(state.entries().await)
}

/* ---------------------------------------------------------------------------
 * 命令 9 ~ 12：加密保存
 * ------------------------------------------------------------------------- */

/// 启用加密保存（设置主密码并加密当前列表）。
#[tauri::command]
pub async fn key_butler_vault_enable(
    state: State<'_, KeyButlerState>,
    password: String,
) -> Result<VaultStatus, CommandError> {
    state
        .enable(&password)
        .await
        .map_err(CommandError::invalid_input)
}

/// 用主密码解锁已有密文库。
#[tauri::command]
pub async fn key_butler_vault_unlock(
    state: State<'_, KeyButlerState>,
    password: String,
) -> Result<VaultStatus, CommandError> {
    state
        .unlock(&password)
        .await
        .map_err(CommandError::invalid_input)
}

/// 关闭加密保存（删除本地密文，内存表保留）。
#[tauri::command]
pub async fn key_butler_vault_disable(
    state: State<'_, KeyButlerState>,
) -> Result<VaultStatus, CommandError> {
    state.disable().await.map_err(CommandError::internal)
}

/// 查询加密保存状态。
#[tauri::command]
pub async fn key_butler_vault_status(
    state: State<'_, KeyButlerState>,
) -> Result<VaultStatus, CommandError> {
    Ok(state.status().await)
}

/* ---------------------------------------------------------------------------
 * 命令 13：余额快照
 * ------------------------------------------------------------------------- */

/// 查询某条 Key 的余额快照序列（按时间升序）。
///
/// 返回的是「内存缓冲 + 磁盘文件」的合并结果：未启用加密保存时
/// 磁盘上没有文件，此时只返回本次会话内累积的记录。
#[tauri::command]
pub async fn key_butler_snapshots(
    state: State<'_, KeyButlerState>,
    key_id: String,
) -> Result<Vec<Snapshot>, CommandError> {
    Ok(state.list_snapshots(&key_id).await)
}

/* ---------------------------------------------------------------------------
 * 单元测试
 * ------------------------------------------------------------------------- */

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_keys_finds_multiple_prefixes() {
        let text = "第一行 sk-ant-api03-abcdefghijklmn\n\
                    第二行 gsk_abcdefghijklmnop\n\
                    apiKey: xai-abcdefghijklmn";
        let keys = extract_keys(text);
        assert!(keys.contains(&"sk-ant-api03-abcdefghijklmn".to_string()));
        assert!(keys.contains(&"gsk_abcdefghijklmnop".to_string()));
        assert!(keys.contains(&"xai-abcdefghijklmn".to_string()));
    }

    #[test]
    fn extract_keys_dedupes_and_keeps_order() {
        let text = "sk-aaaaaaaaaaaaaaaa\nsk-bbbbbbbbbbbbbbbb\nsk-aaaaaaaaaaaaaaaa";
        let keys = extract_keys(text);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0], "sk-aaaaaaaaaaaaaaaa");
        assert_eq!(keys[1], "sk-bbbbbbbbbbbbbbbb");
    }

    #[test]
    fn extract_keys_ignores_urls_and_domains() {
        let text = "https://api.example.com/v1\n\
                    参考 discovery-api.intern-ai.org.cn 这个域名\n\
                    apiKey: sk-real-key-abcdefgh";
        let keys = extract_keys(text);
        assert!(keys.contains(&"sk-real-key-abcdefgh".to_string()));
        assert!(
            !keys.iter().any(|k| k.contains("example.com")),
            "URL 不应被当成 Key：{keys:?}"
        );
        assert!(
            !keys.iter().any(|k| k.contains("intern-ai.org.cn")),
            "域名不应被当成 Key：{keys:?}"
        );
    }

    /// 回归：用户实测反馈 —— 这种 Key 曾被识别成两条。
    ///
    /// `sk-` 后面跟 64 位十六进制，会同时命中「已知前缀」与「纯十六进制」
    /// 两条规则；必须只保留完整的那一条。
    #[test]
    fn extract_keys_keeps_sk_key_with_hex_body_as_one() {
        let key = "sk-2ee1cb8ab47818520921d3bba9deb580dfb82e345ee96841bbff13aad03d82bf";
        let keys = extract_keys(key);
        assert_eq!(keys, vec![key.to_string()], "实际结果：{keys:?}");
    }

    #[test]
    fn extract_keys_handles_bearer_and_hex() {        let text = "Authorization: Bearer abcdefghijklmnopqrst\n\
                    raw=0123456789abcdef0123456789abcdef";
        let keys = extract_keys(text);
        assert!(keys.iter().any(|k| k == "abcdefghijklmnopqrst"));
        assert!(keys
            .iter()
            .any(|k| k == "0123456789abcdef0123456789abcdef"));
    }

    #[test]
    fn endpoint_kind_parsing_matches_contract_values() {
        assert_eq!(
            parse_endpoint_kind("builtin-oneapi"),
            BalanceEndpointKind::BuiltinOneapi
        );
        assert_eq!(parse_endpoint_kind("custom"), BalanceEndpointKind::Custom);
        assert_eq!(parse_endpoint_kind("none"), BalanceEndpointKind::None);
        assert_eq!(parse_endpoint_kind("随便什么"), BalanceEndpointKind::None);
    }

    #[test]
    fn build_entry_fills_platform_and_masks_key() {
        let entry = build_entry("sk-ant-api03-abcdefghijklmn");
        assert_eq!(entry.platform, "anthropic");
        assert_eq!(entry.masked_key, "sk-an***klmn");
        assert_eq!(entry.balance_endpoint, BalanceEndpointKind::None);
        assert_eq!(entry.base_url.as_deref(), Some("https://api.anthropic.com/v1"));
        assert!(entry.last_checked_at.is_none());
        assert_eq!(entry.status, KeyStatus::Error);
    }

    #[test]
    fn status_strings_match_contract() {
        assert_eq!(key_status_str(KeyStatus::Active), "active");
        assert_eq!(key_status_str(KeyStatus::Insufficient), "insufficient");
        assert_eq!(key_status_str(KeyStatus::Invalid), "invalid");
        assert_eq!(key_status_str(KeyStatus::Error), "error");
    }

    #[test]
    fn error_kind_strings_are_camel_case() {
        assert_eq!(error_kind_str(ProbeErrorKind::KeyInvalid), "keyInvalid");
        assert_eq!(error_kind_str(ProbeErrorKind::BaseUrlInvalid), "baseUrlInvalid");
        assert_eq!(error_kind_str(ProbeErrorKind::NetworkTimeout), "networkTimeout");
    }
}
