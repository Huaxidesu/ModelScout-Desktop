/**
 * ============================================================================
 * ModelScout —— Windows 启动前检查：WebView2 运行时
 * ============================================================================
 *
 * 【为什么需要】
 * Tauri 在 Windows 上依赖系统自带的 **WebView2 运行时**来渲染界面。
 * Windows 11 与较新的 Windows 10 已预装，但精简版镜像、长期未更新的
 * Win10、部分 LTSC 版本可能没有 —— 此时程序会直接启动失败，
 * 用户看到的是「双击没反应」，完全无从下手。
 *
 * 【为什么不直接靠 Tauri 自己的处理】
 * Tauri 确实也处理了这种情况：`tauri-runtime-wry` 在 release 构建下会弹一个
 * 英文错误框，里面带可点击的官网链接。但那个提示有三个问题：
 *   1. 是**英文**的；
 *   2. 依赖 Tauri 内部实现，将来版本变化可能改掉；
 *   3. 只在 `not(debug_assertions)` 下触发。
 * 因此这里自行实现一遍：中文提示，并且把「去哪下载」直接变成一次点击。
 *
 * 【检测方式：跟着上游用同一个判据】
 * 复用 `tauri::webview_version()` —— 它底层就是 `wry::webview_version()`，
 * 而 Tauri 内部判断「运行库装没装」用的**正是同一个调用**：
 *
 * ```text
 * webview_runtime_installed: wry::webview_version().is_ok()
 * ```
 *
 * 自己写注册表查询当然也能做（微软文档里有），但没必要：跟着上游用同一个
 * 判据，才不会出现「本模块判定已安装、Tauri 判定未安装」这种不一致的情况。
 *
 * 【为什么这里用裸 Win32】
 * 这个提示必须在 WebView 创建**之前**弹出来 —— 此刻进程里还没有任何窗口
 * 系统可用，Tauri 的对话框、rfd 之类都指望不上。只能走最底层的
 * `MessageBoxW`。为了避免为此新增依赖，直接用 `windows-sys`
 * （它本来就在依赖树里，是 Tauri 的传递依赖）。
 * ============================================================================
 */

use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDOK, MB_ICONERROR, MB_OKCANCEL, MB_TOPMOST, SW_SHOWNORMAL,
};

/* ---------------------------------------------------------------------------
 * 常量
 * ------------------------------------------------------------------------- */

/// 微软官方的 WebView2 下载页。
///
/// 用 `developer.microsoft.com` 的正式地址而不是 `aka.ms` 短链 ——
/// 这个字符串会被编进二进制，发出去就改不动了，正式地址更稳。
pub const DOWNLOAD_URL: &str = "https://developer.microsoft.com/microsoft-edge/webview2/";

const DIALOG_TITLE: &str = "ModelScout 无法启动";

/// 弹窗正文。
///
/// 写清楚三件事：**出了什么问题**、**点哪个按钮会发生什么**、**装完怎么继续**。
/// 用户看到弹窗的第一反应是「我该点哪个」，不写明白就只能瞎猜 ——
/// 尤其是带「取消」的弹窗，不说清楚就会让人以为程序要装什么东西。
const DIALOG_TEXT: &str = "\
ModelScout 需要 Microsoft Edge WebView2 运行时才能显示界面，\
但当前系统没有安装。

这通常出现在精简版系统或长期未更新的 Windows 上；\
Windows 11 与较新的 Windows 10 一般已自带。

点「确定」→ 打开微软官方下载页，安装后重新运行 ModelScout 即可。
点「取消」→ 退出。";

/* ---------------------------------------------------------------------------
 * 对外接口
 * ------------------------------------------------------------------------- */

/// 检测 WebView2 运行时是否可用。
///
/// 与 Tauri 内部使用完全相同的判据（见文件头说明）。
pub fn runtime_available() -> bool {
    tauri::webview_version().is_ok()
}

/// 启动前检查（preflight）：缺运行库就提示用户并退出，不再往下走。
///
/// 由 `run()` 在构造 Tauri Builder **之前**调用。
/// 这个顺序是必须的 —— 一旦开始创建窗口，失败信息就只剩 Tauri 自己的
/// 英文提示了，此处准备的中文说明就没机会出现。
pub fn ensure_runtime() {
    if runtime_available() {
        return;
    }

    eprintln!("[ModelScout] 未检测到 WebView2 运行时，无法启动，已提示用户前往官网下载。");
    prompt_and_exit();
}

/* ---------------------------------------------------------------------------
 * Win32 交互
 * ------------------------------------------------------------------------- */

/// 弹出提示框；用户点「确定」则打开官网，随后退出进程。
fn prompt_and_exit() -> ! {
    let text = to_wide(DIALOG_TEXT);
    let title = to_wide(DIALOG_TITLE);

    // SAFETY: `MessageBoxW` 是同步调用。两个指针都指向本函数内构造的、
    // 以 NUL 结尾的 UTF-16 缓冲，在调用期间始终存活。
    // 父窗口传 null：此刻还没有任何窗口，也不希望它挂靠在别的进程窗口上。
    // `MB_TOPMOST` 保证提示不会被其它窗口盖住 —— 用户此刻多半刚双击完，
    // 前台窗口随时可能变化。
    let choice = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OKCANCEL | MB_ICONERROR | MB_TOPMOST,
        )
    };

    if choice == IDOK {
        open_in_browser(DOWNLOAD_URL);
    }

    // 非 0 退出码：让「程序没启动成功」在脚本/任务管理器里也能看出来。
    std::process::exit(1);
}

/// 用系统默认浏览器打开一个 URL。
///
/// 不用 Tauri 的 opener 插件 —— 那个需要 App 实例，而此刻应用还没建起来。
fn open_in_browser(url: &str) {
    let operation = to_wide("open");
    let target = to_wide(url);

    // SAFETY: 三个指针都在调用期间有效。后两个参数传 null，
    // 分别表示「无启动参数」与「使用当前工作目录」。
    // 返回值是 HINSTANCE，按 Win32 约定小于等于 32 表示失败 ——
    // 这里无法补救（浏览器都没打开），故忽略；用户至少还能手抄弹窗里的网址。
    let _ = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
}

/// 转成以 NUL 结尾的 UTF-16 —— Win32 的 `*W` 系列 API 要的就是这个。
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/* ---------------------------------------------------------------------------
 * 测试
 * ------------------------------------------------------------------------- */

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_url_points_to_official_microsoft_site() {
        // 这个地址会被编进二进制，写错就等于把用户送去别处
        assert!(
            DOWNLOAD_URL.starts_with("https://developer.microsoft.com/"),
            "下载地址必须是微软官方域名，实际为 {DOWNLOAD_URL}"
        );
        assert!(!DOWNLOAD_URL.contains("aka.ms"), "避免使用可能失效的短链");
    }

    #[test]
    fn dialog_text_tells_user_what_each_button_does() {
        // 弹窗里必须能看出「去哪下」以及「两个按钮分别会怎样」——
        // 只说「缺少运行库」而不给下一步，等于没提示
        assert!(DIALOG_TEXT.contains("WebView2"));
        assert!(DIALOG_TEXT.contains("确定"));
        assert!(DIALOG_TEXT.contains("取消"));
        assert!(DIALOG_TEXT.contains("重新运行"));
    }

    #[test]
    fn runtime_check_is_safe_to_call() {
        // 只要求「能安全调用、不 panic」。
        // 断言具体返回值会让测试变成环境依赖：装了运行库的机器返回 true，
        // 没装的返回 false，两者都正常。
        let _ = runtime_available();
    }

    #[test]
    fn to_wide_appends_nul_terminator() {
        assert_eq!(to_wide("abc"), vec![0x61, 0x62, 0x63, 0x00]);
        assert_eq!(to_wide(""), vec![0x00]);
    }

    #[test]
    fn to_wide_encodes_chinese_as_utf16() {
        // 中文在 UTF-16 里是单个 u16（BMP 内），不是 UTF-8 那样的三字节
        assert_eq!(to_wide("中"), vec![0x4E2D, 0x0000]);
        // 补充平面字符（emoji）会占两个 u16 —— 确认没有按「一个字符一个 u16」写错
        assert_eq!(to_wide("🚀"), vec![0xD83D, 0xDE80, 0x0000]);
    }
}
