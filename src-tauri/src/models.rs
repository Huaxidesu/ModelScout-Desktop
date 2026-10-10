/**
 * ============================================================================
 * ModelScout —— 数据契约模块（Rust 侧「唯一事实来源」的镜像）
 * ============================================================================
 *
 * 【核心约定】
 * 本文件中的**每一个**对外结构体都必须带：
 *     #[derive(Debug, Clone, Serialize, Deserialize)]
 *     #[serde(rename_all = "camelCase")]
 *
 * 否则前端拿到的将是 snake_case 字段，与 `src/types/contracts.ts` 不匹配。
 * 这是 Tauri v2 最常见的陷阱之一（任务书第七节第 3 条）。
 *
 * 字段顺序与 `contracts.ts` 中的接口保持一致，便于阶段 5 逐一交叉核对。
 * ============================================================================
 */

use serde::{Deserialize, Serialize};

use crate::error::ProbeError;

/* ---------------------------------------------------------------------------
 * 一、基础枚举
 * ------------------------------------------------------------------------- */

/// 能力探测的三态结果。
///
/// 与前端 `TriState` 对应。用三态而非 bool 的原因：
/// 探测失败（例如上游 500）时**无法判定**支持与否，
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
    /// **接口在 `/models` 中直接声明**（最可信：服务端自己说的）
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

/* ---------------------------------------------------------------------------
 * 二、命令入参 / 出参结构体
 * ------------------------------------------------------------------------- */

/// `start_probe` 的入参。
///
/// 安全提示：`api_key` 字段是唯一持有 Key 明文的载体。
/// 它**绝不**被序列化回前端（本结构体虽 derive 了 Serialize 以便日志调试，
/// 但全项目代码中没有任何一处把 ProbeConfig 直接 emit 出去），
/// 也**绝不**被打印。参见 `error::redact_key`。
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
}

impl ProbeConfig {
    /// 规范化：裁剪空白、补全协议前缀、clamp 数值范围。
    ///
    /// 这是**唯一**允许修正用户输入的地方，后续所有代码都可以假定
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
        }
    }

    /// 校验必填项。返回中文错误信息（面向用户）。
    pub fn validate(&self) -> Result<(), String> {
        if self.base_url.trim().is_empty() {
            return Err("Base URL 不能为空。".to_string());
        }
        if self.api_key.trim().is_empty() {
            return Err("API Key 不能为空。".to_string());
        }
        // 规范化之后的 URL 至少要有 "https://x" 这样 9 个字符
        if self.base_url.len() < 9 || !self.base_url.contains("://") {
            return Err(format!("Base URL 格式不合法：{}", self.base_url));
        }
        Ok(())
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
    /// **为什么需要**：实测发现部分网关的 `/models` 声明可能与官方文档不符
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
/// - `Some(..)` = 接口**已明确声明**该能力 → 直接采信，**无需再发请求探测**
///   （省额度、更快、且比探测更准确）
/// - `None`     = 接口**未声明** → 回退到真实请求探测（标准 OpenAI 接口走这条路）
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
    /// 是否为**推理模型**（`supported_parameters` 中存在 `reasoning`）
    ///
    /// 注意：这比「响应里出现 reasoning_content」更可靠 —— 实测发现
    /// 部分网关对**所有**模型都会返回 `reasoning_content` 字段（可能为空），
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

/* ---------------------------------------------------------------------------
 * 三、事件 payload 结构体
 * ------------------------------------------------------------------------- */

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

/* ---------------------------------------------------------------------------
 * 四、快照结构体（供 get_probe_state 使用）
 * ------------------------------------------------------------------------- */

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

/* ---------------------------------------------------------------------------
 * 五、配置导出
 * ------------------------------------------------------------------------- */

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

/* ---------------------------------------------------------------------------
 * 单元测试
 * ------------------------------------------------------------------------- */

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
        };
        assert_eq!(cfg.normalized().base_url, "http://localhost:11434/v1");
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
