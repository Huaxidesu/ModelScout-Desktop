// release 构建时隐藏 Windows 上的额外控制台窗口。
// debug 构建保留控制台，方便查看我们打印的启动日志与脱敏后的配置摘要。
//
// ⚠️ 这个 inner attribute（#![...]）必须是本文件的**第一项**：
//    Rust 规定 inner attribute 之前不得出现任何文档注释（/// 或 /** */），
//    否则会报 "an inner attribute is not permitted following an outer doc comment"。
//    因此本文件的说明性文字全部使用普通注释 `//` 而非 doc comment。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// ============================================================================
// ModelScout —— 可执行入口
// ============================================================================
//
// 按照 Tauri v2 官方推荐的做法，可执行文件只保留最薄的一层：
// 隐藏 Windows 控制台窗口（release 下），并调用 lib 的 run()。
//
// 全部业务逻辑都在 `modelscout_lib` 中，main.rs 不承载任何逻辑，
// 这样单元测试可以直接针对 lib 运行。
// ============================================================================

fn main() {
    modelscout_lib::run()
}
