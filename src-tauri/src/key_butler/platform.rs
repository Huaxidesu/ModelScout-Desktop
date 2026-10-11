/**
 * ============================================================================
 * ModelScout —— Key 管家：平台前缀识别
 * ============================================================================
 *
 * 【职责】
 * 按 API Key 的**前缀**判断它属于哪个平台，并给出该平台默认的
 * 余额接口适配器与 Base URL。
 *
 * 【匹配语义：长前缀优先】
 * `sk-ant-`（anthropic）必须优先于 `sk-`（通用中转）命中。
 * 实现上在加载时按前缀长度**降序**排序，取第一个命中的即可 ——
 * 这样新增规则时不必关心顺序，数据文件里怎么写都不会出错。
 *
 * 【数据来源与加载顺序】
 *   1. 用户文件 `{app_data_dir}/platform_prefixes.json`（优先，可覆盖扩充）
 *   2. 打包资源 `resources/platform_prefixes.json`
 *   3. 编译期内嵌（`include_str!` 同一份文件），保证资源丢失时不降级
 * ============================================================================
 */

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// 编译期内嵌的兜底规则（与打包资源是同一份文件）。
const BUILTIN_JSON: &str = include_str!("../../resources/platform_prefixes.json");

/// 未命中任何前缀时的平台标识。
pub const UNKNOWN_PLATFORM: &str = "unknown";

/// 未命中任何前缀时的余额适配器（通用中转多为 one-api / new-api 系）。
pub const UNKNOWN_ENDPOINT: &str = "builtin-oneapi";

/// 一条前缀规则。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformRule {
    /// 前缀（大小写敏感 —— `AIza` 与 `aiza` 是不同的东西）
    pub prefix: String,
    /// 平台标识
    pub platform: String,
    /// 余额接口适配器：`builtin-oneapi` / `none` / `custom`
    #[serde(default = "default_endpoint")]
    pub balance_endpoint: String,
    /// 该平台的默认 Base URL（导入时填入 `KeyEntry.base_url`，用户可改）
    #[serde(default)]
    pub default_base_url: Option<String>,
    /// 备注
    #[serde(default)]
    pub note: String,
}

/// `balance_endpoint` 的 serde 默认值。
fn default_endpoint() -> String {
    "none".to_string()
}

/// 全局规则表（进程内只加载一次）。
static RULES: OnceLock<Vec<PlatformRule>> = OnceLock::new();

/* ---------------------------------------------------------------------------
 * 加载
 * ------------------------------------------------------------------------- */

/// 解析一段 JSON 文本为规则列表。
fn parse_rules(text: &str) -> Result<Vec<PlatformRule>, String> {
    serde_json::from_str::<Vec<PlatformRule>>(text).map_err(|err| err.to_string())
}

/// 按前缀长度降序排序，实现「长前缀优先」。
fn sort_rules(rules: &mut [PlatformRule]) {
    rules.sort_by(|a, b| b.prefix.len().cmp(&a.prefix.len()));
}

/// 读取并排序一个规则文件。
fn load_file(path: &Path) -> Option<Vec<PlatformRule>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut rules = match parse_rules(&text) {
        Ok(rules) if !rules.is_empty() => rules,
        Ok(_) => {
            eprintln!("[ModelScout] 前缀表 {} 为空，跳过", path.display());
            return None;
        }
        Err(err) => {
            eprintln!("[ModelScout] 前缀表 {} 解析失败（{err}），跳过", path.display());
            return None;
        }
    };
    sort_rules(&mut rules);
    Some(rules)
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
                    "[ModelScout] 已从 {} 加载平台前缀表，共 {} 条",
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
        "[ModelScout] 未找到外部平台前缀表，使用内嵌规则（{} 条）",
        rules_len()
    );
    rules_len()
}

/// 当前生效的规则表；若尚未初始化则用内嵌规则兜底。
fn rules() -> &'static Vec<PlatformRule> {
    RULES.get_or_init(|| {
        let mut rules = parse_rules(BUILTIN_JSON).unwrap_or_default();
        sort_rules(&mut rules);
        rules
    })
}

/// 当前规则条数。
pub fn rules_len() -> usize {
    rules().len()
}

/// 当前生效的全部前缀（按长度降序，空前缀已剔除）。
///
/// 供 `commands::extract_keys` 构造「已知前缀」提取正则使用 ——
/// 这样前缀表新增一条规则，批量粘贴就自动认识这种 Key，
/// 不需要再去改正则字面量。
pub fn prefixes() -> Vec<String> {
    rules()
        .iter()
        .map(|rule| rule.prefix.clone())
        .filter(|prefix| !prefix.is_empty())
        .collect()
}

/* ---------------------------------------------------------------------------
 * 识别
 * ------------------------------------------------------------------------- */

/// 按前缀识别一个 Key 所属的平台。
///
/// 未命中时返回一条合成的「通用中转」规则（platform = `unknown`，
/// balanceEndpoint = `builtin-oneapi`），而不是 `None` ——
/// 调用方永远不需要处理「没有规则」这种情况。
pub fn identify(key: &str) -> PlatformRule {
    let trimmed = key.trim();
    for rule in rules() {
        if !rule.prefix.is_empty() && trimmed.starts_with(&rule.prefix) {
            return rule.clone();
        }
    }
    PlatformRule {
        prefix: String::new(),
        platform: UNKNOWN_PLATFORM.to_string(),
        balance_endpoint: UNKNOWN_ENDPOINT.to_string(),
        default_base_url: None,
        note: "未命中已知前缀，按通用 OpenAI 兼容端点处理".to_string(),
    }
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
    }

    #[test]
    fn generic_sk_falls_back_to_unknown_with_oneapi_endpoint() {
        let rule = identify("sk-abcdefghijklmnopqrst");
        assert_eq!(rule.platform, "unknown");
        assert_eq!(rule.balance_endpoint, "builtin-oneapi");
    }

    #[test]
    fn unmatched_key_is_unknown_not_none() {
        let rule = identify("totally-unknown-key-value");
        assert_eq!(rule.platform, UNKNOWN_PLATFORM);
        assert_eq!(rule.balance_endpoint, UNKNOWN_ENDPOINT);
    }

    #[test]
    fn rules_are_sorted_by_prefix_length_desc() {
        let rules = rules();
        assert!(!rules.is_empty());
        for pair in rules.windows(2) {
            assert!(
                pair[0].prefix.len() >= pair[1].prefix.len(),
                "规则未按前缀长度降序：{} 在 {} 之前",
                pair[0].prefix,
                pair[1].prefix
            );
        }
    }

    #[test]
    fn builtin_json_is_parseable_and_non_empty() {
        let parsed = parse_rules(BUILTIN_JSON).expect("内嵌前缀表必须可解析");
        assert!(parsed.len() >= 9);
    }
}
