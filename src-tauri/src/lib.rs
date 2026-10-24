//! ModelScout 库入口：Tauri v2 应用装配。
//!
//! - 声明全部业务模块，构造并运行 Tauri Builder；
//! - 注册插件（剪贴板、打开外部链接），托管全局状态
//!   （探测的 `AppState` 与 Key 管家的 `KeyButlerState`）；
//! - setup 阶段初始化模型参数字典、Key 管家前缀表与落盘路径；
//! - 注册全部命令（探测 10 个 + Key 管家 16 个）。
//!
//! 按 Tauri v2 结构约定，全部逻辑置于 lib crate（`modelscout_lib`），
//! `main.rs` 仅做一行调用：移动端目标可复用同一 lib，单元测试可直接
//! 针对 lib 运行，无需启动完整应用。

// 全部业务模块
pub mod commands; // Tauri 命令层
pub mod credential; // 从 Windows 凭据管理器读取 API Key（跨平台签名，非 Windows 返回错误）
pub mod dictionary; // 本地模型参数字典
pub mod error; // 错误分类与脱敏
pub mod events; // 事件名常量
pub mod key_butler; // Key 管家（Beta）：批量 Key 管理，独立模块，不改动探测逻辑
pub mod max_tokens; // 超大 max_tokens 报错解析
pub mod models; // 数据契约（serde camelCase）
pub mod probe; // 探测引擎

// 启动前检查 WebView2 运行时。仅在 Windows 上有意义：
// macOS / Linux 使用系统自带的 WebKit，没有这个组件。
#[cfg(windows)]
pub mod webview2;

use commands::AppState;

// 注意：`tauri::Manager` 必须显式引入：
//    `app.path()` / `app.manage()` / `app.emit()` 等方法都由该 trait 提供。
//    Tauri v2 不在 prelude 中自动导出它（v1 会自动导出），
//    漏掉这一行会报 "no method named `path` found for `&mut tauri::App`"。
use tauri::Manager;

/// 隐藏调试构建附带的控制台窗口。
///
/// 采用运行期隐藏而非再叠加一个 `windows_subsystem` 属性：该属性是编译期的，
/// 一旦开启，debug 构建将无法获取 stdout / stderr，而排查启动问题（字典加载、
/// 前缀表、WebView2 检查）恰恰依赖这些输出。运行期调用 `ShowWindow(SW_HIDE)`
/// 仅隐藏窗口，界面整洁，日志仍可被重定向捕获。
///
/// release 构建已在 `main.rs` 用编译期属性去掉了控制台，此时
/// `GetConsoleWindow()` 返回空句柄，本函数自动跳过。
#[cfg(windows)]
fn hide_console_window() {
    use windows_sys::Win32::System::Console::GetConsoleWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};

    // SAFETY: 两个调用都没有内存副作用。无控制台时 GetConsoleWindow 返回空句柄，
    // 此时不会调用 ShowWindow。
    unsafe {
        let handle = GetConsoleWindow();
        if !handle.is_null() {
            ShowWindow(handle, SW_HIDE);
        }
    }
}

/// 应用入口。
///
/// 由 `main.rs` 调用；移动端入口（若将来添加）也会调用它。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ---- 隐藏控制台窗口（Windows）----
    // 必须放在最前面：一旦开始建窗口，再藏控制台会让用户看到一次闪烁。
    #[cfg(windows)]
    hide_console_window();

    // ---- 启动前检查（preflight，Windows）----
    // 缺 WebView2 运行库时，Tauri 会在创建窗口那一步直接失败，用户看到的
    // 现象是「双击没反应」。此处提前拦截，给出中文说明并提供一键打开官网。
    // 必须在构造 Builder 之前执行：一旦开始创建窗口，就只剩 Tauri 自带的
    // 英文提示。
    #[cfg(windows)]
    webview2::ensure_runtime();

    tauri::Builder::default()
        // ---- 插件注册 ----
        // 剪贴板：WebView 中 navigator.clipboard 的可用性在不同平台与权限下
        // 不一致，统一改用官方插件。
        .plugin(tauri_plugin_clipboard_manager::init())
        // 打开外部链接（GitHub 仓库、官方文档）
        .plugin(tauri_plugin_opener::init())
        // ---- 全局状态 ----
        .manage(AppState::new())
        // Key 管家自带一份状态（模块隔离，与探测模块的 AppState 相互独立）
        .manage(key_butler::vault::KeyButlerState::new())
        // ---- setup 钩子 ----
        .setup(|app| {
            // 初始化模型参数字典：
            // 优先从打包资源中读取 resources/model_params.json，
            // 失败则回退到编译期内嵌字典（保证功能不降级）。
            match app.path().resource_dir() {
                Ok(dir) => match dictionary::init_from_resource_dir(&dir) {
                    Ok(Some(version)) => {
                        println!(
                            "[ModelScout] 字典加载完成：{} 条，版本 {}，资源目录 {}",
                            dictionary::entry_count(),
                            version,
                            dir.display()
                        );
                    }
                    Ok(None) => {
                        println!(
                            "[ModelScout] 未找到外部字典文件，使用内嵌字典（{} 条）",
                            dictionary::entry_count()
                        );
                    }
                    Err(err) => {
                        eprintln!("[ModelScout] 字典初始化异常：{err}，使用内嵌字典");
                    }
                },
                Err(err) => {
                    eprintln!(
                        "[ModelScout] 无法解析资源目录（{err}），使用内嵌字典（{} 条）",
                        dictionary::entry_count()
                    );
                }
            }

            // ---- Key 管家：前缀表 + 落盘路径 ----
            // 前缀表：用户 app data 目录的同名文件优先，其次是打包资源，
            // 最后是编译期内嵌（`platform::init` 内部完成三级回退）。
            let resource_dir = app.path().resource_dir().ok();
            let data_dir = app.path().app_data_dir().ok();
            let resource_ref = resource_dir
                .as_deref()
                .unwrap_or_else(|| std::path::Path::new("."));
            let rule_count = key_butler::platform::init(resource_ref, data_dir.as_deref());
            println!("[ModelScout] Key 管家平台规则表：{rule_count} 条");

            // 落盘路径：密文库与余额快照都放在 app data 目录下
            if let Some(dir) = data_dir.as_deref() {
                app.state::<key_butler::vault::KeyButlerState>()
                    .configure(dir);
            } else {
                eprintln!("[ModelScout] 无法解析应用数据目录，Key 管家的加密保存不可用");
            }

            Ok(())
        })
        // ---- 命令注册 ----
        // 命令名必须与前端 src/api/commands.ts 中 COMMAND_NAMES 常量逐字一致。
        .invoke_handler(tauri::generate_handler![
            commands::list_models,        // 1. 拉取模型列表
            commands::ping_connection,    // 2. 连通性预检
            commands::start_probe,        // 3. 启动完整探测
            commands::stop_probe,         // 4. 中止探测
            commands::get_probe_state,    // 5. 查询实时快照
            commands::save_config_file,   // 6. 导出配置到磁盘
            commands::get_dictionary_info, // 7. 字典自检（辅助）
            commands::inspect_tool_config, // 8. 一键填写：读取目标工具配置
            commands::write_tool_config,   // 9. 一键填写：确认后写入目标配置
            commands::scan_tool_configs,   // 10. 配置管理：扫描全部工具配置
            // ---- Key 管家（Beta）：16 个命令 ----
            // 函数名必须与前端 COMMAND_NAMES.KEY_BUTLER_* 的字符串值逐字一致
            key_butler::commands::key_butler_import,
            key_butler::commands::key_butler_check,
            key_butler::commands::key_butler_check_cancel,
            key_butler::commands::key_butler_list,
            key_butler::commands::key_butler_reveal,
            key_butler::commands::key_butler_delete,
            key_butler::commands::key_butler_update_note,
            key_butler::commands::key_butler_set_config,
            key_butler::commands::key_butler_update_many,
            key_butler::commands::key_butler_vault_enable,
            key_butler::commands::key_butler_vault_enable_system,
            key_butler::commands::key_butler_vault_unlock,
            key_butler::commands::key_butler_vault_disable,
            key_butler::commands::key_butler_vault_status,
            key_butler::commands::key_butler_snapshots,
            key_butler::commands::key_butler_record_probe
        ])
        .run(tauri::generate_context!())
        .expect("ModelScout 启动失败：Tauri 应用初始化异常");
}
