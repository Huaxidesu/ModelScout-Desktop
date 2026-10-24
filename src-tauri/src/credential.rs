//! 从 Windows 凭据管理器按 target 名读取「通用凭据（Generic Credential）」
//! 的明文内容，供 `ProbeConfig::resolve_api_key()` 作为 API Key 的来源之一。
//!
//! - 仅需 CredReadW / CredFree 两个函数，`windows-sys` 已在依赖树中
//!   （Tauri 的传递依赖），加一个 feature 即可，不为此引入第三方 crate
//!   （如 keyring，其会带进一整套平台后端，与单一需求不成比例）；
//! - 安全约定：本模块返回明文密钥，调用方只能将其交给 `ApiClient`，
//!   不得打印、不得写入事件 payload、不得回传前端；本模块自身不产生任何日志。

/// 从 Windows 凭据管理器读取一条「通用凭据」的明文内容。
///
/// # 参数
/// - `target`: 凭据的 target 名（在「凭据管理器 → Windows 凭据」中显示的名称）
///
/// # 返回
/// - `Ok(String)`：去掉首尾空白与尾部 NUL 的明文
/// - `Err(String)`：面向用户的中文错误（未找到 / 内容为空 / 平台不支持）
#[cfg(windows)]
pub fn read_generic_credential(target: &str) -> Result<String, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::Credentials::{
        CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
    };

    let name = target.trim();
    if name.is_empty() {
        return Err("凭据名不能为空。".to_string());
    }

    // CredReadW 要求以 NUL 结尾的 UTF-16 字符串
    let wide: Vec<u16> = std::ffi::OsStr::new(name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut raw: *mut CREDENTIALW = std::ptr::null_mut();

    // SAFETY: `wide` 在本函数作用域内始终有效；`raw` 由系统分配，
    // 成功（返回非 0）时指向一个有效的 CREDENTIALW。
    let ok = unsafe { CredReadW(wide.as_ptr(), CRED_TYPE_GENERIC, 0, &mut raw) };
    if ok == 0 || raw.is_null() {
        // 失败原因通常是「凭据不存在」（ERROR_NOT_FOUND，1168）
        let err = std::io::Error::last_os_error();
        return Err(format!(
            "未能从 Windows 凭据管理器读取凭据「{name}」：{err}。\
             请确认已在「控制面板 → 凭据管理器 → Windows 凭据」中\
             添加一条「通用凭据」，且名称与此处完全一致。"
        ));
    }

    // SAFETY: 上一段已确认 raw 非空且指向有效结构；
    // CredentialBlob 指向 CredentialBlobSize 字节的数据（可能为空）。
    let secret = unsafe {
        let cred = &*raw;
        if cred.CredentialBlob.is_null() || cred.CredentialBlobSize == 0 {
            String::new()
        } else {
            let blob =
                std::slice::from_raw_parts(cred.CredentialBlob, cred.CredentialBlobSize as usize);
            // 凭据内容一般按 UTF-8 写入；这里用 lossy 解码避免非法字节导致 panic。
            String::from_utf8_lossy(blob)
                .trim_end_matches('\0')
                .trim()
                .to_string()
        }
    };

    // 无论后续如何，都要释放系统分配的内存。
    // SAFETY: raw 由 CredReadW 分配，只能由 CredFree 释放，且只释放一次。
    unsafe { CredFree(raw as *const core::ffi::c_void) };

    if secret.is_empty() {
        return Err(format!("凭据「{name}」内容为空。"));
    }
    Ok(secret)
}

/// 非 Windows 平台的占位实现。
///
/// 保持同一函数签名，使 `ProbeConfig::resolve_api_key()` 不必到处写 `#[cfg]`；
/// 在这些平台上调用会得到一个明确的中文错误，而不是编译失败。
#[cfg(not(windows))]
pub fn read_generic_credential(_target: &str) -> Result<String, String> {
    Err("从凭据管理器读取 API Key 仅支持 Windows 平台。".to_string())
}
