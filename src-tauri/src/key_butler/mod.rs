//! Key 管家（Beta）：批量管理大模型 API Key。
//!
//! 功能：前缀识别平台、有效性检测、余额查询、四态分类、
//! 可选加密持久化、余额历史快照。
//!
//! 子模块：
//!   - `models`    数据契约（与前端 contracts.ts 一一对应）
//!   - `platform`  平台规则表（正则优先、长前缀优先，用户可覆盖扩充）
//!   - `balance`   余额查询适配器（builtin-oneapi / builtin-deepseek / none / custom）
//!   - `classify`  四态分类判定
//!   - `vault`     Key 内存表 + 可选加密持久化（argon2id 主密码 或 DPAPI 系统密钥）
//!   - `dpapi`     Windows DPAPI 封装（供「无密码保存」包裹数据密钥）
//!   - `snapshots` 余额历史快照
//!   - `commands`  `key_butler_*` 命令层（在 lib.rs 的 generate_handler! 注册）
//!
//! 模块隔离约定：本模块不得修改探测模块（`probe.rs` / `dictionary.rs` /
//! `max_tokens.rs`）的任何业务逻辑；可复用的只有 `error.rs` 的错误分类与脱敏工具。

pub mod balance;
pub mod classify;
pub mod commands;
pub mod dpapi;
pub mod models;
pub mod platform;
pub mod snapshots;
pub mod vault;
