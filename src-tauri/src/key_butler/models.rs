//! Key 管家数据契约：本模块唯一的对外契约定义。
//!
//! 与前端 `src/types/contracts.ts` 的「Key 管家」一节逐字段对应，
//! 字段名统一 camelCase（serde rename_all）。
//!
//! 独立成文件的原因（模块隔离原则）：Key 管家的契约与探测模块的契约
//! 放在一起会让两个模块的字段互相干扰（尤其 `ProbeErrorKind` 这类共用概念）；
//! `scripts/cross_check.py` 已扩展为同时核对本文件。
//!
//! 安全约定（本模块红线）：
//!   1. `KeyEntry` 绝不包含 Key 明文，只有 `masked_key`；
//!      明文只存在于 `vault` 的内存表中，且只在 `key_butler_reveal` 时按 id 取出；
//!   2. 任何结构体都不会被写入日志；错误信息一律经过 `error::redact_key` /
//!      `scrub_secrets` 处理；
//!   3. 列表展示的脱敏格式是 `sk-ab***cdef`（前 5 后 4），
//!      与错误信息脱敏（`*REDACTED*`）是两套规则，不得混用。


use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ---- 枚举 ----

/// 一条 Key 的状态（四态分类）。
///
/// 判定逻辑见 `classify.rs`：
///   - 401/403                              → `Invalid`
///   - 有效且余额 > 阈值                     → `Active`
///   - 有效且余额 ≤ 阈值（含余额为负）        → `Insufficient`
///   - 网络 / 429 / 5xx / 解析失败 / 余额未知 → `Error`
///
/// 注意：未检测过的 Key 也用 `Error` 承载（配合 `last_checked_at == None`
/// 区分），状态定为四态，不额外增加 `pending`；
/// 前端据此展示「未检测」，且不计入四态汇总。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyStatus {
    /// 可用：Key 有效且余额充足
    Active,
    /// 余额不足：Key 有效但余额低于阈值
    Insufficient,
    /// 已失效：401 / 403
    Invalid,
    /// 查询失败：网络错误 / 429 / 5xx / 解析失败，Key 有效性未知
    Error,
}

impl Default for KeyStatus {
    /// 默认「查询失败」—— 新导入、尚未检测的 Key 落到这里，
    /// 前端再用 `lastCheckedAt == null` 把它显示成「未检测」。
    ///
    /// 手写而不 derive：避免将来在枚举头部插入变体时默认值被静默改变
    /// （与 `ProbeStatus` / `ApiKeySource` 保持同一约定）。
    fn default() -> Self {
        KeyStatus::Error
    }
}

/// 余额接口的适配器类型。
///
/// 余额接口的现实约束：OpenAI 官方已下线公开余额接口、
/// Anthropic 无公开端点，因此必须按平台适配。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BalanceEndpointKind {
    /// 内置模板：one-api / new-api 系中转站
    /// （`/v1/dashboard/billing/subscription` + `/v1/dashboard/billing/usage`）
    ///
    /// 序列化值刻意用 `builtin-oneapi`（带连字符），与 TS 联合类型
    /// `'builtin-oneapi' | 'builtin-deepseek' | 'none' | 'custom'` 对齐 ——
    /// 单靠 rename_all = "camelCase" 会得到 `builtinOneapi`，不匹配。
    #[serde(rename = "builtin-oneapi")]
    BuiltinOneapi,
    /// 内置模板：DeepSeek 官方 `GET {host}/user/balance`
    /// （返回 `balance_infos[0].total_balance` 与 `currency`）
    #[serde(rename = "builtin-deepseek")]
    BuiltinDeepseek,
    /// 该平台没有公开余额接口（官方 OpenAI / Anthropic 默认此项）
    None,
    /// 用户自定义（见 `BalanceEndpoint`）
    Custom,
}

impl Default for BalanceEndpointKind {
    /// 默认「无公开余额接口」—— 宁可不查，也不去猜一个端点。
    fn default() -> Self {
        BalanceEndpointKind::None
    }
}

/// 自定义余额接口的 HTTP 方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
}

impl Default for HttpMethod {
    fn default() -> Self {
        HttpMethod::Get
    }
}

/// 批量检测中单条 Key 所处的阶段（用于进度事件）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckStage {
    /// 排队等待
    Pending,
    /// 正在做有效性检测（最小 chat 请求）
    Checking,
    /// 正在查询余额
    Balance,
    /// 已完成
    Done,
}

impl CheckStage {
    /// 阶段的中文显示名（与探测模块的 `ProbeStage::label` 风格一致）。
    pub fn label(&self) -> &'static str {
        match self {
            CheckStage::Pending => "排队等待",
            CheckStage::Checking => "检测有效性",
            CheckStage::Balance => "查询余额",
            CheckStage::Done => "完成",
        }
    }
}

// ---- 数据模型 ----

/// 自定义余额接口配置（`balanceEndpoint == "custom"` 时生效）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceEndpoint {
    /// URL 模板，支持 `{base}` 占位（`{base}` = 该 Key 的 Base URL）。
    /// 例：`{base}/v1/dashboard/billing/subscription`
    pub url_template: String,
    /// HTTP 方法
    #[serde(default)]
    pub method: HttpMethod,
    /// 额外请求头（例如某些中转站需要自定义鉴权头）
    #[serde(default)]
    pub extra_headers: BTreeMap<String, String>,
    /// 总额度取值路径（JSONPath 式，如 `hard_limit_usd`）
    #[serde(default)]
    pub total_path: Option<String>,
    /// 已用额度取值路径（如 `total_usage`）
    #[serde(default)]
    pub used_path: Option<String>,
    /// 直接余额取值路径（服务端直接返回余额字段时使用）
    #[serde(default)]
    pub balance_path: Option<String>,
    /// 单位除数：默认 1；one-api 的 `total_usage` 以美分计时为 100
    #[serde(default = "default_unit_divisor")]
    pub unit_divisor: f64,
}

/// `unit_divisor` 的 serde 默认值（1.0）。
fn default_unit_divisor() -> f64 {
    1.0
}

/// 一条被管理的 Key。
///
/// 本结构体不含 Key 明文：只有 `masked_key`。
/// 明文保存在 `vault` 的内存表里，仅在 `key_butler_reveal` 时按 id 取出。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyEntry {
    /// 唯一 id（uuid v4）
    pub id: String,
    /// 脱敏展示串，格式 `sk-ab***cdef`（前 5 后 4）
    pub masked_key: String,
    /// 平台标识：anthropic / openai / groq / google / xai / huggingface /
    /// replicate / unknown，或用户自定义前缀映射出来的值
    pub platform: String,
    /// 用户备注
    #[serde(default)]
    pub note: String,
    /// 导入时间（Unix 毫秒）
    pub created_at: u64,
    /// 最近一次检测完成时间（Unix 毫秒）；`None` = 从未检测
    pub last_checked_at: Option<u64>,
    /// 四态状态（未检测时为 `Error`，前端配合 `lastCheckedAt == null` 显示「未检测」）
    pub status: KeyStatus,
    /// Key 是否有效；`None` = 未能判定（查询失败）
    ///
    /// 用途：`status == Error` 但 `keyValid == Some(true)` 时，
    /// UI 显示「检测未完成（Key 有效）」—— 这是「部分成功」的表达方式。
    pub key_valid: Option<bool>,
    /// 余额数值；`None` = 未知
    pub balance: Option<f64>,
    /// 余额是否可信（`balance` 有值且解析成功）
    pub balance_known: bool,
    /// 币种（如 USD / CNY）；取不到为 `None`
    pub currency: Option<String>,
    /// 错误分类（复用探测模块的 9 类字符串值，如 `keyInvalid`）；
    /// 无错误为 `None`
    pub error_category: Option<String>,
    /// 该 Key 使用的余额接口适配器
    pub balance_endpoint: BalanceEndpointKind,
    /// 自定义余额接口配置（`balance_endpoint == Custom` 时生效）
    pub custom_endpoint: Option<BalanceEndpoint>,
    /// 该 Key 的 Base URL（有效性检测与余额查询都需要）。
    ///
    /// 数据模型必须有 Base URL 的原因：「最小 chat 请求」与
    /// `{base}` 占位都需要知道端点，而同一批 Key 可能来自不同中转站，
    /// 因此按 Key 记录。导入时按平台给默认值（见
    /// `platform_prefixes.json` 的 `defaultBaseUrl`），用户可逐条修改。
    pub base_url: Option<String>,
}

/// 一条余额快照。
///
/// 不含 Key 明文，只含 `key_id` 引用 + 数值，因此可明文落盘
/// （与加密的 Key 本体分离存储，见 vault 模块安全模型）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// 关联的 Key id
    pub key_id: String,
    /// 快照时间（Unix 毫秒）
    pub ts: u64,
    /// 当时的余额；`None` = 未知
    pub balance: Option<f64>,
    /// 当时的状态字符串（`KeyStatus` 的序列化值）
    pub status: String,
}

// ---- 命令入参 / 出参 ----

/// 批量导入时的平台判定方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyImportMode {
    /// 按 Key 形态自动识别平台（默认）
    Auto,
    /// 整批统一指定厂商（`KeyImportRequest.platform` 必填）
    Manual,
}

impl Default for KeyImportMode {
    /// 默认自动识别 —— 大多数情况下形态足够区分，不该逼用户先做选择。
    fn default() -> Self {
        KeyImportMode::Auto
    }
}

/// `key_butler_import` 的入参。
///
/// 手动模式（`mode == Manual`）下，`platform` 是必填的；`base_url` 与
/// `balance_endpoint` 是可选覆盖：
///   - 不传 / 传空 → 用该平台在规则表里的默认值（见 `platform::defaults_for`）
///   - 传了 → 覆盖默认值
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyImportRequest {
    /// 待导入的整段文本（提取规则在后端，前端不参与切分）
    pub text: String,
    /// 平台判定方式；不传 = `Auto`
    #[serde(default)]
    pub mode: KeyImportMode,
    /// 手动模式下的平台标识（如 `deepseek`）
    #[serde(default)]
    pub platform: Option<String>,
    /// 手动模式下的 Base URL 覆盖；空 = 用该平台默认值
    #[serde(default)]
    pub base_url: Option<String>,
    /// 手动模式下的余额适配器覆盖；不传 = 用该平台默认值
    #[serde(default)]
    pub balance_endpoint: Option<BalanceEndpointKind>,
}

/// `key_butler_import` 的返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyImportResult {
    /// 导入后的全量脱敏列表（省一次 key_butler_list 往返）
    pub entries: Vec<KeyEntry>,
    /// 本次新增条数
    pub added: usize,
    /// 因重复（同一明文已存在）被跳过的条数
    pub duplicates: usize,
    /// 文本里没能识别成 Key 的行数
    pub ignored: usize,
}

/// `key_butler_check` 的入参。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyCheckRequest {
    /// 要检测的 Key id；空数组 = 全部
    #[serde(default)]
    pub ids: Vec<String>,
    /// 并发数（1-10，后端 clamp）
    #[serde(default)]
    pub concurrency: Option<usize>,
    /// 单请求超时（秒，5-300，后端 clamp）
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// 余额不足阈值：余额 ≤ 该值判为 `insufficient`。默认 0
    #[serde(default)]
    pub low_balance_threshold: Option<f64>,
}

/// `key_butler_check` 的返回：整批检测的汇总。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyCheckSummary {
    /// 本次运行的唯一 id（Unix 毫秒字符串），前端据此丢弃过期事件
    pub run_id: String,
    /// 实际检测的条数
    pub checked: usize,
    /// 四态计数
    pub active: usize,
    pub insufficient: usize,
    pub invalid: usize,
    pub error: usize,
    /// 总耗时（毫秒）
    pub elapsed_ms: u64,
}

/// `key_butler_set_config` 的入参。
///
/// 三个字段都是「可选覆盖」语义：
/// - `None`（或前端不传）= 该项保持不变
/// - `Some(...)` = 覆盖为给定值
///
/// `base_url` 传空字符串表示清空（后端归一为 `None`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyConfigPatch {
    /// 新的 Base URL
    #[serde(default)]
    pub base_url: Option<String>,
    /// 新的余额接口适配器
    #[serde(default)]
    pub balance_endpoint: Option<BalanceEndpointKind>,
    /// 新的自定义余额接口配置
    #[serde(default)]
    pub custom_endpoint: Option<BalanceEndpoint>,
}

/// `key_butler_update_many` 的入参：一次改多条 Key。
///
/// 语义与 `KeyConfigPatch` 一致 —— `None` 表示该项不动。
/// 与单条版本的区别是多了 `platform` 与 `note`：
/// 批量编辑最常见的诉求就是「这一批其实都是 DeepSeek，平台识别错了」，
/// 只改平台标签而不动端点往往不够用，因此提供 `apply_platform_defaults`
/// 让「改平台」可以顺带套用该平台的默认 Base URL 与余额适配器。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyBatchPatch {
    /// 新的平台标识
    #[serde(default)]
    pub platform: Option<String>,
    /// 新的 Base URL；空字符串表示清空（后端归一为 `None`）
    #[serde(default)]
    pub base_url: Option<String>,
    /// 新的余额接口适配器
    #[serde(default)]
    pub balance_endpoint: Option<BalanceEndpointKind>,
    /// 新的备注
    #[serde(default)]
    pub note: Option<String>,
    /// 改平台时是否同时套用该平台在规则表里的默认 Base URL 与余额适配器。
    ///
    /// 默认 `false`：只改标签，避免「只想改个名字」却把用户手填的 Base URL 冲掉。
    /// 显式的 `base_url` / `balance_endpoint` 始终优先于本项。
    #[serde(default)]
    pub apply_platform_defaults: bool,
}

/// 本地保存（vault）的当前状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    /// 磁盘上是否已存在密文库（即此前启用过）
    pub enabled: bool,
    /// 当前会话是否已解锁（能读到明文表）
    pub unlocked: bool,
    /// 当前内存中的 Key 条数
    pub entry_count: usize,
    /// 是否使用「无密码保存」：数据密钥由 Windows DPAPI 绑定当前账户，
    /// 启动时自动解开。磁盘上依然是 AES-256-GCM 密文，不是明文。
    ///
    /// 注意：`enabled && !unlocked && systemKey` 表示「这份数据属于别的
    /// Windows 账户或另一台电脑」，前端要给出与「等待输入主密码」不同的引导。
    pub system_key: bool,
}

// ---- 事件 payload ----

/// 事件 `key-butler://progress` 的 payload。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyCheckProgress {
    /// 所属运行 id
    pub run_id: String,
    /// 当前处理的 Key id
    pub key_id: String,
    /// 当前处理的 Key 脱敏串（便于日志与界面直接展示，不含明文）
    pub masked_key: String,
    /// 当前阶段
    pub stage: CheckStage,
    /// 阶段的中文描述
    pub stage_label: String,
    /// 已完成条数
    pub completed: usize,
    /// 总条数
    pub total: usize,
    /// 整体完成百分比（0-100，后端算好）
    pub percent: u32,
}

/// 事件 `key-butler://entry-done` 的 payload：单条检测完成。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyCheckEntryDone {
    pub run_id: String,
    /// 该 Key 的最新完整状态
    pub entry: KeyEntry,
    pub completed: usize,
    pub total: usize,
}

/// 事件 `key-butler://finished` 的 payload：整批检测结束。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyCheckFinished {
    pub run_id: String,
    /// 终态：`done` | `cancelled` | `failed`
    pub status: String,
    pub checked: usize,
    pub total: usize,
    pub elapsed_ms: u64,
    /// 汇总（与 `key_butler_check` 的返回值一致，便于事件路径单独驱动 UI）
    pub summary: KeyCheckSummary,
}
