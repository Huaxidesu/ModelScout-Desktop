//! Key 管家：Windows DPAPI 封装。
//!
//! 「无密码保存」要求用户不输入主密码，磁盘上依然不能出现明文；
//! 没有用户口令时唯一可用的秘密来源是操作系统账户本身。
//! DPAPI（CryptProtectData / CryptUnprotectData）用当前登录用户的凭据
//! 保护数据，密文只能由同一个 Windows 用户解开。
//!
//! 用法：启用无密码保存时随机生成 32 字节数据密钥，经 DPAPI 包裹后
//! 存进密文库文件；Key 列表仍用该数据密钥走 AES-256-GCM 加密，
//! 与主密码模式共用同一套加解密代码路径，仅「数据密钥从哪来」不同。
//! 文件被复制走、备份泄漏、换 Windows 账户登录时均无法解开；
//! 用户本人打开应用无需输入任何内容。
//!
//! 保护边界（必须如实告知用户）：DPAPI 只防「不是这个 Windows 用户」的人；
//! 同一账户下运行的任何程序都能解开，挡不住本机恶意软件，也挡不住
//! 他人直接使用该账户登录。该说明必须出现在启用前的注意事项里。
//!
//! 不引第三方 crate 的原因：只需两个函数，windows-sys 已在依赖树中
//! （Tauri 的传递依赖），加一个 feature 即可（与 `src/credential.rs` 同一取舍）。

/// DPAPI 的「附加熵」。
///
/// 它不是密钥（会明文出现在二进制里），作用只是域隔离：
/// 让本应用保护的密文不会与同一账户下其他程序用 DPAPI 保护的数据互相解开。
#[cfg(windows)]
const ENTROPY: &[u8] = b"ModelScout::key_butler::v1";

/// 用当前 Windows 用户凭据保护一段数据。
///
/// 输出是不透明的二进制块，只能由 `unprotect` 在同一个 Windows 用户下解开。
#[cfg(windows)]
pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: ENTROPY.len() as u32,
        pbData: ENTROPY.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();

    // SAFETY: 两个输入 blob 都指向本函数作用域内有效的切片；
    // `output` 由系统分配，成功（返回非 0）时由 take_blob 负责释放。
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            &entropy,
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(format!(
            "调用系统加密接口（DPAPI）失败：{}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(take_blob(output))
}

/// 用当前 Windows 用户凭据解开 `protect` 的产物。
#[cfg(windows)]
pub fn unprotect(blob: &[u8]) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: blob.len() as u32,
        pbData: blob.as_ptr() as *mut u8,
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: ENTROPY.len() as u32,
        pbData: ENTROPY.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // DPAPI 会把「数据的描述」写进这里，我们不需要，但必须接收并释放
    let mut description: windows_sys::core::PWSTR = std::ptr::null_mut();

    // SAFETY: 同 protect；`description` 与 `output` 均由系统分配。
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            &mut description,
            &entropy,
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(format!(
            "无法用当前 Windows 账户解开本机保存的数据：{}。\
             该文件可能由**其他 Windows 用户**或**另一台电脑**创建。",
            std::io::Error::last_os_error()
        ));
    }

    // SAFETY: description 由 CryptUnprotectData 分配，只能由 LocalFree 释放
    if !description.is_null() {
        unsafe { LocalFree(description as *mut core::ffi::c_void) };
    }
    Ok(take_blob(output))
}

/// 把系统分配的 blob 拷成 `Vec<u8>` 并释放系统内存。
///
/// DPAPI 的输出内存由 `LocalAlloc` 分配，必须用 `LocalFree` 释放 ——
/// 漏掉就是一处每次调用都会增长的内存泄漏。
#[cfg(windows)]
fn take_blob(blob: windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB) -> Vec<u8> {
    use windows_sys::Win32::Foundation::LocalFree;

    if blob.pbData.is_null() || blob.cbData == 0 {
        return Vec::new();
    }
    // SAFETY: 系统保证 pbData 指向 cbData 字节的有效内存
    let bytes = unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize) }.to_vec();
    // SAFETY: pbData 由 DPAPI 分配，只能由 LocalFree 释放，且只释放一次
    unsafe { LocalFree(blob.pbData as *mut core::ffi::c_void) };
    bytes
}

/// 非 Windows 平台的占位实现。
///
/// 保持同一函数签名，让 `vault.rs` 不必到处写 `#[cfg]`；在这些平台上调用会
/// 得到一个明确的中文错误（并引导用户改用主密码保存），而不是编译失败。
#[cfg(not(windows))]
pub fn protect(_data: &[u8]) -> Result<Vec<u8>, String> {
    Err("「无密码保存」依赖 Windows 数据保护接口（DPAPI），当前平台暂不支持，\
         请改用主密码保存。"
        .to_string())
}

/// 非 Windows 平台的占位实现（见 `protect`）。
#[cfg(not(windows))]
pub fn unprotect(_blob: &[u8]) -> Result<Vec<u8>, String> {
    Err("「无密码保存」依赖 Windows 数据保护接口（DPAPI），当前平台暂不支持，\
         请改用主密码保存。"
        .to_string())
}

// ---- 单元测试 ----

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn protect_then_unprotect_roundtrips() {
        let secret = b"sk-1d84f27e44de4a669ea2d345b7b3ae76";
        let blob = protect(secret).expect("DPAPI 加密应成功");
        assert_ne!(blob, secret, "密文不应等于明文");
        assert!(blob.len() > secret.len(), "DPAPI 输出应当包含头部");
        let back = unprotect(&blob).expect("DPAPI 解密应成功");
        assert_eq!(back, secret);
    }

    #[test]
    fn protect_is_not_deterministic() {
        // 同一段数据两次保护的输出应当不同（DPAPI 内部带随机盐）
        let secret = b"same-input-twice";
        let first = protect(secret).unwrap();
        let second = protect(secret).unwrap();
        assert_ne!(first, second, "两次保护不应产生完全相同的密文");
    }

    #[test]
    fn unprotect_rejects_garbage_without_panicking() {
        assert!(unprotect(b"definitely-not-a-dpapi-blob").is_err());
        assert!(unprotect(&[]).is_err());
    }

    #[test]
    fn roundtrip_handles_empty_and_large_inputs() {
        for size in [0usize, 1, 32, 4096] {
            let data = vec![0xA5u8; size];
            let blob = protect(&data).expect("加密应成功");
            let back = unprotect(&blob).expect("解密应成功");
            assert_eq!(back, data, "长度 {size} 的往返应一致");
        }
    }
}
