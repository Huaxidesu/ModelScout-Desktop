//! 本地模型参数字典：上下文窗口 / 单次输出上限的第一优先级数据源。
//!
//! - 探测上限时先用模型 ID 在本地字典做模糊匹配（如 `gpt-4o`），命中即直接
//!   返回、不发起网络请求（快且省额度）；未命中才走「超大 max_tokens 触发
//!   报错」的探测路径（见 `max_tokens.rs`）；
//! - 加载顺序：优先读取 `resources/model_params.json`，读取成功则整体替换
//!   内嵌数据（用户可直接编辑该文件扩充字典，无需重新编译）；文件缺失或
//!   损坏时回退到内嵌字典，功能不降级；
//! - `matchPatterns` 按数组顺序取第一个命中项：无通配符为完全相等、
//!   `xxx*` 为前缀匹配、`*xxx*` 为包含匹配；同为通配符时更具体的模式
//!   必须排在更宽泛的模式之前（如 `gpt-4o-mini*` 须排在 `gpt-4o*` 之前，
//!   否则 mini 会被 4o 截获）。

use std::path::Path;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

// ---- 数据结构 ----

/// 词典中的一个条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictEntry {
    /// 匹配模式列表（按顺序，先具体后宽泛）
    pub match_patterns: Vec<String>,
    /// 上下文窗口上限（token）
    pub context_limit: u64,
    /// 单次最大输出上限（token）
    pub max_output_limit: u64,
    /// 备注（会在前端展示来源说明）
    #[serde(default)]
    pub note: String,
}

/// 词典文件整体结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictFile {
    /// 词典版本号（形如 2026.10.09），仅用于展示
    #[serde(default)]
    pub dictionary_version: String,
    /// 条目列表
    pub entries: Vec<DictEntry>,
}

/// 一次字典匹配的命中结果。
#[derive(Debug, Clone)]
pub struct DictHit {
    /// 上下文上限
    pub context_limit: u64,
    /// 输出上限
    pub max_output_limit: u64,
    /// 命中的模式（用于向前端说明来源）
    pub matched_pattern: String,
    /// 备注
    pub note: String,
}

// ---- 匹配算法 ----

/// 判断单个模式是否命中给定的模型 ID。
///
/// 三种语法：
/// - `abc*`  前缀匹配（大小写不敏感）
/// - `*abc*` 包含匹配（大小写不敏感）
/// - `abc`   完全相等（大小写不敏感）
///
/// 注意：模型 ID 大小写不敏感是刻意的 —— 各家服务商在
/// `GPT-4o` / `gpt-4o` 上并不统一。
pub fn pattern_matches(pattern: &str, model_id: &str) -> bool {
    let p = pattern.trim().to_lowercase();
    let m = model_id.trim().to_lowercase();

    if p.is_empty() || m.is_empty() {
        return false;
    }

    let starts_wild = p.starts_with('*');
    let ends_wild = p.ends_with('*');

    match (starts_wild, ends_wild) {
        // `*abc*` → 包含
        (true, true) => {
            let core = p.trim_matches('*');
            core.is_empty() || m.contains(core)
        }
        // `abc*` → 前缀
        (false, true) => {
            let core = p.trim_end_matches('*');
            m.starts_with(core)
        }
        // `*abc` → 后缀（虽然当前字典未使用，但支持它以免行为意外）
        (true, false) => {
            let core = p.trim_start_matches('*');
            m.ends_with(core)
        }
        // 无通配符 → 完全相等
        (false, false) => m == p,
    }
}

/// 在一组条目中查找命中项。
///
/// 采用双轮扫描，优先级明确：
/// - 第一轮只接受「无通配符的精确串」命中，精确串永远最可信；
/// - 第二轮接受通配符命中，按 entries 顺序取第一个。
///
/// 这样即使具体模式没有排在前面，`gpt-4o-mini` 也依然会被精确条目
/// `gpt-4o-mini` 正确截获，不会被宽泛的 `gpt-4o*` 抢先匹配，
/// 消除了对字典书写顺序的脆弱依赖。
pub fn lookup_in(entries: &[DictEntry], model_id: &str) -> Option<DictHit> {
    // ---- 第一轮：仅精确串 ----
    for entry in entries {
        for pattern in &entry.match_patterns {
            let is_exact = !pattern.contains('*');
            if is_exact && pattern_matches(pattern, model_id) {
                return Some(DictHit {
                    context_limit: entry.context_limit,
                    max_output_limit: entry.max_output_limit,
                    matched_pattern: pattern.clone(),
                    note: entry.note.clone(),
                });
            }
        }
    }

    // ---- 第二轮：通配符，按条目与模式的书写顺序 ----
    for entry in entries {
        for pattern in &entry.match_patterns {
            if pattern.contains('*') && pattern_matches(pattern, model_id) {
                return Some(DictHit {
                    context_limit: entry.context_limit,
                    max_output_limit: entry.max_output_limit,
                    matched_pattern: pattern.clone(),
                    note: entry.note.clone(),
                });
            }
        }
    }

    None
}

// ---- 内嵌回退字典 ----

/// 生成内嵌的回退字典条目。
///
/// 与 `resources/model_params.json` 内容保持一致，
/// 保证资源文件缺失或损坏时仍有可用字典。
fn builtin_entries() -> Vec<DictEntry> {
    /// 小工具：少写点样板代码
    fn e(patterns: &[&str], ctx: u64, out: u64, note: &str) -> DictEntry {
        DictEntry {
            match_patterns: patterns.iter().map(|s| s.to_string()).collect(),
            context_limit: ctx,
            max_output_limit: out,
            note: note.to_string(),
        }
    }

    vec![
        // ---------- OpenAI ----------
        e(&["gpt-4o-mini*"], 128_000, 16_384, "OpenAI GPT-4o mini"),
        e(
            &["gpt-4o*"],
            128_000,
            16_384,
            "OpenAI GPT-4o（含快照版本）",
        ),
        e(&["chatgpt-4o-latest"], 128_000, 16_384, "OpenAI ChatGPT-4o 滚动最新"),
        e(
            &["gpt-4-turbo*", "gpt-4-0125*", "gpt-4-1106*"],
            128_000,
            4_096,
            "OpenAI GPT-4 Turbo 系列",
        ),
        e(
            &["gpt-4-32k*", "gpt-4-32k-0613"],
            32_768,
            4_096,
            "OpenAI GPT-4 32K",
        ),
        e(
            &["gpt-4-0613", "gpt-4"],
            8_192,
            4_096,
            "OpenAI 初代 GPT-4",
        ),
        e(&["gpt-3.5-turbo-16k*"], 16_385, 4_096, "OpenAI GPT-3.5 Turbo 16K"),
        e(
            &["gpt-3.5-turbo*", "gpt-3.5*"],
            16_385,
            4_096,
            "OpenAI GPT-3.5 Turbo 系列",
        ),
        e(
            &["gpt-5-mini*", "gpt-5-nano*", "gpt-5*"],
            400_000,
            128_000,
            "OpenAI GPT-5 系列",
        ),
        e(&["o1-mini*"], 128_000, 65_536, "OpenAI o1-mini 推理模型"),
        e(&["o1-preview*"], 128_000, 32_768, "OpenAI o1-preview 推理模型"),
        e(&["o4-mini*", "o3*", "o1*"], 200_000, 100_000, "OpenAI o 系列推理模型"),
        // ---------- Anthropic ----------
        e(
            &["claude-3-5-sonnet*", "claude-3.5-sonnet*"],
            200_000,
            8_192,
            "Anthropic Claude 3.5 Sonnet",
        ),
        e(
            &["claude-3-5-haiku*", "claude-3.5-haiku*"],
            200_000,
            8_192,
            "Anthropic Claude 3.5 Haiku",
        ),
        e(
            &["claude-3-7-sonnet*", "claude-3.7-sonnet*"],
            200_000,
            64_000,
            "Anthropic Claude 3.7 Sonnet",
        ),
        e(
            &["claude-sonnet-4*", "claude-4-sonnet*"],
            200_000,
            64_000,
            "Anthropic Claude Sonnet 4",
        ),
        e(
            &["claude-opus-4*", "claude-4-opus*"],
            200_000,
            32_000,
            "Anthropic Claude Opus 4",
        ),
        e(&["claude-3-opus*"], 200_000, 4_096, "Anthropic Claude 3 Opus"),
        e(&["claude-3-sonnet*"], 200_000, 4_096, "Anthropic Claude 3 Sonnet"),
        e(&["claude-3-haiku*"], 200_000, 4_096, "Anthropic Claude 3 Haiku"),
        e(&["claude-2*"], 100_000, 4_096, "Anthropic Claude 2"),
        // ---------- DeepSeek ----------
        e(
            &["deepseek-reasoner*", "deepseek-r1*"],
            65_536,
            8_192,
            "DeepSeek R1 推理模型",
        ),
        e(
            &["deepseek-chat*", "deepseek-v3*"],
            65_536,
            8_192,
            "DeepSeek V3 系列",
        ),
        e(&["deepseek-coder*"], 65_536, 8_192, "DeepSeek Coder"),
        // ---------- Google ----------
        e(&["gemini-2.5-pro*"], 1_048_576, 65_536, "Google Gemini 2.5 Pro"),
        e(&["gemini-2.5-flash*"], 1_048_576, 65_536, "Google Gemini 2.5 Flash"),
        e(&["gemini-2.0-flash*"], 1_048_576, 8_192, "Google Gemini 2.0 Flash"),
        e(&["gemini-1.5-pro*"], 2_097_152, 8_192, "Google Gemini 1.5 Pro"),
        e(
            &["gemini-1.5-flash*"],
            1_048_576,
            8_192,
            "Google Gemini 1.5 Flash",
        ),
        // ---------- 阿里通义千问 ----------
        e(&["qwen3*"], 131_072, 32_768, "阿里通义千问 Qwen3"),
        e(&["qwen2.5-coder*"], 131_072, 8_192, "阿里通义千问 Qwen2.5 Coder"),
        e(
            &["qwen2.5*", "qwen-max*", "qwen-plus*", "qwen-turbo*"],
            131_072,
            8_192,
            "阿里通义千问 Qwen2.5 / 商业版",
        ),
        e(&["qwen2*", "qwen1.5*"], 32_768, 8_192, "阿里通义千问早期系列"),
        // ---------- 智谱 ----------
        e(&["glm-4.6*", "glm-4.5*"], 131_072, 16_384, "智谱 GLM-4.5 / 4.6"),
        e(&["glm-4*"], 131_072, 4_096, "智谱 GLM-4 系列"),
        // ---------- 月之暗面 ----------
        e(
            &["moonshot-v1-128k*", "kimi-k2*"],
            131_072,
            16_384,
            "月之暗面 Kimi K2 / Moonshot 128K",
        ),
        e(&["moonshot-v1-32k*"], 32_768, 8_192, "月之暗面 Moonshot 32K"),
        e(&["moonshot-v1-8k*"], 8_192, 4_096, "月之暗面 Moonshot 8K"),
        // ---------- Meta ----------
        e(
            &["llama-3.3-70b*", "llama3.3*"],
            131_072,
            8_192,
            "Meta Llama 3.3 70B",
        ),
        e(
            &["llama-3.1*", "llama3.1*"],
            131_072,
            8_192,
            "Meta Llama 3.1 系列",
        ),
        e(&["llama-3*", "llama3*"], 8_192, 4_096, "Meta Llama 3 系列"),
        // ---------- Mistral ----------
        e(&["mistral-large*"], 131_072, 8_192, "Mistral Large"),
        e(
            &["mistral-small*", "mistral-nemo*"],
            131_072,
            8_192,
            "Mistral Small / Nemo",
        ),
        e(
            &["mixtral-8x7b*", "mixtral-8x22b*"],
            65_536,
            8_192,
            "Mistral Mixtral MoE",
        ),
        // ---------- xAI ----------
        e(&["grok-4*"], 256_000, 32_768, "xAI Grok 4"),
        e(&["grok-3*"], 131_072, 16_384, "xAI Grok 3"),
        e(&["grok-2*", "grok-beta*"], 131_072, 8_192, "xAI Grok 2"),
        // ---------- 国内厂商 ----------
        e(
            &["doubao-1.5*", "doubao-pro*", "doubao-lite*"],
            262_144,
            12_288,
            "字节豆包 Doubao 系列",
        ),
        e(
            &["ernie-4.5*", "ernie-4*"],
            131_072,
            8_192,
            "百度文心一言 ERNIE 4 系列",
        ),
        e(&["step-3*", "step-2*"], 65_536, 8_192, "阶跃星辰 Step 系列"),
        e(
            &["minimax-text-01*", "abab6.5*", "abab7*"],
            1_000_000,
            8_192,
            "MiniMax abab / text-01",
        ),
        e(
            &["hunyuan-turbos*", "hunyuan-large*", "hunyuan-standard*"],
            131_072,
            8_192,
            "腾讯混元 Hunyuan 系列",
        ),
        e(
            &["spark-max*", "spark-pro*", "spark-lite*", "generalv3*"],
            32_768,
            8_192,
            "科大讯飞星火 Spark 系列",
        ),
        e(
            &["command-r-plus*", "command-r*"],
            131_072,
            4_096,
            "Cohere Command R 系列",
        ),
        e(&["phi-4*"], 16_384, 4_096, "Microsoft Phi-4"),
        e(&["phi-3*"], 131_072, 4_096, "Microsoft Phi-3"),
        e(
            &["yi-large*", "yi-medium*", "yi-lightning*"],
            32_768,
            4_096,
            "零一万物 Yi 系列",
        ),
        e(&["baichuan*"], 32_768, 4_096, "百川智能 Baichuan 系列"),
    ]
}

// ---- 全局字典实例 ----

/// 全局字典条目缓存。
///
/// 首次访问时初始化：先尝试读取外部资源文件，失败则用内嵌数据。
/// 用 `OnceLock<Vec<DictEntry>>` 而非 `Lazy` 的原因是 ——
/// 需要在处理读取失败时打印一条日志，而日志需要 Tauri AppHandle，/// 因此把「初始化」做成显式函数由 lib.rs 在 setup 阶段调用，
/// 结果存入这个 OnceLock。
static DICT_ENTRIES: OnceLock<Vec<DictEntry>> = OnceLock::new();

/// 获取字典条目。若尚未初始化则先用内嵌数据初始化。
pub fn entries() -> &'static [DictEntry] {
    DICT_ENTRIES.get_or_init(builtin_entries)
}

/// 显式初始化字典：尝试从外部资源文件加载并覆盖内嵌数据。
///
/// # 参数
/// - `resource_dir`: Tauri 解析出的资源目录（`app.path().resource_dir()`）
///
/// # 返回
/// - `Ok(Some(version))`: 成功从外部文件加载，返回其版本号
/// - `Ok(None)`: 外部文件不存在或解析失败，已保留内嵌回退数据
/// - `Err(msg)`: 仅在 `set` 失败（已被初始化过）时返回，实际不会发生
pub fn init_from_resource_dir(resource_dir: &Path) -> Result<Option<String>, String> {
    let candidates = [
        resource_dir.join("resources").join("model_params.json"),
        resource_dir.join("model_params.json"),
        // dev 模式下 resource_dir 指向 target/debug，回退到源码目录
        resource_dir
            .join("..")
            .join("..")
            .join("resources")
            .join("model_params.json"),
    ];

    for path in candidates.iter() {
        if !path.exists() {
            continue;
        }
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<DictFile>(&text) {
                Ok(file) if !file.entries.is_empty() => {
                    let version = file.dictionary_version.clone();
                    let count = file.entries.len();
                    // 忽略返回值：若已被初始化（例如测试中重复调用）属正常情况
                    let _ = DICT_ENTRIES.set(file.entries);
                    eprintln!(
                        "[ModelScout] 已从 {} 加载模型字典，共 {} 条，版本 {}",
                        path.display(),
                        count,
                        if version.is_empty() { "未知" } else { &version }
                    );
                    return Ok(Some(version));
                }
                Ok(_) => {
                    eprintln!(
                        "[ModelScout] 字典文件 {} 为空，回退内嵌字典",
                        path.display()
                    );
                }
                Err(err) => {
                    eprintln!(
                        "[ModelScout] 字典文件 {} 解析失败（{}），回退内嵌字典",
                        path.display(),
                        err
                    );
                }
            },
            Err(err) => {
                eprintln!(
                    "[ModelScout] 字典文件 {} 读取失败（{}）",
                    path.display(),
                    err
                );
            }
        }
    }

    // 回退：确保 OnceLock 已被内嵌数据填充
    let _ = DICT_ENTRIES.get_or_init(builtin_entries);
    Ok(None)
}

/// 对外主入口：按模型 ID 查字典。
///
/// 这是 `probe.rs` 唯一需要调用的函数。
pub fn lookup(model_id: &str) -> Option<DictHit> {
    lookup_in(entries(), model_id)
}

/// 字典条目总数（用于启动日志与交付报告自检）。
pub fn entry_count() -> usize {
    entries().len()
}

// ---- 单元测试 ----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_wins_over_wildcard_even_when_listed_later() {
        // 构造一个「宽泛规则恰好排在前面」的不利场景，
        // 验证双轮扫描策略仍能让精确串胜出。
        let entries = vec![
            DictEntry {
                match_patterns: vec!["gpt-4o*".to_string()],
                context_limit: 128_000,
                max_output_limit: 16_384,
                note: "宽泛规则（故意排前面）".to_string(),
            },
            DictEntry {
                match_patterns: vec!["gpt-4o-mini".to_string()],
                context_limit: 111,
                max_output_limit: 222,
                note: "精确规则".to_string(),
            },
        ];
        let hit = lookup_in(&entries, "gpt-4o-mini").expect("应命中");
        assert_eq!(hit.context_limit, 111, "精确串必须胜过排在更前的通配符");
        assert_eq!(hit.matched_pattern, "gpt-4o-mini");
    }

    #[test]
    fn builtin_dictionary_hits_common_models() {
        let gpt4o = lookup("gpt-4o-2024-08-06").expect("gpt-4o 系列应命中");
        assert_eq!(gpt4o.context_limit, 128_000);
        assert_eq!(gpt4o.max_output_limit, 16_384);

        let mini = lookup("gpt-4o-mini").expect("gpt-4o-mini 应独立命中");
        assert_eq!(mini.context_limit, 128_000);
        assert_eq!(mini.matched_pattern, "gpt-4o-mini*");

        let ds = lookup("deepseek-reasoner").expect("deepseek-reasoner 应命中");
        assert_eq!(ds.context_limit, 65_536);

        let claude = lookup("claude-3-5-sonnet-20241022").expect("claude 应命中");
        assert_eq!(claude.context_limit, 200_000);

        let unknown = lookup("some-random-local-model-7b");
        assert!(unknown.is_none(), "未收录模型应返回 None 以触发上限探测");
    }

    #[test]
    fn pattern_matching_semantics() {
        assert!(pattern_matches("gpt-4o", "GPT-4O"), "应大小写不敏感");
        assert!(pattern_matches("gpt-4o*", "gpt-4o-mini"));
        assert!(!pattern_matches("gpt-4o*", "gpt-3.5-turbo"));
        assert!(pattern_matches("*sonnet*", "claude-3-5-sonnet-20241022"));
        assert!(!pattern_matches("", "gpt-4o"), "空模式不应匹配任何东西");
        assert!(pattern_matches("gpt-4o-0613", "gpt-4o-0613"));
        assert!(!pattern_matches("gpt-4o-0613", "gpt-4o-0613-extra"));
    }

    #[test]
    fn embedded_dictionary_is_not_empty() {
        assert!(
            entry_count() >= 40,
            "内嵌字典条目数异常偏少：{}",
            entry_count()
        );
    }
}
