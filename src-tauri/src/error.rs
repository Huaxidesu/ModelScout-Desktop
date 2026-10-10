/**
 * ============================================================================
 * ModelScout —— 错误分类模块
 * ============================================================================
 *
 * 【职责】
 * 把各种来源的失败（reqwest 传输错误、HTTP 非 2xx 状态码、JSON 解析失败、
 * 用户主动中止）统一归约为 `ProbeError`，使前端能够按类别给出明确的
 * 排障指引（任务书第五节「错误分类」要求）：
 *
 *   401 / 403  → Key 无效
 *   404        → Base URL 错误
 *   超时       → 网络问题
 *   429        → 限流，建议降低并发
 *   5xx        → 服务端故障
 *
 * 【安全约束】
 * 本模块以及全项目的任何 `format!` / 日志输出中，**禁止**出现完整 API Key。
 * `redact_key()` 提供统一的脱敏工具。
 * ============================================================================
 */

use serde::{Deserialize, Serialize};

use crate::models::ProbeStage;

/* ---------------------------------------------------------------------------
 * 错误分类枚举
 * ------------------------------------------------------------------------- */

/// 探测错误的分类标签。
///
/// `#[serde(rename_all = "camelCase")]` 保证序列化到前端时为：
/// `keyInvalid` / `baseUrlInvalid` / `networkTimeout` ...
/// 与 `src/types/contracts.ts` 的 `ProbeErrorKind` 一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProbeErrorKind {
    /// HTTP 401 / 403：API Key 无效或权限不足
    KeyInvalid,
    /// HTTP 404 或域名解析失败：Base URL 写错
    BaseUrlInvalid,
    /// 连接超时 / 读取超时：网络问题
    NetworkTimeout,
    /// HTTP 429：触发限流
    RateLimited,
    /// HTTP 5xx：服务端故障
    ServerError,
    /// 该模型对当前 Key 不可用
    ModelNotFound,
    /// 响应不是合法 JSON，或结构不符合 OpenAI 规范
    ParseError,
    /// 用户主动中止
    Cancelled,
    /// 未能归类的其他错误
    Unknown,
}

impl ProbeErrorKind {
    /// 返回面向用户的中文分类名，用于前端汇总卡片展示。
    pub fn label(&self) -> &'static str {
        match self {
            ProbeErrorKind::KeyInvalid => "Key 无效",
            ProbeErrorKind::BaseUrlInvalid => "Base URL 错误",
            ProbeErrorKind::NetworkTimeout => "网络问题",
            ProbeErrorKind::RateLimited => "触发限流",
            ProbeErrorKind::ServerError => "服务端故障",
            ProbeErrorKind::ModelNotFound => "模型不可用",
            ProbeErrorKind::ParseError => "响应解析失败",
            ProbeErrorKind::Cancelled => "用户中止",
            ProbeErrorKind::Unknown => "未知错误",
        }
    }
}

/* ---------------------------------------------------------------------------
 * 单条错误结构体
 * ------------------------------------------------------------------------- */

/// 一条探测错误。
///
/// 与前端 `ProbeError` 接口一一对应。
/// `rename_all = "camelCase"` 使 `http_status` → `httpStatus`、
/// `stage` → `stage`（已是单词，无变化）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeError {
    /// 错误分类
    pub kind: ProbeErrorKind,
    /// 面向用户的简短中文说明
    pub message: String,
    /// HTTP 状态码；0 表示错误发生在 HTTP 层之前（如 DNS 失败、超时）
    pub http_status: u16,
    /// 触发该错误的探测环节
    pub stage: ProbeStage,
}

impl ProbeError {
    /// 构造一条错误。`message` 由调用方给出具体上下文。
    pub fn new(
        kind: ProbeErrorKind,
        message: impl Into<String>,
        http_status: u16,
        stage: ProbeStage,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            http_status,
            stage,
        }
    }

    /// 依据 HTTP 状态码构造一条**带默认中文说明**的错误。
    ///
    /// 这是最常用的构造入口：调用方只需给出状态码与所处阶段，
    /// 分类与文案由本函数统一决定，保证全项目文案一致。
    pub fn from_http_status(status: u16, stage: ProbeStage) -> Self {
        let (kind, message) = match status {
            400 => (
                ProbeErrorKind::Unknown,
                "服务端返回 400（请求格式不被接受）。可能是该模型不支持此请求形态。".to_string(),
            ),
            401 => (
                ProbeErrorKind::KeyInvalid,
                "服务端返回 401：API Key 无效或未提供。请检查 Key 是否正确、是否已过期。"
                    .to_string(),
            ),
            402 => (
                ProbeErrorKind::KeyInvalid,
                "服务端返回 402：账户余额不足或计费异常。".to_string(),
            ),
            403 => (
                ProbeErrorKind::KeyInvalid,
                "服务端返回 403：Key 权限不足，或该模型未对当前账号开放。".to_string(),
            ),
            404 => (
                ProbeErrorKind::BaseUrlInvalid,
                "服务端返回 404：Base URL 路径错误，或该模型不存在于当前端点。请确认 Base URL 形如 https://host/v1。"
                    .to_string(),
            ),
            408 => (
                ProbeErrorKind::NetworkTimeout,
                "服务端返回 408：请求超时。".to_string(),
            ),
            413 => (
                ProbeErrorKind::Unknown,
                "服务端返回 413：请求体过大，可能因此拒绝了大 max_tokens 探测。".to_string(),
            ),
            422 => (
                ProbeErrorKind::Unknown,
                "服务端返回 422：参数校验失败，可能是该模型不支持此参数。".to_string(),
            ),
            429 => (
                ProbeErrorKind::RateLimited,
                "服务端返回 429：触发限流。建议把并发数降到 1-2 后重试。".to_string(),
            ),
            500..=599 => (
                ProbeErrorKind::ServerError,
                format!("服务端返回 {status}：上游服务故障，请稍后重试。"),
            ),
            _ => (
                ProbeErrorKind::Unknown,
                format!("服务端返回未预期的状态码 {status}。"),
            ),
        };

        Self::new(kind, message, status, stage)
    }

    /// 在 [`Self::from_http_status`] 的基础上，**用响应正文细化分类**。
    ///
    /// 典型场景：部分聚合网关对「模型不存在 / 未开通」也返回 **404**，
    /// 但正文里写明 `"code": "model_not_available"`。
    /// 此时若一律归为「Base URL 错误」，会把用户引向错误的排查方向
    /// （去改地址），而真正该做的是**换一个模型**或确认订阅状态。
    pub fn from_http_status_with_body(status: u16, stage: ProbeStage, body: &str) -> Self {
        let mut err = Self::from_http_status(status, stage);

        if status == 404 {
            let lower = body.to_lowercase();
            let model_missing = lower.contains("model_not_available")
                || lower.contains("model not available")
                || lower.contains("model_not_found")
                || lower.contains("model not found")
                || lower.contains("no such model")
                || lower.contains("模型不存在")
                || lower.contains("未开通")
                || (lower.contains("不支持") && lower.contains("model"));
            if model_missing {
                err.kind = ProbeErrorKind::ModelNotFound;
                err.message = "服务端返回 404，但响应正文表明是「模型不可用 / 未开通」，\
                               而非 Base URL 错误。该模型可能未对当前账号开放，\
                               建议更换模型或确认订阅状态。"
                    .to_string();
            }
        }

        err
    }

    /// 依据 reqwest 传输错误构造。区分超时与域名解析失败/连接被拒。
    ///
    /// 判定顺序很关键：
    /// 1. 先看 `is_timeout()` —— 超时是明确的网络问题
    /// 2. 再看错误链里是否含 DNS/连接失败关键字 —— 通常是 Base URL 写错
    pub fn from_reqwest_error(err: &reqwest::Error, stage: ProbeStage) -> Self {
        if err.is_timeout() {
            return Self::new(
                ProbeErrorKind::NetworkTimeout,
                "请求超时：网络不通或服务端响应过慢。可尝试调大超时时间或检查代理设置。",
                0,
                stage,
            );
        }

        // 收集完整错误链文本用于关键字判定（reqwest 的错误是分层的）
        let mut chain = err.to_string();
        let mut source: Option<&dyn std::error::Error> = std::error::Error::source(err);
        while let Some(inner) = source {
            chain.push_str(" | ");
            chain.push_str(&inner.to_string());
            source = inner.source();
        }
        let lower = chain.to_lowercase();

        if lower.contains("dns")
            || lower.contains("failed to lookup address")
            || lower.contains("name or service not known")
            || lower.contains("no such host")
            || lower.contains("nodename nor servname")
        {
            return Self::new(
                ProbeErrorKind::BaseUrlInvalid,
                "域名解析失败：Base URL 中的主机名无法解析，请检查是否拼写错误或缺少协议前缀。",
                0,
                stage,
            );
        }

        if lower.contains("connection refused") || lower.contains("actively refused") {
            return Self::new(
                ProbeErrorKind::BaseUrlInvalid,
                "连接被拒绝：目标主机端口未监听。请检查 Base URL 的端口号是否正确。",
                0,
                stage,
            );
        }

        if lower.contains("certificate") || lower.contains("tls") || lower.contains("ssl") {
            return Self::new(
                ProbeErrorKind::NetworkTimeout,
                "TLS 握手失败：证书校验未通过。若使用自建网关，请确认其证书有效。",
                0,
                stage,
            );
        }

        if lower.contains("connection reset") || lower.contains("broken pipe") {
            return Self::new(
                ProbeErrorKind::NetworkTimeout,
                "连接被中断：可能由代理或防火墙导致，请检查网络环境。",
                0,
                stage,
            );
        }

        if err.is_decode() {
            return Self::new(
                ProbeErrorKind::ParseError,
                "响应体解码失败：服务端返回的内容不是合法 JSON。请确认 Base URL 指向的是 OpenAI 兼容端点。",
                0,
                stage,
            );
        }

        if err.is_connect() {
            return Self::new(
                ProbeErrorKind::NetworkTimeout,
                "无法建立连接：请检查网络、代理或 Base URL 是否可达。",
                0,
                stage,
            );
        }

        Self::new(
            ProbeErrorKind::Unknown,
            format!("请求失败：{chain}"),
            0,
            stage,
        )
    }

    /// 生成一条「用户中止」错误，用于被 `stop_probe` 打断的任务。
    pub fn cancelled(stage: ProbeStage) -> Self {
        Self::new(
            ProbeErrorKind::Cancelled,
            "探测已被用户中止。",
            0,
            stage,
        )
    }

    /// 给命令层使用的字符串形式（前端 invoke 的 Err 分支会拿到它）。
    pub fn to_user_string(&self) -> String {
        format!("[{}] {}", self.kind.label(), self.message)
    }
}

/* ---------------------------------------------------------------------------
 * 命令层错误
 * ------------------------------------------------------------------------- */

/// 命令层统一错误。所有 `#[tauri::command]` 返回 `Result<T, CommandError>`。
///
/// 序列化到前端后是一个对象：`{ "kind": "keyInvalid", "message": "...", "httpStatus": 401 }`，
/// 前端可以据此做差异化提示，而不是只拿到一个裸字符串。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandError {
    /// 复用的错误分类
    pub kind: ProbeErrorKind,
    /// 人类可读中文说明（已脱敏，绝不含完整 Key）
    pub message: String,
    /// 关联的 HTTP 状态码，0 表示非 HTTP 层
    pub http_status: u16,
}

impl CommandError {
    pub fn new(kind: ProbeErrorKind, message: impl Into<String>, http_status: u16) -> Self {
        Self {
            kind,
            message: message.into(),
            http_status,
        }
    }

    /// 从 `ProbeError` 提升而来
    pub fn from_probe(err: &ProbeError) -> Self {
        Self {
            kind: err.kind,
            message: err.message.clone(),
            http_status: err.http_status,
        }
    }

    /// 参数校验失败
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ProbeErrorKind::Unknown, message, 0)
    }

    /// 内部错误（锁中毒、序列化失败等）
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ProbeErrorKind::Unknown, message, 0)
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.kind.label(), self.message)
    }
}

impl std::error::Error for CommandError {}

/* ---------------------------------------------------------------------------
 * 敏感信息脱敏
 * ------------------------------------------------------------------------- */

/// 把 API Key 脱敏成可安全打印的形式。
///
/// 规则：
/// - 长度 <= 8  ：全部替换为 `***`
/// - 长度 >  8  ：保留前 3 位与后 4 位，中间用 `…` 连接
///
/// 本函数是**全项目唯一**允许触碰 Key 明文的出口，
/// 除它之外任何地方的日志都不应包含 Key。
pub fn redact_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return "(空)".to_string();
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 8 {
        return "***".to_string();
    }
    let head: String = chars.iter().take(3).collect();
    let tail: String = chars.iter().skip(chars.len() - 4).collect();
    format!("{head}…{tail}")
}

/// 把可能出现在错误正文里的 Key 明文替换掉。
///
/// 某些服务端的错误响应会回显请求头，因此所有**来自上游响应体**的文本
/// 在对外暴露前都必须过一遍本函数。
pub fn scrub_secrets(text: &str, key: &str) -> String {
    if key.trim().is_empty() {
        return text.to_string();
    }
    text.replace(key.trim(), "***REDACTED***")
}

/* ---------------------------------------------------------------------------
 * 单元测试
 * ------------------------------------------------------------------------- */

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_status_classification_matches_spec() {
        // 任务书第五节要求：401/403 → Key 无效，404 → Base URL 错误
        assert_eq!(
            ProbeError::from_http_status(401, ProbeStage::ToolProbe).kind,
            ProbeErrorKind::KeyInvalid
        );
        assert_eq!(
            ProbeError::from_http_status(403, ProbeStage::ToolProbe).kind,
            ProbeErrorKind::KeyInvalid
        );
        assert_eq!(
            ProbeError::from_http_status(404, ProbeStage::ToolProbe).kind,
            ProbeErrorKind::BaseUrlInvalid
        );
        assert_eq!(
            ProbeError::from_http_status(429, ProbeStage::ToolProbe).kind,
            ProbeErrorKind::RateLimited
        );
        assert_eq!(
            ProbeError::from_http_status(503, ProbeStage::ToolProbe).kind,
            ProbeErrorKind::ServerError
        );
        assert_eq!(
            ProbeError::from_http_status(401, ProbeStage::ToolProbe).http_status,
            401
        );
    }

    #[test]
    fn redact_key_never_leaks_full_secret() {
        let secret = "sk-proj-abcdefghijklmnop1234";
        let redacted = redact_key(secret);
        assert!(!redacted.contains("abcdefghij"));
        assert!(redacted.starts_with("sk-"));
        assert_eq!(redact_key("short"), "***");
        assert_eq!(redact_key(""), "(空)");
    }

    #[test]
    fn scrub_secrets_removes_key_from_upstream_text() {
        let key = "sk-test-1234567890";
        let upstream = format!("invalid api key: {key} provided");
        let scrubbed = scrub_secrets(&upstream, key);
        assert!(!scrubbed.contains(key));
        assert!(scrubbed.contains("REDACTED"));
    }
}
