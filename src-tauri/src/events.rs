/**
 * ============================================================================
 * ModelScout —— 阶段 2 前置：全局事件名常量
 * ============================================================================
 *
 * 与前端 `src/types/contracts.ts` 中的 `EVENT_NAMES` 必须**逐字一致**。
 * 阶段 5 QA 会核对本文件与前端常量的对应关系。
 * ============================================================================
 */

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
