/**
 * ============================================================================
 * ModelScout —— Key 管家：平台识别规则表
 * ============================================================================
 *
 * 【职责】
 * 按 API Key 的**形态**判断它属于哪个平台，并给出该平台默认的
 * 余额接口适配器与 Base URL。
 *
 * 【两种匹配方式】
 *   1. `prefix`（默认）：Key 以该前缀开头即命中。
 *   2. `pattern`：可选的正则。**存在时以正则为准**，用于「前缀与别人撞车、
 *      只能靠形态区分」的平台。
 *      典型例子：DeepSeek 官方 Key 是 `sk-` + 32 位十六进制，而 `sk-` 同时是
 *      大量 OpenAI 兼容中转站的通用前缀 —— 只靠前缀必然把 DeepSeek 误判成
 *      「通用中转」。加上 `^sk-[0-9a-fA-F]{32}$` 之后就能精确命中。
 *
 * 【匹配语义：正则优先、长前缀优先】
 *   - 带 `pattern` 的规则先于纯前缀规则尝试（它更具体）；
 *   - 同为纯前缀时按前缀长度**降序**，保证 `sk-ant-` 先于 `sk-` 命中。
 *   排序在加载时一次性完成，因此数据文件里怎么写都不会出错。
 *
 * 【autoDetect = false：只给「手动指定厂商」用的目录条目】
 *   有些平台的 Key 形态与通用中转完全一样（Moonshot / 硅基流动都是 `sk-`），
 *   自动识别永远分不出来。这类厂商在本表里登记 `autoDetect: false`：
 *     - **不参与**自动识别（`identify` 跳过）
 *     - **不参与**批量粘贴的提取前缀派生（`prefixes` 跳过）
 *     - 但可以通过 `defaults_for()` 取到它的默认 Base URL / 余额适配器，
 *       供「导入时手动指定厂商」与「批量改平台」使用。
 *
 * 【数据来源与加载顺序】
 *   1. 用户文件 `{app_data_dir}/platform_prefixes.json`（优先，可覆盖扩充）
 *   2. 打包资源 `resources/platform_prefixes.json`
 *   3. 编译期内嵌（`include_str!` 同一份文件），保证资源丢失时不降级
 * ============================================================================
 */

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// 编译期内嵌的兜底规则（与打包资源是同一份文件）。
const BUILTIN_JSON: &str = include_str!("../../resources/platform_prefixes.json");

/// 未命中任何规则时的平台标识。
pub const UNKNOWN_PLATFORM: &str = "unknown";

/// 未命中任何规则时的余额适配器（通用中转多为 one-api / new-api 系）。
pub const UNKNOWN_ENDPOINT: &str = "builtin-oneapi";

/// 一条平台规则。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformRule {
    /// 前缀（大小写敏感 —— `AIza` 与 `aiza` 是不同的东西）。
    ///
    /// `autoDetect = false` 的目录条目可以省略（省略即空串，不参与匹配）。
    #[serde(default)]
    pub prefix: String,
    /// 可选正则：存在时**以正则为准**（见文件头说明）。
    #[serde(default)]
    pub pattern: Option<String>,
    /// 是否参与「按 Key 形态自动识别」。目录条目（仅供手动指定厂商）设 false。
    #[serde(default = "default_auto_detect")]
    pub auto_detect: bool,
    /// 平台标识
    pub platform: String,
    /// 余额接口适配器：`builtin-oneapi` / `builtin-deepseek` / `none` / `custom`
    #[serde(default = "default_endpoint")]
    pub balance_endpoint: String,
    /// 该平台的默认 Base URL（导入时填入 `KeyEntry.base_url`，用户可改）
    #[serde(default)]
    pub default_base_url: Option<String>,
    /// 备注
    #[serde(default)]
    pub note: String,
}

/// `auto_detect` 的 serde 默认值：默认参与自动识别。
fn default_auto_detect() -> bool {
    true
}

/// `balance_endpoint` 的 serde 默认值。
fn default_endpoint() -> String {
    "none".to_string()
}

/// 编译后的规则：正则只编译一次，识别时不再重复构造。
struct CompiledRule {
    rule: PlatformRule,
    regex: Option<Regex>,
}

/// 全局规则表（进程内只加载一次）。
static RULES: OnceLock<Vec<CompiledRule>> = OnceLock::new();

/* ---------------------------------------------------------------------------
 * 加载
 * ------------------------------------------------------------------------- */

/// 解析一段 JSON 文本为规则列表。
fn parse_rules(text: &str) -> Result<Vec<PlatformRule>, String> {
    serde_json::from_str::<Vec<PlatformRule>>(text).map_err(|err| err.to_string())
}

/// 按「正则优先、长前缀优先」排序，实现「更具体的规则先命中」。
fn sort_rules(rules: &mut [CompiledRule]) {
    rules.sort_by(|a, b| {
        // 1) 带正则的规则更具体，排在纯前缀规则之前
        let by_pattern = b.rule.pattern.is_some().cmp(&a.rule.pattern.is_some());
        if by_pattern != std::cmp::Ordering::Equal {
            return by_pattern;
        }
        // 2) 同为纯前缀时，长前缀优先
        b.rule.prefix.len().cmp(&a.rule.prefix.len())
    });
}

/// 把原始规则编译成可用的规则表。
fn compile(rules: Vec<PlatformRule>) -> Vec<CompiledRule> {
    let mut compiled: Vec<CompiledRule> = rules
        .into_iter()
        .map(|rule| {
            let regex = match rule.pattern.as_deref() {
                Some(pattern) => match Regex::new(pattern) {
                    Ok(regex) => Some(regex),
                    Err(err) => {
                        // 正则写错时降级为前缀匹配，绝不因此让整个规则表失效
                        eprintln!(
                            "[ModelScout] 平台规则 {} 的正则无法编译（{err}），降级为前缀匹配",
                            rule.platform
                        );
                        None
                    }
                },
                None => None,
            };
            CompiledRule { rule, regex }
        })
        .collect();
    sort_rules(&mut compiled);
    compiled
}

/// 读取并编译一个规则文件。
fn load_file(path: &Path) -> Option<Vec<CompiledRule>> {
    let text = std::fs::read_to_string(path).ok()?;
    let rules = match parse_rules(&text) {
        Ok(rules) if !rules.is_empty() => rules,
        Ok(_) => {
            eprintln!("[ModelScout] 规则表 {} 为空，跳过", path.display());
            return None;
        }
        Err(err) => {
            eprintln!("[ModelScout] 规则表 {} 解析失败（{err}），跳过", path.display());
            return None;
        }
    };
    Some(compile(rules))
}

/// 初始化规则表。
///
/// # 参数
/// - `resource_dir`: Tauri 资源目录
/// - `app_data_dir`: 应用数据目录（用户可在此放同名文件覆盖扩充）
///
/// # 返回
/// 实际生效的规则条数。本函数**不会失败** —— 任何异常都回退到内嵌规则。
pub fn init(resource_dir: &Path, app_data_dir: Option<&Path>) -> usize {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // 1) 用户文件优先
    if let Some(dir) = app_data_dir {
        candidates.push(dir.join("platform_prefixes.json"));
    }

    // 2) 打包资源（含 dev 模式的源码目录回退，与 dictionary.rs 同一思路）
    candidates.push(resource_dir.join("resources").join("platform_prefixes.json"));
    candidates.push(resource_dir.join("platform_prefixes.json"));
    candidates.push(
        resource_dir
            .join("..")
            .join("..")
            .join("resources")
            .join("platform_prefixes.json"),
    );

    for path in candidates {
        if !path.exists() {
            continue;
        }
        if let Some(rules) = load_file(&path) {
            let count = rules.len();
            if RULES.set(rules).is_ok() {
                eprintln!(
                    "[ModelScout] 已从 {} 加载平台规则表，共 {} 条",
                    path.display(),
                    count
                );
                return count;
            }
            // 已被初始化（例如测试中重复调用）：直接返回当前条数
            return rules_len();
        }
    }

    eprintln!(
        "[ModelScout] 未找到外部平台规则表，使用内嵌规则（{} 条）",
        rules_len()
    );
    rules_len()
}

/// 当前生效的规则表；若尚未初始化则用内嵌规则兜底。
fn rules() -> &'static Vec<CompiledRule> {
    RULES.get_or_init(|| {
        let rules = parse_rules(BUILTIN_JSON).unwrap_or_default();
        compile(rules)
    })
}

/// 当前规则条数。
pub fn rules_len() -> usize {
    rules().len()
}

/// 参与自动识别的全部前缀（按长度降序，已去重、已剔除空串）。
///
/// 供 `commands::extract_keys` 构造「已知前缀」提取正则使用 ——
/// 这样规则表新增一条前缀，批量粘贴就自动认识这种 Key，
/// 不需要再去改正则字面量。
///
/// 只取 `autoDetect != false` 的条目：目录条目（Moonshot 之类）与通用
/// 中转站前缀相同，把它们塞进提取正则没有任何增益，反而误导读者。
pub fn prefixes() -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for compiled in rules() {
        let rule = &compiled.rule;
        if !rule.auto_detect || rule.prefix.is_empty() {
            continue;
        }
        if !result.iter().any(|existing| existing == &rule.prefix) {
            result.push(rule.prefix.clone());
        }
    }
    result
}

/* ---------------------------------------------------------------------------
 * 识别
 * ------------------------------------------------------------------------- */

/// 未命中任何规则时的合成规则（platform = `unknown`）。
fn unknown_rule() -> PlatformRule {
    PlatformRule {
        prefix: String::new(),
        pattern: None,
        auto_detect: true,
        platform: UNKNOWN_PLATFORM.to_string(),
        balance_endpoint: UNKNOWN_ENDPOINT.to_string(),
        default_base_url: None,
        note: "未命中已知规则，按通用 OpenAI 兼容端点处理".to_string(),
    }
}

/// 按形态识别一个 Key 所属的平台。
///
/// 未命中时返回一条合成的「通用中转」规则（platform = `unknown`，
/// balanceEndpoint = `builtin-oneapi`），而不是 `None` ——
/// 调用方永远不需要处理「没有规则」这种情况。
pub fn identify(key: &str) -> PlatformRule {
    let trimmed = key.trim();
    for compiled in rules() {
        let rule = &compiled.rule;
        if !rule.auto_detect {
            continue;
        }
        let matched = match &compiled.regex {
            Some(regex) => regex.is_match(trimmed),
            None => !rule.prefix.is_empty() && trimmed.starts_with(&rule.prefix),
        };
        if matched {
            return rule.clone();
        }
    }
    unknown_rule()
}

/// 按平台标识取该平台的默认配置：`(默认 Base URL, 余额适配器字符串)`。
///
/// 供「导入时手动指定厂商」与「批量改平台并套用默认端点」使用 ——
/// 命中同名的第一条规则即可（同一平台的多条规则默认配置一致）。
pub fn defaults_for(platform: &str) -> Option<(Option<String>, String)> {
    let target = platform.trim();
    if target.is_empty() {
        return None;
    }
    if target == UNKNOWN_PLATFORM {
        return Some((None, UNKNOWN_ENDPOINT.to_string()));
    }
    rules()
        .iter()
        .find(|compiled| compiled.rule.platform == target)
        .map(|compiled| {
            (
                compiled.rule.default_base_url.clone(),
                compiled.rule.balance_endpoint.clone(),
            )
        })
}

/* ---------------------------------------------------------------------------
 * 单元测试
 * ------------------------------------------------------------------------- */

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_prefix_wins() {
        // sk-ant- 必须先于 sk- 命中，否则 Anthropic 会被误判成通用中转
        assert_eq!(identify("sk-ant-api03-abcdefghijklmn").platform, "anthropic");
        assert_eq!(identify("sk-proj-abcdefghijklmn").platform, "openai");
        assert_eq!(identify("sk-svcacct-abcdefghijklmn").platform, "openai");
        assert_eq!(identify("gsk_abcdefghijklmn").platform, "groq");
        assert_eq!(identify("AIzaSyAbcdefghijklmn").platform, "google");
        assert_eq!(identify("xai-abcdefghijklmn").platform, "xai");
        assert_eq!(identify("hf_abcdefghijklmn").platform, "huggingface");
        assert_eq!(identify("r8_abcdefghijklmn").platform, "replicate");
        assert_eq!(identify("sk-or-v1-abcdefghijklmn").platform, "openrouter");
    }

    /// 回归：用户实测反馈 —— DeepSeek 官方 Key 曾被识别成「通用中转」。
    ///
    /// DeepSeek 的 Key 是 `sk-` + 32 位十六进制，与通用 `sk-` 兜底规则前缀完全相同，
    /// 只有靠 `pattern` 才能区分开。
    #[test]
    fn deepseek_key_is_not_swallowed_by_generic_sk_rule() {
        for key in [
            "sk-1d84f27e44de4a669ea2d345b7b3ae76",
            "sk-004572706d26498b86d58a8d9ebbdafe",
            "sk-9f8d9887cf0344fa99c983412a036539",
        ] {
            let rule = identify(key);
            assert_eq!(rule.platform, "deepseek", "实际：{rule:?}");
            assert_eq!(rule.balance_endpoint, "builtin-deepseek");
            assert_eq!(
                rule.default_base_url.as_deref(),
                Some("https://api.deepseek.com/v1")
            );
        }
    }

    /// 形态不像 DeepSeek 的 `sk-` Key 仍然落到通用中转，不能被正则误伤。
    #[test]
    fn generic_sk_key_still_falls_back_to_unknown() {
        // 长度不对
        assert_eq!(identify("sk-abcdefghijklmnopqrst").platform, "unknown");
        // 32 位但含非十六进制字符
        assert_eq!(
            identify("sk-zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").platform,
            "unknown"
        );
        // 大写十六进制也算 DeepSeek（同一形态）
        assert_eq!(
            identify("sk-ABCDEF0123456789ABCDEF0123456789").platform,
            "deepseek"
        );
    }

    #[test]
    fn unmatched_key_is_unknown_not_none() {
        let rule = identify("totally-unknown-key-value");
        assert_eq!(rule.platform, UNKNOWN_PLATFORM);
        assert_eq!(rule.balance_endpoint, UNKNOWN_ENDPOINT);
    }

    /// `autoDetect = false` 的目录条目不得参与自动识别。
    #[test]
    fn catalog_only_platforms_never_match_automatically() {
        // moonshot 的 Key 也是 sk- 开头，自动识别必须给「通用中转」而不是 moonshot
        let rule = identify("sk-moonshot-abcdefghijklmn");
        assert_eq!(rule.platform, "unknown");
    }

    /// 目录条目要能通过 `defaults_for` 取到默认配置（手动指定厂商用）。
    #[test]
    fn defaults_for_returns_catalog_defaults() {
        let (base, endpoint) = defaults_for("moonshot").expect("moonshot 应在目录里");
        assert_eq!(base.as_deref(), Some("https://api.moonshot.cn/v1"));
        assert_eq!(endpoint, "none");

        let (base, endpoint) = defaults_for("deepseek").expect("deepseek 应在目录里");
        assert_eq!(base.as_deref(), Some("https://api.deepseek.com/v1"));
        assert_eq!(endpoint, "builtin-deepseek");

        // 兜底平台：无 Base URL，走 one-api 适配器
        let (base, endpoint) = defaults_for("unknown").expect("unknown 必须有兜底");
        assert_eq!(base, None);
        assert_eq!(endpoint, UNKNOWN_ENDPOINT);

        assert!(defaults_for("不存在的平台").is_none());
        assert!(defaults_for("").is_none());
    }

    #[test]
    fn prefixes_exclude_catalog_only_entries_and_duplicates() {
        let prefixes = prefixes();
        assert!(prefixes.contains(&"sk-ant-".to_string()));
        assert!(prefixes.contains(&"sk-".to_string()));
        // sk- 在表里出现多次（deepseek / 兜底），派生结果里只能有一条
        assert_eq!(prefixes.iter().filter(|p| *p == "sk-").count(), 1);
        assert!(!prefixes.iter().any(|p| p.is_empty()));
    }

    #[test]
    fn rules_are_sorted_pattern_first_then_prefix_length_desc() {
        let rules = rules();
        assert!(!rules.is_empty());

        // 带正则的规则必须全部排在纯前缀规则之前
        let last_pattern = rules.iter().rposition(|c| c.rule.pattern.is_some());
        let first_plain = rules.iter().position(|c| c.rule.pattern.is_none());
        if let (Some(last_pattern), Some(first_plain)) = (last_pattern, first_plain) {
            assert!(
                last_pattern < first_plain,
                "带正则的规则必须排在纯前缀规则之前"
            );
        }

        // 同为纯前缀时按长度降序
        let plain: Vec<&CompiledRule> = rules
            .iter()
            .filter(|c| c.rule.pattern.is_none())
            .collect();
        for pair in plain.windows(2) {
            assert!(
                pair[0].rule.prefix.len() >= pair[1].rule.prefix.len(),
                "规则未按前缀长度降序：{} 在 {} 之前",
                pair[0].rule.prefix,
                pair[1].rule.prefix
            );
        }
    }

    #[test]
    fn builtin_json_is_parseable_and_non_empty() {
        let parsed = parse_rules(BUILTIN_JSON).expect("内嵌规则表必须可解析");
        assert!(parsed.len() >= 9);
        // 目录条目允许省略 prefix，解析时补空串
        let catalog: Vec<&PlatformRule> = parsed.iter().filter(|r| !r.auto_detect).collect();
        assert!(!catalog.is_empty(), "应当至少有一条「仅手动」目录条目");
        assert!(catalog.iter().all(|r| r.prefix.is_empty()));
    }

    #[test]
    fn bad_regex_degrades_to_prefix_matching_instead_of_panicking() {
        let rules = vec![PlatformRule {
            prefix: "abc-".to_string(),
            pattern: Some("(".to_string()), // 非法正则
            auto_detect: true,
            platform: "broken".to_string(),
            balance_endpoint: "none".to_string(),
            default_base_url: None,
            note: String::new(),
        }];
        let compiled = compile(rules);
        assert_eq!(compiled.len(), 1);
        assert!(compiled[0].regex.is_none(), "非法正则应被丢弃而不是 panic");
    }
}
