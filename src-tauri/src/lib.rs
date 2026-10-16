/**
 * ============================================================================
 * ModelScout —— 库入口（Tauri v2 应用装配）
 * ============================================================================
 *
 * 【职责】
 *   1. 声明全部模块
 *   2. 构造并运行 Tauri Builder
 *   3. 注册插件（剪贴板、打开外部链接）
 *   4. 托管全局状态（AppState）
 *   5. 在 setup 阶段初始化模型参数字典
 *   6. 注册全部 6 个命令（generate_handler!）
 *
 * 【Tauri v2 结构约定】
 * 官方推荐把全部逻辑放进 lib crate（`modelscout_lib`），
 * `main.rs` 只做一行调用。这样做的好处：
 *   - 移动端目标（Android/iOS）可以复用同一个 lib
 *   - 单元测试可以直接针对 lib 运行，无需启动完整应用
 * ============================================================================
 */

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

// 启动前检查 WebView2 运行时。只在 Windows 上有意义 ——
// macOS / Linux 用的是系统自带的 WebKit，没有这个组件。
#[cfg(windows)]
pub mod webview2;

use commands::AppState;

// 注意：`tauri::Manager` 必须显式引入：
//    `app.path()` / `app.manage()` / `app.emit()` 等方法都由该 trait 提供。
//    Tauri v2 不在 prelude 中自动导出它（v1 会自动导出），
//    漏掉这一行会报 "no method named `path` found for `&mut tauri::App`"。
use tauri::Manager;

/// 隐藏调试构建下附带出来的控制台窗口。
///
/// 【为什么用运行期隐藏，而不是再加一个 `windows_subsystem` 属性】
/// `#![windows_subsystem = "windows"]` 是编译期的，一旦开启，debug 构建就再也
/// 拿不到 stdout / stderr —— 而排查启动问题（字典加载、前缀表、WebView2 检查）
/// 恰恰最需要这些输出。运行期调用 `ShowWindow(SW_HIDE)` 只把窗口藏起来：
/// 界面干净了，日志仍可被重定向捕获。
///
/// release 构建已经在 `main.rs` 用编译期属性去掉了控制台，
/// 此时 `GetConsoleWindow()` 返回空句柄，本函数自动跳过。
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
    // 缺 WebView2 运行库时，Tauri 会在创建窗口那一步直接失败，
    // 用户看到的现象是「双击没反应」，完全无从下手。
    // 这里提前拦下来，给一句中文说明 + 一键打开官网。
    //
    // 注意：必须在 Builder 之前 —— 一旦开始建窗口，就只剩 Tauri 自己的英文提示了。
    #[cfg(windows)]
    webview2::ensure_runtime();

    tauri::Builder::default()
        // ---- 插件注册 ----
        // 剪贴板：任务书第七节第 5 条 —— 因 WebView 中 navigator.clipboard
        // 的可用性在不同平台/权限下不一致，统一改用官方插件。
        .plugin(tauri_plugin_clipboard_manager::init())
        // 打开外部链接（GitHub 仓库、官方文档）
        .plugin(tauri_plugin_opener::init())
        // ---- 全局状态 ----
        .manage(AppState::new())
        // Key 管家自带一份状态（模块隔离，不与探测模块的 AppState 混在一起）
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
        // 注意：这里的 7 个名字必须与前端 src/api/commands.ts 中的
        //    COMMAND_NAMES 常量逐字一致，阶段 5 QA 会逐一核对。
        .invoke_handler(tauri::generate_handler![
            commands::list_models,        // 1. 拉取模型列表
            commands::ping_connection,    // 2. 连通性预检
            commands::start_probe,        // 3. 启动完整探测
            commands::stop_probe,         // 4. 中止探测
            commands::get_probe_state,    // 5. 查询实时快照
            commands::save_config_file,   // 6. 导出配置到磁盘
            commands::get_dictionary_info, // 7. 字典自检（辅助）
            // ---- Key 管家（Beta）：15 个命令 ----
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
            key_butler::commands::key_butler_snapshots
        ])
        .run(tauri::generate_context!())
        .expect("ModelScout 启动失败：Tauri 应用初始化异常");
}
