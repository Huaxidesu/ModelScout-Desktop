// release 构建时隐藏 Windows 上的额外控制台窗口；
// debug 构建保留控制台，便于查看启动日志与脱敏后的配置摘要。
//
// 该 inner attribute（#![...]）必须是本文件的第一项：Rust 规定 inner
// attribute 之前不得出现任何文档注释（/// 或 /** */），否则编译报错
// "an inner attribute is not permitted following an outer doc comment"。
// 因此本文件的说明性文字全部使用普通注释 `//` 而非文档注释。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// ---- 可执行入口 ----
// 按 Tauri v2 的推荐结构，可执行文件只负责隐藏 Windows 控制台窗口
// （release 下）并调用 lib 的 `run()`。全部业务逻辑都在 `modelscout_lib` 中，
// 单元测试可直接针对 lib 运行，无需启动完整应用。

fn main() {
    modelscout_lib::run()
}
