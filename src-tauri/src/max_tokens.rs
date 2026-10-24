//! 超大 max_tokens 报错解析器。
//!
//! 背景：模型 ID 未命中本地字典时，发送超大 max_tokens（如 999999）
//! 让服务端报错，再从错误信息里提取真实的上下文窗口 / 输出上限。
//! 可行依据是主流 OpenAI 兼容实现都会在错误里回显约束值，例如：
//!
//!  OpenAI / Azure:
//!    "Invalid 'max_tokens': integer above the maximum value. Please reduce
//!     your prompt or set max_tokens to at most 16384."
//!    "This model's maximum context length is 128000 tokens."
//!
//!  DeepSeek / Moonshot / 智谱:
//!    "max_tokens is too large: 999999. This model supports at most 8192
//!     completion tokens."
//!
//!  vLLM / Ollama / LM Studio（本地部署，报错风格差异最大）:
//!    "max_tokens 999999 is greater than the maximum model length 4096"
//!    "# 'max_tokens' must be less than or equal to 2048"
//!    "The requested max_tokens (999999) exceeds the maximum allowed output
//!     length of 4096 tokens."
//!
//! 本模块职责：用一组从严格到宽松的正则，从任意文本中提取
//! 单次输出上限（max output limit）与上下文窗口上限（context length），
//! 返回命中值与可信度，交由 `probe.rs` 决定是否采纳。
//!
//! 设计原则：
//! 1. 宁可返回 None 也不返回错误数字——编造的 128000 比「未知」伤害更大；
//! 2. 数字必须落在合理区间内（见 MIN/MAX 常量），超出即视为解析噪声丢弃；
//! 3. 支持 `128k` / `128K` / `1.5m` 缩写写法。


use once_cell::sync::Lazy;
use regex::Regex;

// ---- 合理区间约束 ----

/// 输出上限的合理最小值。小于此值的数字基本可以判定不是「输出上限」，
/// 而是错误信息里的其他数字（例如状态码、请求 id 片段）。
const MIN_OUTPUT_LIMIT: u64 = 256;

/// 输出上限的合理最大值（200 万 token 已覆盖当前全部已知模型）。
const MAX_OUTPUT_LIMIT: u64 = 4_000_000;

/// 上下文窗口的合理最小值。
const MIN_CONTEXT_LIMIT: u64 = 512;

/// 上下文窗口的合理最大值。
const MAX_CONTEXT_LIMIT: u64 = 32_000_000;

// ---- 正则集合 ----

/// 数字字面量：支持 `1234`、`1,234`、`1_234` 三种分组写法，也支持 `128k` / `1.5m` 缩写。
///
/// 正则拆解：
///   `(\d{1,3}(?:[,_]\d{3})+)`  → 带分组符的千位以上数字，如 128,000
///   `|(\d+(?:\.\d+)?)`         → 普通整数或小数，如 16384 / 1.5
///   `\s*([kKmM])?`             → 可选的 k/K/m/M 缩写后缀
const NUM: &str = r"(\d{1,3}(?:[,_]\d{3})+|\d+(?:\.\d+)?)\s*([kKmM])?";

/// 「输出上限」类正则，从严格到宽松排列。
///
/// 第一轮使用，因为「输出上限」的表述比「上下文上限」更具体，
/// 先匹配具体规则可避免 "maximum context length ... max_tokens" 这类
/// 复合句子被错误归类。
static OUTPUT_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    let sources = [
        // ---- 英文：显式点出 max_tokens / completion tokens ----
        // "set max_tokens to at most 16384"
        // "reduce ... to 16384 or fewer"
        r"(?i)max[_ ]?tokens?[^0-9]{0,60}?(?:at\s+most|to|<=|≤|of|is|:)\s*" .to_string() + NUM,
        // "maximum allowed max_tokens is 4096" / "max_tokens must be <= 4096"
        r"(?i)(?:maximum|max|limit)[^.\n]{0,40}?max[_ ]?tokens?[^0-9]{0,20}?" .to_string() + NUM,
        // "at most 8192 completion tokens"
        r"(?i)at\s+most\s+" .to_string() + NUM + r"(?:\s*[a-zA-Z]*)?\s*(?:completion|output)\s+tokens?",
        // "The maximum allowed output length of 4096 tokens"
        r"(?i)(?:maximum|max)[^.\n]{0,40}?(?:output|completion)[^.\n]{0,40}?" .to_string() + NUM,
        // "must be less than or equal to 2048"
        r"(?i)(?:less\s+than\s+or\s+equal\s+to|not\s+greater\s+than|≤)\s*" .to_string() + NUM,
        // "exceeds the maximum ... of 4096 tokens"
        r"(?i)exceed[s]?[^.\n]{0,60}?" .to_string() + NUM,
        // "maximum allowed is 16384" / "the limit is 16384" / "allowed up to 16384"
        //
        // 【为什么需要这条】很多网关在报错里直接用 "maximum allowed is N"
        // 这种极简表述，既不出现 max_tokens 也不出现 output / completion，
        // 前面的模式全都抓不到。实测 OpenAI 与若干中转网关都会这样回。
        r"(?i)(?:maximum|max|limit)[^.\n]{0,30}?(?:allowed|permitted|supported)?[^0-9]{0,20}?(?:is|of|=|:)\s*" .to_string() + NUM,
        // "greater than the maximum model length 4096"（vLLM / Ollama 风格）
        //
        // 【语义说明】vLLM 在请求的 max_tokens 超过模型总长度时会回这句。
        // 虽然字面写的是 "model length"，但当请求值就是 max_tokens 时，
        // 该约束实际作用于单次输出，因此归入输出上限。
        r"(?i)(?:greater|larger|more)\s+than[^.\n]{0,40}?(?:maximum|max|limit)[^.\n]{0,40}?" .to_string() + NUM,
        // ---- 中文：国内服务商常见表述 ----
        // "单次最大输出 8192" / "最大输出长度不能超过 8192"
        r"最大输出(?:长度)?" .to_string() + NUM,
        r"(?:输出|补全)[^0-9]{0,10}(?:上限|最大值)[^0-9]{0,10}?" .to_string() + NUM,
        // "最大允许 16384" / "上限为 16384"
        r"(?:最大|上限|限制)[^0-9]{0,12}?(?:为|是|不能超过|不超过)?\s*" .to_string() + NUM,
    ];
    sources
        .iter()
        .filter_map(|s| Regex::new(s).ok())
        .collect()
});

/// 「上下文窗口」类正则。
static CONTEXT_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    let sources = [
        // "maximum context length is 128000 tokens"
        r"(?i)(?:maximum|max)\s+context\s+(?:length|window|size)[^0-9]{0,40}?" .to_string() + NUM,
        // "context length of 128000"
        r"(?i)context\s+(?:length|window|size)[^0-9]{0,40}?" .to_string() + NUM,
        // "context_length_exceeded ... limit is 128000"
        r"(?i)context[_ ]?length[_ ]?exceeded[^0-9]{0,80}?" .to_string() + NUM,
        // "This model's maximum model length is 4096"（vLLM 风格）
        r"(?i)(?:maximum|max)\s+model\s+length[^0-9]{0,40}?" .to_string() + NUM,
        // "tokens in the messages exceed the maximum 32768"
        r"(?i)messages?[^.\n]{0,80}?(?:maximum|max|limit)[^0-9]{0,20}?" .to_string() + NUM,
        // ---- 中文 ----
        r"上下文(?:长度|窗口|上限)[^0-9]{0,20}?" .to_string() + NUM,
        r"最大上下文[^0-9]{0,20}?" .to_string() + NUM,
    ];
    sources
        .iter()
        .filter_map(|s| Regex::new(s).ok())
        .collect()
});

/// 用于判断文本是否「确实是在抱怨 token 数超限」。
///
/// 这个前置判断很重要：如果没有它，一个普通的 400 参数错误
/// （例如 "unsupported parameter temperature"）也可能被宽松正则
/// 抓到里面的某个数字，从而产生误判。
///
/// 【为什么必须包含 "maximum context length" 这类「中性」表述】
/// 很多服务端的上下文超限报错不含 too large / exceed 之类的词：
///
/// ```text
/// "This model's maximum context length is 128000 tokens.
///  However, your messages resulted in 130000 tokens."
/// ```
///
/// 这句话本身就是「你超限了」的陈述 —— 陈述里给了上限值，
/// 恰恰是最需要的信号。因此把它纳入 hint，否则会在前置检查处
/// 直接返回空结果，白白丢掉一个可用的上限值。
static TOO_LARGE_HINT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        concat!(
            r"(?i)(",
            // 直白型：too large / exceed / greater than 等
            r"too\s+large|too\s+big|too\s+high|exceed|greater\s+than",
            r"|above\s+the\s+maximum|out\s+of\s+range|invalid\s+max",
            // 上限陈述型：报错里给出「最大是 N」也是有效信号
            r"|maximum\s+(?:context|model|allowed|supported)",
            r"|max(?:imum)?\s+(?:context|model)\s+(?:length|window|size)",
            r"|context\s+length",
            r"|at\s+most|less\s+than\s+or\s+equal|not\s+greater\s+than|up\s+to",
            // 中文
            r"|降低|过大|超出|超过|不能大于|不能超过|不支持|上限|最大",
            r")",
        ),
    )
    .expect("TOO_LARGE_HINT 正则编译失败")
});

// ---- 解析结果 ----

/// 一次上限解析的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitParseResult {
    /// 解析出的单次输出上限；None 表示未解析到
    pub max_output_limit: Option<u64>,
    /// 解析出的上下文窗口上限；None 表示未解析到
    pub context_limit: Option<u64>,
    /// 命中的原文片段，用于在日志/前端 tooltip 中向用户展示依据
    pub evidence: String,
}

impl LimitParseResult {
    /// 是否什么都没解析到
    pub fn is_empty(&self) -> bool {
        self.max_output_limit.is_none() && self.context_limit.is_none()
    }
}

// ---- 数字提取工具 ----

/// 把正则捕获组 1（数字本体）与捕获组 2（k/m 后缀）还原为 u64。
///
/// 支持：
///   "128000"   → 128000
///   "128,000"  → 128000
///   "128_000"  → 128000
///   "128k"     → 131072
///   "1.5m"     → 1572864
fn parse_number(digits: &str, suffix: Option<&str>) -> Option<u64> {
    // 去掉千位分隔符
    let cleaned: String = digits.chars().filter(|c| *c != ',' && *c != '_').collect();
    let base: f64 = cleaned.parse().ok()?;

    if !base.is_finite() || base <= 0.0 {
        return None;
    }

    let multiplier: f64 = match suffix.map(|s| s.to_lowercase()).as_deref() {
        Some("k") => 1_024.0,
        Some("m") => 1_048_576.0,
        _ => 1.0,
    };

    let value = base * multiplier;
    if value > u64::MAX as f64 {
        return None;
    }
    Some(value.round() as u64)
}

/// 从一段文本中取出第一个捕获到的合法数字（应用区间约束）。
fn first_valid_number(regex: &Regex, text: &str, min: u64, max: u64) -> Option<(u64, String)> {
    for caps in regex.captures_iter(text) {
        // 捕获组 1 是数字本体，组 2 是可选后缀
        let Some(digits) = caps.get(1) else { continue };
        let suffix = caps.get(2).map(|m| m.as_str());
        let Some(value) = parse_number(digits.as_str(), suffix) else {
            continue;
        };
        if value >= min && value <= max {
            return Some((value, caps.get(0).map(|m| m.as_str()).unwrap_or("").to_string()));
        }
    }
    None
}

// ---- 主入口 ----

/// 从上游错误文本中解析上下文窗口与输出上限。
///
/// # 参数
/// - `text`: 上游返回的错误正文（错误 message / 整个 JSON 均可）
///
/// # 返回
/// 解析结果。调用方应检查 `is_empty()` 后决定是否采纳。
///
/// # 行为说明
/// 1. 先做「是否在抱怨参数过大」的前置检查，不满足直接返回空结果。
/// 2. 再依次用 OUTPUT_PATTERNS 匹配。取所有模式中的最大值，
///    因为错误信息里往往先出现请求值（999999）再出现约束值（16384），
///    取最大值反而更稳健地拿到那个「不超过」的约束。
/// 3. 最后用 CONTEXT_PATTERNS 匹配上下文窗口。
pub fn parse_limits(text: &str) -> LimitParseResult {
    if text.trim().is_empty() {
        return LimitParseResult {
            max_output_limit: None,
            context_limit: None,
            evidence: String::new(),
        };
    }

    // ---- 前置检查：确认这是一条「参数过大」的报错 ----
    if !TOO_LARGE_HINT.is_match(text) {
        return LimitParseResult {
            max_output_limit: None,
            context_limit: None,
            evidence: String::new(),
        };
    }

    let mut evidences: Vec<String> = Vec::new();

    // ---- 输出上限 ----
    // 遍历全部模式，收集全部候选值，取「不太可能是探测请求发送的值」中的最大者。
    // 探测请求发送的值通常 >= 999999，因此把 999999 本身排除掉。
    let mut output_candidates: Vec<(u64, String)> = Vec::new();
    for re in OUTPUT_PATTERNS.iter() {
        for caps in re.captures_iter(text) {
            let Some(digits) = caps.get(1) else { continue };
            let suffix = caps.get(2).map(|m| m.as_str());
            let Some(value) = parse_number(digits.as_str(), suffix) else {
                continue;
            };
            if value >= MIN_OUTPUT_LIMIT && value <= MAX_OUTPUT_LIMIT {
                let frag = caps.get(0).map(|m| m.as_str()).unwrap_or("").to_string();
                output_candidates.push((value, frag));
            }
        }
    }
    // 优先选择「明显不是探测请求送出的值」的候选：把 >= 900000 的当作回显请求值剔除。
    let filtered: Vec<(u64, String)> = output_candidates
        .iter()
        .filter(|(v, _)| *v < 900_000)
        .cloned()
        .collect();
    let output_pool = if filtered.is_empty() {
        &output_candidates
    } else {
        &filtered
    };
    let max_output_limit = output_pool.iter().map(|(v, _)| *v).max();
    if let Some(best) = output_pool.iter().filter(|(v, _)| Some(*v) == max_output_limit).next() {
        evidences.push(format!("输出上限依据：“{}”", best.1));
    }

    // ---- 上下文窗口 ----
    let mut context_limit: Option<u64> = None;
    for re in CONTEXT_PATTERNS.iter() {
        if let Some((value, frag)) = first_valid_number(re, text, MIN_CONTEXT_LIMIT, MAX_CONTEXT_LIMIT)
        {
            context_limit = Some(value);
            evidences.push(format!("上下文依据：“{}”", frag));
            break;
        }
    }

    // ---- 一致性修正：上下文不可能小于输出上限 ----
    // 若解析结果明显矛盾（例如上下文 4096 而输出 8192），保留上下文、丢弃输出，
    // 因为这种矛盾通常意味着输出上限抓错了数字。
    if let (Some(ctx), Some(out)) = (context_limit, max_output_limit) {
        if ctx < out {
            evidences.push("检测到上限矛盾（上下文 < 输出），已丢弃输出上限以免误导".to_string());
            return LimitParseResult {
                max_output_limit: None,
                context_limit: Some(ctx),
                evidence: evidences.join("；"),
            };
        }
    }

    LimitParseResult {
        max_output_limit,
        context_limit,
        evidence: evidences.join("；"),
    }
}

/// 便捷函数：只关心输出上限时使用。
pub fn parse_max_output(text: &str) -> Option<u64> {
    parse_limits(text).max_output_limit
}

/// 便捷函数：只关心上下文上限时使用。
pub fn parse_context(text: &str) -> Option<u64> {
    parse_limits(text).context_limit
}

// ---- 单元测试 ----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_style_error() {
        let text = "Invalid 'max_tokens': integer above the maximum value. \
                    Please reduce your prompt or set max_tokens to at most 16384.";
        let r = parse_limits(text);
        assert_eq!(r.max_output_limit, Some(16_384));
    }

    #[test]
    fn parses_deepseek_style_error() {
        let text = "max_tokens is too large: 999999. This model supports at most 8192 completion tokens.";
        let r = parse_limits(text);
        assert_eq!(r.max_output_limit, Some(8_192));
    }

    #[test]
    fn parses_context_length_error() {
        let text = "This model's maximum context length is 128000 tokens. \
                    However, your messages resulted in 130000 tokens.";
        let r = parse_limits(text);
        assert_eq!(r.context_limit, Some(128_000));
    }

    #[test]
    fn parses_vllm_style_error() {
        let text = "max_tokens 999999 is greater than the maximum model length 4096";
        let r = parse_limits(text);
        // vLLM 用 "maximum model length" 表述被请求的 max_tokens 的上界，
        // 因此该值同时被识别为输出上限与上下文上限候选。
        // 此处只断言输出上限 —— 这是调用方实际采用的那个。
        assert_eq!(r.max_output_limit, Some(4_096));
    }

    #[test]
    fn parses_chinese_provider_error() {
        let text = "请求参数错误：单次最大输出长度不能超过 8192，请调整 max_tokens 后重试。";
        let r = parse_limits(text);
        assert_eq!(r.max_output_limit, Some(8_192));
    }

    #[test]
    fn parses_k_suffix_notation() {
        let text = "maximum context length is 128k tokens";
        let r = parse_limits(text);
        assert_eq!(r.context_limit, Some(131_072));
    }

    #[test]
    fn parses_thousand_separators() {
        let text = "requested max_tokens exceeds the limit of 128,000 tokens";
        let r = parse_limits(text);
        assert_eq!(r.max_output_limit, Some(128_000));
    }

    #[test]
    fn rejects_unrelated_error_text() {
        // 一条和参数过大无关的报错，不应被解析出任何上限
        let text = "unsupported parameter: temperature. Please remove it.";
        let r = parse_limits(text);
        assert!(r.is_empty(), "无关文本不应产生上限结果：{r:?}");
    }

    #[test]
    fn rejects_absurd_numbers() {
        // 数字超出合理区间应被区间约束过滤掉
        let text = "max_tokens is too large, limit is 7 tokens";
        let r = parse_limits(text);
        assert_eq!(r.max_output_limit, None, "小于 256 的值应被丢弃");
    }

    #[test]
    fn resolves_contradiction_between_context_and_output() {
        let text = "max_tokens is too large; maximum context length is 4096 and \
                    set max_tokens to at most 8192";
        let r = parse_limits(text);
        // 上下文 < 输出属矛盾，应保留上下文、丢弃输出
        assert_eq!(r.context_limit, Some(4_096));
        assert_eq!(r.max_output_limit, None);
    }

    #[test]
    fn ignores_echoed_probe_value() {
        // 错误里既回显了请求值 999999 又给出了约束 16384，
        // 应取约束值而不是请求值。
        let text = "max_tokens 999999 is too large, maximum allowed is 16384";
        let r = parse_limits(text);
        assert_eq!(r.max_output_limit, Some(16_384));
    }

    #[test]
    fn empty_input_is_safe() {
        assert!(parse_limits("").is_empty());
        assert!(parse_limits("   ").is_empty());
    }
}
