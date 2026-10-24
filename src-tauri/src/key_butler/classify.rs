//! Key 管家：四态分类。
//!
//! 把「Key 是否有效」「余额是否已知 / 是多少」「阈值」三组事实，
//! 归约成一个四态结果。纯函数，不碰网络、不碰文件，可穷举边界测试。
//!
//! 判定表：
//! | Key 有效 | 该平台有余额接口 | 余额已知 | 余额 vs 阈值 | 结果 |
//! |---|---|---|---|---|
//! | false | — | — | — | `Invalid` |
//! | true | 否 | — | — | `Active`（拿不到余额不代表 Key 不可用） |
//! | true | 是 | 是 | > 阈值 | `Active` |
//! | true | 是 | 是 | ≤ 阈值 | `Insufficient` |
//! | true | 是 | 否 | — | `Error`（Key 有效但余额查询失败） |
//! | 未知(null) | — | — | — | `Error` |
//!
//! 「平台没有余额接口」不算失败：OpenAI 官方已下线公开余额接口、
//! Anthropic 没有公开端点，这是预期内的情况而非查询出错；
//! 若判为 `Error`，用户界面会满屏「查询失败」，掩盖真正需要修复的问题。
//! 因此引入 `balance_applicable` 区分「查了但失败」与「本来就查不到」。

use crate::key_butler::models::KeyStatus;

/// 四态分类。
///
/// # 参数
/// - `key_valid`: `Some(true)` 有效 / `Some(false)` 失效 / `None` 未能判定
/// - `balance_applicable`: 该平台是否本应有余额接口
///   （`none` 适配器为 false；`builtin-oneapi` / `custom` 为 true）
/// - `balance_known`: 余额是否可信（解析成功）
/// - `balance`: 余额数值（`balance_known == false` 时忽略）
/// - `threshold`: 余额不足阈值（余额 ≤ 该值判为不足）
pub fn classify(
    key_valid: Option<bool>,
    balance_applicable: bool,
    balance_known: bool,
    balance: Option<f64>,
    threshold: f64,
) -> KeyStatus {
    match key_valid {
        // 明确失效：401 / 403
        Some(false) => KeyStatus::Invalid,
        Some(true) => {
            // 该平台没有公开余额接口：能确认 Key 有效即可用
            if !balance_applicable {
                return KeyStatus::Active;
            }
            match (balance_known, balance) {
                (true, Some(value)) if value > threshold => KeyStatus::Active,
                (true, Some(_)) => KeyStatus::Insufficient,
                // 余额应有却拿不到：视为查询失败，不谎报为可用
                _ => KeyStatus::Error,
            }
        }
        // 有效性未能判定（网络 / 429 / 5xx / 解析失败）
        None => KeyStatus::Error,
    }
}

// ---- 单元测试 ----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_key_always_wins() {
        assert_eq!(
            classify(Some(false), true, true, Some(999.0), 0.0),
            KeyStatus::Invalid
        );
        assert_eq!(
            classify(Some(false), false, false, None, 0.0),
            KeyStatus::Invalid
        );
    }

    #[test]
    fn balance_exactly_at_threshold_is_insufficient() {
        // 边界：用 `<=` 而不是 `<`
        assert_eq!(
            classify(Some(true), true, true, Some(10.0), 10.0),
            KeyStatus::Insufficient
        );
        assert_eq!(
            classify(Some(true), true, true, Some(10.01), 10.0),
            KeyStatus::Active
        );
    }

    #[test]
    fn zero_or_negative_balance_is_insufficient() {
        assert_eq!(
            classify(Some(true), true, true, Some(0.0), 0.0),
            KeyStatus::Insufficient
        );
        assert_eq!(
            classify(Some(true), true, true, Some(-3.5), 0.0),
            KeyStatus::Insufficient
        );
    }

    #[test]
    fn platform_without_balance_api_is_active_when_key_valid() {
        // OpenAI / Anthropic 这类没有公开余额接口的平台，不应显示「查询失败」
        assert_eq!(
            classify(Some(true), false, false, None, 0.0),
            KeyStatus::Active
        );
    }

    #[test]
    fn unknown_balance_with_applicable_endpoint_is_error() {
        // 「Key 有效但余额查询失败」→ Error，由 key_valid 字段在 UI 上区分
        assert_eq!(classify(Some(true), true, false, None, 0.0), KeyStatus::Error);
        assert_eq!(classify(Some(true), true, true, None, 0.0), KeyStatus::Error);
    }

    #[test]
    fn undetermined_validity_is_error() {
        assert_eq!(classify(None, true, false, None, 0.0), KeyStatus::Error);
        assert_eq!(classify(None, false, false, None, 0.0), KeyStatus::Error);
        assert_eq!(classify(None, true, true, Some(100.0), 0.0), KeyStatus::Error);
    }
}
