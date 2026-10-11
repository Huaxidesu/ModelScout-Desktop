/**
 * ============================================================================
 * ModelScout —— Key 管家：余额查询适配器
 * ============================================================================
 *
 * 【现实约束（任务书第四节）】
 *   - OpenAI 官方**已下线**公开余额接口；Anthropic **没有**公开余额端点。
 *   - 真正能查到余额的主要是 one-api / new-api 系中转站：
 *       GET {host}/v1/dashboard/billing/subscription  → 总额度
 *       GET {host}/v1/dashboard/billing/usage         → 已用额度（**单位是美分**）
 *   因此必须做适配器，而不是假设「所有平台都能查余额」。
 *
 * 【铁律】余额查询失败 **≠** Key 失效。
 * 本模块的失败只反映在 `BalanceOutcome.error` 里，由调用方决定怎么展示；
 * 它绝不参与 `keyValid` 的判定。
 *
 * 【{base} 占位语义（重要）】
 *   - `{base}`    → **去掉版本段的主机地址**，如 `https://relay.example.com`
 *                   （与任务书示例 `{base}/v1/dashboard/billing/subscription` 一致）
 *   - `{baseUrl}` → 原样存储的 Base URL，如 `https://relay.example.com/v1`
 *   提供两种是为了兼容「用户填的 Base URL 带不带 /v1」两种情况。
 * ============================================================================
 */

use std::time::Duration;

use serde_json::Value;

use crate::error::{scrub_secrets, ProbeErrorKind};
use crate::key_butler::models::{BalanceEndpoint, BalanceEndpointKind, HttpMethod};

/// 一次余额查询的结果。
#[derive(Debug, Clone, Default)]
pub struct BalanceOutcome {
    /// 余额数值；`None` 表示未知
    pub balance: Option<f64>,
    /// 币种（内置适配器固定 USD）
    pub currency: Option<String>,
    /// 失败原因（分类 + 中文说明）；成功或「平台无接口」时为 `None`
    pub error: Option<(ProbeErrorKind, String)>,
    /// 面向用户的补充说明（如「该平台无公开余额接口」）
    pub note: Option<String>,
}

impl BalanceOutcome {
    /// 余额是否可信。
    pub fn is_known(&self) -> bool {
        self.balance.is_some() && self.error.is_none()
    }

    /// 「该平台没有公开余额接口」这一中性结果。
    fn no_endpoint() -> Self {
        Self {
            balance: None,
            currency: None,
            error: None,
            note: Some("该平台没有公开的余额查询接口，余额需到服务商后台查看。".to_string()),
        }
    }

    /// 构造一个失败结果。
    fn failed(kind: ProbeErrorKind, message: impl Into<String>) -> Self {
        Self {
            balance: None,
            currency: None,
            error: Some((kind, message.into())),
            note: None,
        }
    }
}

/* ---------------------------------------------------------------------------
 * 主入口
 * ------------------------------------------------------------------------- */

/// 按适配器类型查询余额。
///
/// # 参数
/// - `client`: 复用的 reqwest 客户端（由调用方控制超时与连接池）
/// - `base_url`: 该 Key 的 Base URL（可能带也可能不带版本段）
/// - `api_key`: Key 明文，仅用于构造 Authorization 头，**不得**进日志
pub async fn query(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    kind: BalanceEndpointKind,
    custom: Option<&BalanceEndpoint>,
) -> BalanceOutcome {
    let base = base_url.trim();
    if base.is_empty() {
        return BalanceOutcome::failed(
            ProbeErrorKind::Unknown,
            "该 Key 尚未设置 Base URL，无法查询余额。",
        );
    }

    match kind {
        BalanceEndpointKind::None => BalanceOutcome::no_endpoint(),
        BalanceEndpointKind::BuiltinOneapi => query_oneapi(client, base, api_key).await,
        BalanceEndpointKind::Custom => match custom {
            Some(endpoint) => query_custom(client, base, api_key, endpoint).await,
            None => BalanceOutcome::failed(
                ProbeErrorKind::Unknown,
                "余额接口选择了「自定义」，但没有填写自定义配置。",
            ),
        },
    }
}

/* ---------------------------------------------------------------------------
 * 内置：one-api / new-api 系
 * ------------------------------------------------------------------------- */

/// one-api 系中转站的余额查询。
///
/// 两个接口分别拿总额度与已用额度，相减得到余额。
async fn query_oneapi(client: &reqwest::Client, base_url: &str, api_key: &str) -> BalanceOutcome {
    let host = strip_version_suffix(base_url);

    let subscription_url = format!("{host}/v1/dashboard/billing/subscription");
    let usage_url = format!("{host}/v1/dashboard/billing/usage");

    let subscription = match get_json(client, &subscription_url, api_key).await {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };

    // 各中转实现的字段名不统一，两个都试
    let total = json_path(&subscription, "hard_limit_usd")
        .and_then(as_f64)
        .or_else(|| json_path(&subscription, "system_hard_limit_usd").and_then(as_f64));

    let Some(total) = total else {
        return BalanceOutcome::failed(
            ProbeErrorKind::ParseError,
            "余额接口返回的 JSON 里既没有 hard_limit_usd 也没有 system_hard_limit_usd，\
             无法解析总额度。可在该 Key 上改用「自定义」余额接口。",
        );
    };

    let usage = match get_json(client, &usage_url, api_key).await {
        Ok(value) => value,
        Err(outcome) => return outcome,
    };

    let Some(used_cents) = json_path(&usage, "total_usage").and_then(as_f64) else {
        return BalanceOutcome::failed(
            ProbeErrorKind::ParseError,
            "用量接口返回的 JSON 里没有 total_usage 字段，无法解析已用额度。",
        );
    };

    // 关键：one-api 的 total_usage 单位是**美分**，除以 100 才是美元
    let used_usd = used_cents / 100.0;

    BalanceOutcome {
        balance: Some(total - used_usd),
        currency: Some("USD".to_string()),
        error: None,
        note: None,
    }
}

/// 发一个 GET 并解析为 JSON；失败时返回已经构造好的 `BalanceOutcome`。
async fn get_json(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
) -> Result<Value, BalanceOutcome> {
    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {api_key}"))
        // 部分网关不带 Accept 会返回 406（探测模块踩过的同一个坑）
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|err| {
            let (kind, message) = classify_reqwest_error(&err, api_key);
            BalanceOutcome::failed(kind, message)
        })?;

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    // 来自上游的正文可能回显请求头，必须脱敏后再进入错误信息
    let scrubbed = scrub_secrets(&text, api_key);

    if !status.is_success() {
        return Err(BalanceOutcome::failed(
            classify_status(status.as_u16()),
            format!(
                "余额接口返回 HTTP {}：{}",
                status.as_u16(),
                summarize(&scrubbed)
            ),
        ));
    }

    serde_json::from_str::<Value>(&scrubbed).map_err(|_| {
        BalanceOutcome::failed(
            ProbeErrorKind::ParseError,
            format!("余额接口返回的不是合法 JSON：{}", summarize(&scrubbed)),
        )
    })
}

/* ---------------------------------------------------------------------------
 * 自定义适配器
 * ------------------------------------------------------------------------- */

/// 用户自定义余额接口。
async fn query_custom(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    endpoint: &BalanceEndpoint,
) -> BalanceOutcome {
    let url = expand_template(&endpoint.url_template, base_url);
    if url.trim().is_empty() {
        return BalanceOutcome::failed(
            ProbeErrorKind::Unknown,
            "自定义余额接口的 URL 模板为空。",
        );
    }

    let mut request = match endpoint.method {
        HttpMethod::Get => client.get(&url),
        HttpMethod::Post => client.post(&url),
    };

    request = request
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json");

    for (name, value) in &endpoint.extra_headers {
        request = request.header(name.as_str(), value.as_str());
    }

    let response = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            let (kind, message) = classify_reqwest_error(&err, api_key);
            return BalanceOutcome::failed(kind, message);
        }
    };

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let scrubbed = scrub_secrets(&text, api_key);

    if !status.is_success() {
        return BalanceOutcome::failed(
            classify_status(status.as_u16()),
            format!(
                "自定义余额接口返回 HTTP {}：{}",
                status.as_u16(),
                summarize(&scrubbed)
            ),
        );
    }

    let value = match serde_json::from_str::<Value>(&scrubbed) {
        Ok(value) => value,
        Err(_) => {
            return BalanceOutcome::failed(
                ProbeErrorKind::ParseError,
                format!("自定义余额接口返回的不是合法 JSON：{}", summarize(&scrubbed)),
            )
        }
    };

    // 除数：0 或负数没有意义，按 1 处理（防御性）
    let divisor = if endpoint.unit_divisor.is_finite() && endpoint.unit_divisor > 0.0 {
        endpoint.unit_divisor
    } else {
        1.0
    };

    // 三种取值方式，优先级：直接余额 > 总额度-已用
    if let Some(raw) = endpoint.balance_path.as_deref().and_then(|p| json_path(&value, p)) {
        if let Some(number) = as_f64(raw) {
            return BalanceOutcome {
                balance: Some(number / divisor),
                currency: None,
                error: None,
                note: None,
            };
        }
    }

    let total = endpoint
        .total_path
        .as_deref()
        .and_then(|p| json_path(&value, p))
        .and_then(as_f64);
    let used = endpoint
        .used_path
        .as_deref()
        .and_then(|p| json_path(&value, p))
        .and_then(as_f64);

    match (total, used) {
        (Some(total), Some(used)) => BalanceOutcome {
            balance: Some((total - used) / divisor),
            currency: None,
            error: None,
            note: None,
        },
        (Some(total), None) => BalanceOutcome {
            balance: Some(total / divisor),
            currency: None,
            error: None,
            note: None,
        },
        _ => BalanceOutcome::failed(
            ProbeErrorKind::ParseError,
            "自定义余额接口的取值路径没能从响应里取到数值。\
             请检查「直接余额路径」或「总额度路径 + 已用路径」是否与返回 JSON 匹配。",
        ),
    }
}

/* ---------------------------------------------------------------------------
 * 工具
 * ------------------------------------------------------------------------- */

/// 去掉 Base URL 末尾的版本段（`/v1`、`/v2`、`/api/v3` 之类），得到主机地址。
///
/// 这样无论用户填的是 `https://relay.com` 还是 `https://relay.com/v1`，
/// 拼出来的余额接口地址都是同一个。
pub fn strip_version_suffix(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(index) => {
            let last = &trimmed[index + 1..];
            let looks_like_version = last.len() >= 2
                && last.len() <= 5
                && last.starts_with('v')
                && last[1..].chars().all(|c| c.is_ascii_digit());
            if looks_like_version {
                trimmed[..index].to_string()
            } else {
                trimmed.to_string()
            }
        }
        None => trimmed.to_string(),
    }
}

/// 展开 URL 模板里的 `{base}` / `{baseUrl}` 占位。
pub fn expand_template(template: &str, base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    template
        .replace("{baseUrl}", trimmed)
        .replace("{base}", &strip_version_suffix(trimmed))
}

/// 按点号路径从 JSON 里取值，支持数组下标（如 `data.items.0.balance`）。
pub fn json_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.').filter(|s| !s.trim().is_empty()) {
        current = match current {
            Value::Object(map) => map.get(segment)?,
            Value::Array(list) => list.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// 把 JSON 值转成 f64；数字与「数字字符串」都接受。
pub fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// 截断过长的响应正文，避免错误信息里塞进整页 HTML。
fn summarize(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= 200 {
        collapsed
    } else {
        let head: String = collapsed.chars().take(200).collect();
        format!("{head}…")
    }
}

/// HTTP 状态码 → 错误分类。
fn classify_status(status: u16) -> ProbeErrorKind {
    match status {
        401 | 403 => ProbeErrorKind::KeyInvalid,
        404 => ProbeErrorKind::BaseUrlInvalid,
        429 => ProbeErrorKind::RateLimited,
        500..=599 => ProbeErrorKind::ServerError,
        _ => ProbeErrorKind::Unknown,
    }
}

/// reqwest 传输错误 → 错误分类 + 中文说明（已脱敏）。
fn classify_reqwest_error(err: &reqwest::Error, api_key: &str) -> (ProbeErrorKind, String) {
    let raw = scrub_secrets(&err.to_string(), api_key);
    if err.is_timeout() {
        (
            ProbeErrorKind::NetworkTimeout,
            format!("余额查询超时：{raw}"),
        )
    } else if err.is_connect() {
        (
            ProbeErrorKind::BaseUrlInvalid,
            format!("无法连接到余额接口（域名解析失败或连接被拒）：{raw}"),
        )
    } else {
        (
            ProbeErrorKind::NetworkTimeout,
            format!("余额查询失败：{raw}"),
        )
    }
}

/// 供命令层复用的超时构造（与探测模块保持同一量级）。
pub fn timeout_from_secs(secs: u64) -> Duration {
    Duration::from_secs(secs.clamp(5, 300))
}

/* ---------------------------------------------------------------------------
 * 单元测试
 * ------------------------------------------------------------------------- */

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strip_version_suffix_handles_both_forms() {
        assert_eq!(
            strip_version_suffix("https://relay.example.com/v1"),
            "https://relay.example.com"
        );
        assert_eq!(
            strip_version_suffix("https://relay.example.com"),
            "https://relay.example.com"
        );
        assert_eq!(
            strip_version_suffix("https://relay.example.com/v1/"),
            "https://relay.example.com"
        );
        assert_eq!(
            strip_version_suffix("https://relay.example.com/api/v3"),
            "https://relay.example.com/api"
        );
        // 路径最后一段不是版本号时保持原样
        assert_eq!(
            strip_version_suffix("https://relay.example.com/compatible-mode"),
            "https://relay.example.com/compatible-mode"
        );
    }

    #[test]
    fn expand_template_supports_both_placeholders() {
        let base = "https://relay.example.com/v1";
        assert_eq!(
            expand_template("{base}/v1/dashboard/billing/subscription", base),
            "https://relay.example.com/v1/dashboard/billing/subscription"
        );
        assert_eq!(
            expand_template("{baseUrl}/dashboard/billing/usage", base),
            "https://relay.example.com/v1/dashboard/billing/usage"
        );
    }

    #[test]
    fn json_path_supports_nested_and_index() {
        let value = json!({ "data": { "items": [ { "balance": 12.5 } ] } });
        assert_eq!(
            json_path(&value, "data.items.0.balance").and_then(as_f64),
            Some(12.5)
        );
        assert_eq!(json_path(&value, "data.missing"), None);
    }

    #[test]
    fn as_f64_accepts_numeric_strings() {
        assert_eq!(as_f64(&json!("3.14")), Some(3.14));
        assert_eq!(as_f64(&json!(7)), Some(7.0));
        assert_eq!(as_f64(&json!("not-a-number")), None);
    }

    #[test]
    fn oneapi_cents_conversion_is_exact() {
        // one-api 的 total_usage 单位是美分：1234 → 12.34 美元
        let total = 100.0_f64;
        let used_cents = 1234.0_f64;
        let balance = total - used_cents / 100.0;
        assert!((balance - 87.66).abs() < 1e-9, "实际 {balance}");
    }

    #[test]
    fn no_endpoint_result_is_neutral_not_error() {
        let outcome = BalanceOutcome::no_endpoint();
        assert!(outcome.error.is_none(), "「无公开接口」不应算失败");
        assert!(!outcome.is_known());
        assert!(outcome.note.is_some());
    }

    #[test]
    fn status_classification_matches_probe_conventions() {
        assert_eq!(classify_status(401), ProbeErrorKind::KeyInvalid);
        assert_eq!(classify_status(403), ProbeErrorKind::KeyInvalid);
        assert_eq!(classify_status(404), ProbeErrorKind::BaseUrlInvalid);
        assert_eq!(classify_status(429), ProbeErrorKind::RateLimited);
        assert_eq!(classify_status(503), ProbeErrorKind::ServerError);
    }
}
