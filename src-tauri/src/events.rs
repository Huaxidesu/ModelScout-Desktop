//! 全局事件名常量。
//!
//! 与前端 `src/types/contracts.ts` 中的 `EVENT_NAMES` 逐字一致。

/// 进度更新事件（逐阶段推送）
pub const EVENT_PROGRESS: &str = "probe://progress";

/// 日志事件（补全运行期提示，让界面不空转）
pub const EVENT_LOG: &str = "probe://log";

/// 单个模型探测完成事件
pub const EVENT_MODEL_DONE: &str = "probe://model-done";

/// 整轮探测结束事件（正常完成 / 被中止）
pub const EVENT_FINISHED: &str = "probe://finished";

/// 致命错误事件（导致整轮任务终止）
pub const EVENT_ERROR: &str = "probe://error";

// ---- Key 管家（Beta） ----
// 命名沿用 `probe://` 的「命名空间://事件」风格，前缀为 `key-butler://`，
// 与前端 `EVENT_NAMES.KEY_BUTLER_*` 逐字一致。

/// 批量检测进度（逐 key、逐阶段）
pub const EVENT_KEY_BUTLER_PROGRESS: &str = "key-butler://progress";

/// 单条 Key 检测完成
pub const EVENT_KEY_BUTLER_ENTRY_DONE: &str = "key-butler://entry-done";

/// 整批检测结束（done / cancelled / failed）
pub const EVENT_KEY_BUTLER_FINISHED: &str = "key-butler://finished";
