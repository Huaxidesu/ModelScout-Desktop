/**
 * ============================================================================
 * ModelScout —— Key 管家：内存表与可选加密持久化
 * ============================================================================
 *
 * 【安全模型（任务书第四节）】
 *   1. **默认不持久化**：不启用加密保存时，任何操作都**不写盘**，
 *      关闭应用即清空（只有余额快照是例外，它不含明文）。
 *   2. **加密保存需手动启用**并设置主密码：
 *      argon2id(m=19456, t=2, p=1) 派生 32 字节密钥 → AES-256-GCM 加密，
 *      密文写入 `{app_data_dir}/key_butler/vault.json`。
 *   3. 主密码**不可找回**。派生密钥在解锁期间缓存在内存里（否则每次改动
 *      都要重新输入密码），关闭加密保存或进程退出即清零。
 *      主密码字符串本身在派生完成后立即 `zeroize`。
 *   4. 明文 Key 只存在于本模块的 `StoredKey` 里，`KeyEntry` 只带脱敏串。
 *
 * 【锁定态的行为】
 * 密文库已启用但未解锁时，内存表是空的。此时**拒绝**一切写操作
 * （导入 / 删除 / 改配置 / 检测），否则新数据既不会落盘、又会在下次解锁时
 * 被磁盘内容覆盖而凭空消失。读操作（列表）返回空表，由前端引导用户解锁。
 * ============================================================================
 */

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use zeroize::{Zeroize, Zeroizing};

use crate::key_butler::models::{KeyCheckSummary, KeyEntry, KeyStatus, Snapshot, VaultStatus};
use crate::key_butler::snapshots::{SnapshotStore, MAX_PER_KEY};

/* ---------------------------------------------------------------------------
 * 常量
 * ------------------------------------------------------------------------- */

/// argon2id 参数（OWASP 推荐）。
const ARGON2_M_COST: u32 = 19456;
const ARGON2_T_COST: u32 = 2;
const ARGON2_P_COST: u32 = 1;
/// 派生密钥长度（AES-256）。
const KEY_LEN: usize = 32;
/// 盐长度。
const SALT_LEN: usize = 16;
/// AES-GCM nonce 长度。
const NONCE_LEN: usize = 12;
/// 主密码最短长度。
pub const MIN_PASSWORD_LEN: usize = 8;

/* ---------------------------------------------------------------------------
 * 脱敏
 * ------------------------------------------------------------------------- */

/// Key 的展示脱敏：前 5 + `***` + 后 4（如 `sk-ab***cdef`）。
///
/// 注意：这与 `error::redact_key()` 是**两套**格式 —— 后者用于错误信息
/// （`***REDACTED***` 风格），不要混用。
pub fn mask_key(key: &str) -> String {
    let trimmed = key.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= 9 {
        return "***".to_string();
    }
    let head: String = chars.iter().take(5).collect();
    let tail: String = chars.iter().skip(chars.len() - 4).collect();
    format!("{head}***{tail}")
}

/* ---------------------------------------------------------------------------
 * 内存中的一条 Key
 * ------------------------------------------------------------------------- */

/// 内存表条目：脱敏元数据 + 明文。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredKey {
    /// 对外可见的元数据（不含明文）
    pub entry: KeyEntry,
    /// 明文 Key。**只在本模块内流转**，用于构造 Authorization 头。
    pub key: String,
}

impl StoredKey {
    /// 用明文与元数据构造。
    pub fn new(key: String, entry: KeyEntry) -> Self {
        Self { entry, key }
    }

    /// 脱敏后的展示串。
    pub fn masked(&self) -> String {
        self.entry.masked_key.clone()
    }
}

impl Drop for StoredKey {
    /// 丢弃时清零明文，避免明文长期滞留在堆内存里。
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

/* ---------------------------------------------------------------------------
 * 密文文件格式
 * ------------------------------------------------------------------------- */

/// 磁盘上的密文库。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VaultFile {
    /// 文件格式版本
    version: u32,
    /// 密钥派生算法标识（当前只有 argon2id）
    kdf: String,
    /// argon2id 参数（写进文件，便于将来调参后仍能解密旧文件）
    m: u32,
    t: u32,
    p: u32,
    /// 盐（base64）
    salt: String,
    /// nonce（base64）—— **每次加密都重新生成**
    nonce: String,
    /// 密文（base64）
    ciphertext: String,
}

/// 明文载荷。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VaultPayload {
    version: u32,
    keys: Vec<StoredKey>,
}

/* ---------------------------------------------------------------------------
 * 路径
 * ------------------------------------------------------------------------- */

/// Key 管家的两个落盘位置。
#[derive(Debug, Clone)]
pub struct KeyButlerPaths {
    /// 密文库
    pub vault: PathBuf,
    /// 余额快照（明文，不含 Key）
    pub snapshots: PathBuf,
}

/* ---------------------------------------------------------------------------
 * 检测运行状态
 * ------------------------------------------------------------------------- */

/// 一次批量检测的运行状态（协作式中止，与探测模块同一语义）。
///
/// 计数器放在这里而不是命令层：`key_butler_check_cancel` 需要在不持有
/// 检测任务句柄的情况下，返回一份「中止那一刻」的汇总。
#[derive(Debug)]
pub struct CheckRun {
    /// 运行 id（Unix 毫秒字符串）
    pub run_id: String,
    /// 是否已请求中止
    cancelled: AtomicBool,
    /// 开始时刻
    started: Instant,
    /// 总条数
    pub total: AtomicUsize,
    /// 已完成条数
    pub completed: AtomicUsize,
    /// 四态计数
    pub active: AtomicUsize,
    pub insufficient: AtomicUsize,
    pub invalid: AtomicUsize,
    pub error: AtomicUsize,
}

impl CheckRun {
    pub fn new(run_id: String, total: usize) -> Self {
        Self {
            run_id,
            cancelled: AtomicBool::new(false),
            started: Instant::now(),
            total: AtomicUsize::new(total),
            completed: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            insufficient: AtomicUsize::new(0),
            invalid: AtomicUsize::new(0),
            error: AtomicUsize::new(0),
        }
    }

    /// 请求中止。
    pub fn request_cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// 是否已请求中止。
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// 已完成条数。
    pub fn completed(&self) -> usize {
        self.completed.load(Ordering::SeqCst)
    }

    /// 总条数。
    pub fn total(&self) -> usize {
        self.total.load(Ordering::SeqCst)
    }

    /// 按状态累加计数。
    pub fn bump(&self, status: KeyStatus) {
        let counter = match status {
            KeyStatus::Active => &self.active,
            KeyStatus::Insufficient => &self.insufficient,
            KeyStatus::Invalid => &self.invalid,
            KeyStatus::Error => &self.error,
        };
        counter.fetch_add(1, Ordering::SeqCst);
    }

    /// 当前汇总快照。
    pub fn summary(&self) -> KeyCheckSummary {
        KeyCheckSummary {
            run_id: self.run_id.clone(),
            checked: self.completed(),
            active: self.active.load(Ordering::SeqCst),
            insufficient: self.insufficient.load(Ordering::SeqCst),
            invalid: self.invalid.load(Ordering::SeqCst),
            error: self.error.load(Ordering::SeqCst),
            elapsed_ms: self.started.elapsed().as_millis() as u64,
        }
    }
}

/* ---------------------------------------------------------------------------
 * 应用状态
 * ------------------------------------------------------------------------- */

/// 内部可变状态。
struct Inner {
    /// 内存表（锁定态或未启用时可能是空表）
    keys: Vec<StoredKey>,
    /// 解锁后缓存的派生密钥；未解锁 / 未启用时为 `None`
    derived: Option<Zeroizing<[u8; KEY_LEN]>>,
}

impl Inner {
    fn new() -> Self {
        Self {
            keys: Vec::new(),
            derived: None,
        }
    }

    /// 当前会话是否已解锁。
    fn unlocked(&self) -> bool {
        self.derived.is_some()
    }
}

/// 被 Tauri 托管的 Key 管家状态。
///
/// 与探测模块的 `AppState` 完全独立（模块隔离原则），单独 `manage`。
pub struct KeyButlerState {
    inner: Mutex<Inner>,
    /// 当前批量检测运行；空闲时 `None`
    run: Mutex<Option<Arc<CheckRun>>>,
    /// 会话内的余额快照缓冲。
    ///
    /// 【为什么要单独有一份内存缓冲】
    /// 「默认不持久化」这条红线要求：**加密保存未启用时一个字节都不写盘**。
    /// 但余额历史又是要用的功能 —— 于是快照先进内存，只有加密保存启用后
    /// 才同时落盘。这样既满足零写盘，又让本次会话内的曲线可用。
    snapshots: Mutex<Vec<Snapshot>>,
    /// 落盘路径；由 `lib.rs` 的 setup 阶段注入
    paths: OnceLock<KeyButlerPaths>,
}

impl Default for KeyButlerState {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyButlerState {
    /// 构造空状态（路径稍后由 `configure` 注入）。
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::new()),
            run: Mutex::new(None),
            snapshots: Mutex::new(Vec::new()),
            paths: OnceLock::new(),
        }
    }

    /// 注入落盘路径。由 `lib.rs` 的 setup 调用（幂等）。
    pub fn configure(&self, data_dir: &Path) {
        let base = data_dir.join("key_butler");
        let _ = self.paths.set(KeyButlerPaths {
            vault: base.join("vault.json"),
            snapshots: base.join("snapshots.json"),
        });
    }

    /// 取落盘路径。
    fn paths(&self) -> Result<&KeyButlerPaths, String> {
        self.paths
            .get()
            .ok_or_else(|| "Key 管家的存储路径尚未初始化。".to_string())
    }

    /// 密文库是否已在磁盘上存在（即此前启用过）。
    fn enabled_on_disk(&self) -> bool {
        self.paths
            .get()
            .map(|paths| paths.vault.exists())
            .unwrap_or(false)
    }

    /// 余额快照存储（内部使用）。
    fn snapshot_store(&self) -> Result<SnapshotStore, String> {
        Ok(SnapshotStore::new(self.paths()?.snapshots.clone()))
    }

    /// 密文库是否已在磁盘上存在（即此前启用过）。
    ///
    /// 公开出去是给命令层判断「要不要落盘」用的 ——
    /// 这是「默认不持久化」红线的唯一开关。
    pub fn is_enabled(&self) -> bool {
        self.enabled_on_disk()
    }

    /// 追加一条余额快照。
    ///
    /// **始终进内存缓冲**；只有在加密保存已启用时才额外写盘。
    /// 未启用时本函数不触碰文件系统。
    pub async fn append_snapshot(&self, snapshot: Snapshot) -> Result<(), String> {
        {
            let mut buffer = self.snapshots.lock().await;
            let key_id = snapshot.key_id.clone();
            buffer.push(snapshot.clone());

            // 与磁盘同样的裁剪策略，避免长时间运行把内存撑起来
            let mut indices: Vec<usize> = buffer
                .iter()
                .enumerate()
                .filter(|(_, item)| item.key_id == key_id)
                .map(|(index, _)| index)
                .collect();
            if indices.len() > MAX_PER_KEY {
                indices.sort_by_key(|&index| buffer[index].ts);
                let drop_count = indices.len() - MAX_PER_KEY;
                let to_remove: HashSet<usize> =
                    indices.into_iter().take(drop_count).collect();
                let mut index = 0usize;
                buffer.retain(|_| {
                    let keep = !to_remove.contains(&index);
                    index += 1;
                    keep
                });
            }
        }

        // 未启用加密保存 → 到此为止，绝不写盘
        if !self.enabled_on_disk() {
            return Ok(());
        }
        self.snapshot_store()?.append(snapshot)
    }

    /// 查询某条 Key 的全部快照（内存缓冲 + 磁盘，按 ts 去重后升序）。
    pub async fn list_snapshots(&self, key_id: &str) -> Vec<Snapshot> {
        let mut merged: Vec<Snapshot> = Vec::new();

        if self.enabled_on_disk() {
            if let Ok(store) = self.snapshot_store() {
                merged.extend(store.list(key_id));
            }
        }

        {
            let buffer = self.snapshots.lock().await;
            for item in buffer.iter().filter(|item| item.key_id == key_id) {
                if !merged.iter().any(|existing| existing.ts == item.ts) {
                    merged.push(item.clone());
                }
            }
        }

        merged.sort_by_key(|item| item.ts);
        merged
    }

    /// 删除某条 Key 的全部快照（内存 + 磁盘）。
    pub async fn remove_snapshots(&self, key_id: &str) {
        {
            let mut buffer = self.snapshots.lock().await;
            buffer.retain(|item| item.key_id != key_id);
        }
        if self.enabled_on_disk() {
            if let Ok(store) = self.snapshot_store() {
                let _ = store.remove_key(key_id);
            }
        }
    }

    /// 当前状态。
    pub async fn status(&self) -> VaultStatus {
        let inner = self.inner.lock().await;
        VaultStatus {
            enabled: self.enabled_on_disk(),
            unlocked: inner.unlocked(),
            entry_count: inner.keys.len(),
        }
    }

    /// 脱敏条目列表。
    pub async fn entries(&self) -> Vec<KeyEntry> {
        let inner = self.inner.lock().await;
        inner.keys.iter().map(|k| k.entry.clone()).collect()
    }

    /// 按 id 取明文（仅供一键复制）。
    pub async fn reveal(&self, id: &str) -> Option<String> {
        let inner = self.inner.lock().await;
        inner
            .keys
            .iter()
            .find(|k| k.entry.id == id)
            .map(|k| k.key.clone())
    }

    /// 已存在的明文集合（用于导入去重）。
    pub async fn existing_keys(&self) -> Vec<String> {
        let inner = self.inner.lock().await;
        inner.keys.iter().map(|k| k.key.clone()).collect()
    }

    /// 取一批待检测目标（明文 + 元数据）。
    pub async fn targets(&self, ids: &[String]) -> Vec<(StoredKey, Zeroizing<String>)> {
        let inner = self.inner.lock().await;
        inner
            .keys
            .iter()
            .filter(|k| ids.is_empty() || ids.iter().any(|id| id == &k.entry.id))
            .map(|k| (k.clone(), Zeroizing::new(k.key.clone())))
            .collect()
    }

    /// 追加条目，返回实际写入的条数。
    pub async fn insert(&self, items: Vec<StoredKey>) -> usize {
        let mut inner = self.inner.lock().await;
        let count = items.len();
        inner.keys.extend(items);
        count
    }

    /// 删除一条，返回是否删掉。
    pub async fn remove(&self, id: &str) -> bool {
        let mut inner = self.inner.lock().await;
        let before = inner.keys.len();
        inner.keys.retain(|k| k.entry.id != id);
        inner.keys.len() != before
    }

    /// 就地把某个条目交给闭包修改；返回是否找到。
    pub async fn update<F>(&self, id: &str, apply: F) -> bool
    where
        F: FnOnce(&mut KeyEntry),
    {
        let mut inner = self.inner.lock().await;
        match inner.keys.iter_mut().find(|k| k.entry.id == id) {
            Some(stored) => {
                apply(&mut stored.entry);
                true
            }
            None => false,
        }
    }

    /// 就地把**一批**条目交给同一个闭包修改；返回实际改动的条数。
    ///
    /// 批量编辑必须一次拿锁改完：逐条调 `update` 会拿 N 次锁，
    /// 中途还可能被检测任务插进来看到「改了一半」的中间态。
    pub async fn update_many<F>(&self, ids: &[String], apply: F) -> usize
    where
        F: Fn(&mut KeyEntry),
    {
        if ids.is_empty() {
            return 0;
        }
        let mut inner = self.inner.lock().await;
        let mut changed = 0usize;
        for stored in inner.keys.iter_mut() {
            if ids.iter().any(|id| id == &stored.entry.id) {
                apply(&mut stored.entry);
                changed += 1;
            }
        }
        changed
    }

    /// 写操作的前置检查：密文库启用但未解锁时拒绝写入。
    pub async fn ensure_writable(&self) -> Result<(), String> {
        if !self.enabled_on_disk() {
            return Ok(());
        }
        let inner = self.inner.lock().await;
        if inner.unlocked() {
            Ok(())
        } else {
            Err("加密保存已启用但当前未解锁，请先输入主密码解锁后再操作。".to_string())
        }
    }

    /* ---------------------- 加密保存 ---------------------- */

    /// 启用加密保存：用当前内存表加密落盘。
    pub async fn enable(&self, password: &str) -> Result<VaultStatus, String> {
        if password.chars().count() < MIN_PASSWORD_LEN {
            return Err(format!("主密码至少需要 {MIN_PASSWORD_LEN} 个字符。"));
        }
        if self.enabled_on_disk() {
            return Err("加密保存已经启用。如需更换主密码，请先关闭再重新启用。".to_string());
        }
        let paths = self.paths()?.clone();

        // 生成新盐并派生密钥
        let salt = random_bytes(SALT_LEN)?;
        let mut password_buffer = Zeroizing::new(password.to_string());
        let derived = derive_key(&password_buffer, &salt, ARGON2_M_COST, ARGON2_T_COST, ARGON2_P_COST)?;
        password_buffer.zeroize();

        let mut inner = self.inner.lock().await;
        let payload = VaultPayload {
            version: 1,
            keys: inner.keys.clone(),
        };
        write_vault(&paths.vault, &payload, &derived, &salt)?;
        inner.derived = Some(Zeroizing::new(derived));

        Ok(VaultStatus {
            enabled: true,
            unlocked: true,
            entry_count: inner.keys.len(),
        })
    }

    /// 用主密码解锁已有密文库。
    pub async fn unlock(&self, password: &str) -> Result<VaultStatus, String> {
        let paths = self.paths()?.clone();
        if !paths.vault.exists() {
            return Err("尚未启用加密保存，无需解锁。".to_string());
        }

        let text = std::fs::read_to_string(&paths.vault)
            .map_err(|err| format!("读取密文库失败：{err}"))?;
        let file: VaultFile =
            serde_json::from_str(&text).map_err(|err| format!("密文库格式损坏：{err}"))?;

        let salt = BASE64
            .decode(file.salt.as_bytes())
            .map_err(|_| "密文库里的盐不是合法的 base64。".to_string())?;

        let mut password_buffer = Zeroizing::new(password.to_string());
        let derived = derive_key(&password_buffer, &salt, file.m, file.t, file.p)?;
        password_buffer.zeroize();

        let payload = read_payload(&file, &derived)
            .map_err(|_| "主密码错误，或密文库已损坏。".to_string())?;

        let mut inner = self.inner.lock().await;
        inner.keys = payload.keys;
        inner.derived = Some(Zeroizing::new(derived));

        Ok(VaultStatus {
            enabled: true,
            unlocked: true,
            entry_count: inner.keys.len(),
        })
    }

    /// 关闭加密保存：删除密文文件（内存表保留，用户还在用）。
    pub async fn disable(&self) -> Result<VaultStatus, String> {
        let paths = self.paths()?.clone();
        if paths.vault.exists() {
            std::fs::remove_file(&paths.vault)
                .map_err(|err| format!("删除密文库失败：{err}"))?;
        }
        let mut inner = self.inner.lock().await;
        // 派生密钥用后即清
        inner.derived = None;

        Ok(VaultStatus {
            enabled: false,
            unlocked: false,
            entry_count: inner.keys.len(),
        })
    }

    /// 若已启用且已解锁，则重新加密落盘（每次都用新的 nonce）。
    ///
    /// 未启用时**不写任何文件** —— 这是「默认不持久化」红线的落点。
    pub async fn persist_if_enabled(&self) -> Result<(), String> {
        if !self.enabled_on_disk() {
            return Ok(());
        }
        let paths = self.paths()?.clone();

        let inner = self.inner.lock().await;
        let Some(derived) = inner.derived.as_ref() else {
            // 锁定态下不写盘（写操作已被 ensure_writable 拦住，这里是防御性分支）
            return Ok(());
        };

        let payload = VaultPayload {
            version: 1,
            keys: inner.keys.clone(),
        };
        let salt = random_bytes(SALT_LEN)?;
        // 复制一份派生密钥用于本次加密；副本随作用域结束自动清零
        let key_copy = Zeroizing::new(**derived);
        write_vault(&paths.vault, &payload, &key_copy, &salt)
    }

    /* ---------------------- 检测运行 ---------------------- */

    /// 登记一次新的检测运行；若已有运行未结束则返回 `None`。
    pub async fn begin_run(&self, run_id: String, total: usize) -> Option<Arc<CheckRun>> {
        let mut guard = self.run.lock().await;
        if let Some(existing) = guard.as_ref() {
            if !existing.is_cancelled() {
                return None;
            }
        }
        let run = Arc::new(CheckRun::new(run_id, total));
        *guard = Some(run.clone());
        Some(run)
    }

    /// 结束检测运行。
    pub async fn end_run(&self) {
        *self.run.lock().await = None;
    }

    /// 请求中止当前检测；返回中止那一刻的汇总快照。
    ///
    /// 无运行中的任务时返回一份「空汇总」（runId 为空），不报错。
    pub async fn cancel_run(&self) -> KeyCheckSummary {
        let guard = self.run.lock().await;
        match guard.as_ref() {
            Some(run) => {
                run.request_cancel();
                run.summary()
            }
            None => KeyCheckSummary {
                run_id: String::new(),
                checked: 0,
                active: 0,
                insufficient: 0,
                invalid: 0,
                error: 0,
                elapsed_ms: 0,
            },
        }
    }
}

/* ---------------------------------------------------------------------------
 * 加解密原语
 * ------------------------------------------------------------------------- */

/// 生成指定长度的随机字节。
fn random_bytes(len: usize) -> Result<Vec<u8>, String> {
    let mut buffer = vec![0u8; len];
    getrandom::getrandom(&mut buffer).map_err(|err| format!("随机数生成失败：{err}"))?;
    Ok(buffer)
}

/// argon2id 派生密钥。
fn derive_key(
    password: &str,
    salt: &[u8],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
) -> Result<[u8; KEY_LEN], String> {
    let params = Params::new(m_cost, t_cost, p_cost, Some(KEY_LEN))
        .map_err(|err| format!("argon2 参数不合法：{err}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut out = [0u8; KEY_LEN];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|err| format!("密钥派生失败：{err}"))?;
    Ok(out)
}

/// 用派生密钥加密载荷并写入文件。
fn write_vault(
    path: &Path,
    payload: &VaultPayload,
    key: &[u8; KEY_LEN],
    salt: &[u8],
) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|err| format!("无法创建目录 {}：{err}", dir.display()))?;
    }

    let plaintext = serde_json::to_vec(payload)
        .map_err(|err| format!("序列化密文载荷失败：{err}"))?;

    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| "AES 密钥长度不合法。".to_string())?;

    // nonce 每次加密都必须重新随机生成，严禁复用
    let nonce_bytes = random_bytes(NONCE_LEN)?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|_| "加密失败。".to_string())?;

    let file = VaultFile {
        version: 1,
        kdf: "argon2id".to_string(),
        m: ARGON2_M_COST,
        t: ARGON2_T_COST,
        p: ARGON2_P_COST,
        salt: BASE64.encode(salt),
        nonce: BASE64.encode(&nonce_bytes),
        ciphertext: BASE64.encode(&ciphertext),
    };

    let text = serde_json::to_string_pretty(&file)
        .map_err(|err| format!("序列化密文库失败：{err}"))?;

    // 原子替换：先写临时文件再 rename，避免写入中途崩溃留下半截密文
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text.as_bytes()).map_err(|err| format!("写入密文库失败：{err}"))?;
    std::fs::rename(&tmp, path).map_err(|err| format!("替换密文库失败：{err}"))?;
    Ok(())
}

/// 用派生密钥解出载荷；密码错误或数据损坏时返回 `Err`。
fn read_payload(file: &VaultFile, key: &[u8; KEY_LEN]) -> Result<VaultPayload, String> {
    let nonce_bytes = BASE64
        .decode(file.nonce.as_bytes())
        .map_err(|_| "nonce 不是合法的 base64。".to_string())?;
    if nonce_bytes.len() != NONCE_LEN {
        return Err("nonce 长度不合法。".to_string());
    }
    let ciphertext = BASE64
        .decode(file.ciphertext.as_bytes())
        .map_err(|_| "密文不是合法的 base64。".to_string())?;

    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| "AES 密钥长度不合法。".to_string())?;
    let nonce = Nonce::from_slice(&nonce_bytes);

    let mut plaintext = cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|_| "解密失败（GCM 校验不通过）。".to_string())?;

    let parsed = serde_json::from_slice::<VaultPayload>(&plaintext);
    // 明文缓冲区用后即清
    plaintext.zeroize();
    parsed.map_err(|err| format!("解密后的内容不是合法载荷：{err}"))
}

/* ---------------------------------------------------------------------------
 * 单元测试
 * ------------------------------------------------------------------------- */

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key_butler::models::{BalanceEndpointKind, KeyEntry, KeyStatus};

    /// 造一个测试用的 KeyEntry（元数据与明文无关）。
    fn sample_entry(id: &str, masked: &str) -> KeyEntry {
        KeyEntry {
            id: id.to_string(),
            masked_key: masked.to_string(),
            platform: "unknown".to_string(),
            note: String::new(),
            created_at: 0,
            last_checked_at: None,
            status: KeyStatus::Error,
            key_valid: None,
            balance: None,
            balance_known: false,
            currency: None,
            error_category: None,
            balance_endpoint: BalanceEndpointKind::BuiltinOneapi,
            custom_endpoint: None,
            base_url: None,
        }
    }

    fn temp_state(tag: &str) -> KeyButlerState {
        let dir = std::env::temp_dir().join(format!("modelscout-vault-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        let state = KeyButlerState::new();
        state.configure(&dir);
        state
    }

    #[test]
    fn mask_key_uses_five_and_four() {
        assert_eq!(mask_key("sk-abcdefghijklmnop"), "sk-ab***mnop");
        assert_eq!(mask_key("sk-ab***cdef"), "sk-ab***cdef");
        // 过短时退化为全遮蔽，不 panic
        assert_eq!(mask_key("short"), "***");
        assert_eq!(mask_key(""), "***");
    }

    #[tokio::test]
    async fn encryption_roundtrip_restores_plaintext() {
        let state = temp_state("roundtrip");
        state
            .insert(vec![
                StoredKey::new("sk-ant-secret-value-1".to_string(), sample_entry("id-1", "sk-an***e-1")),
                StoredKey::new("sk-secret-value-2".to_string(), sample_entry("id-2", "sk-se***e-2")),
            ])
            .await;

        let status = state.enable("correct horse battery").await.expect("启用应成功");
        assert!(status.enabled && status.unlocked);
        assert_eq!(status.entry_count, 2);

        // 换一个全新状态，用正确密码解锁
        let reopened = KeyButlerState::new();
        reopened.configure(&state.paths().unwrap().vault.parent().unwrap().parent().unwrap());
        let unlocked = reopened.unlock("correct horse battery").await.expect("解锁应成功");
        assert_eq!(unlocked.entry_count, 2);
        assert_eq!(
            reopened.reveal("id-1").await.as_deref(),
            Some("sk-ant-secret-value-1")
        );
    }

    #[tokio::test]
    async fn wrong_password_is_rejected_without_panic() {
        let state = temp_state("wrongpw");
        state
            .insert(vec![StoredKey::new(
                "sk-some-secret".to_string(),
                sample_entry("id-1", "sk-so***cret"),
            )])
            .await;
        state.enable("correct horse battery").await.unwrap();

        let reopened = KeyButlerState::new();
        reopened.configure(&state.paths().unwrap().vault.parent().unwrap().parent().unwrap());
        let result = reopened.unlock("wrong password here").await;
        assert!(result.is_err(), "错误密码必须失败");
        assert!(result.unwrap_err().contains("主密码错误"));
    }

    #[tokio::test]
    async fn disabled_vault_writes_nothing() {
        let state = temp_state("nowrite");
        state
            .insert(vec![StoredKey::new(
                "sk-nothing-written".to_string(),
                sample_entry("id-1", "sk-no***tten"),
            )])
            .await;
        // 未启用：持久化调用应当是空操作
        state.persist_if_enabled().await.unwrap();

        let dir = state.paths().unwrap().vault.parent().unwrap().to_path_buf();
        assert!(
            !dir.join("vault.json").exists(),
            "未启用加密保存时不得写出任何密文库"
        );
    }

    #[tokio::test]
    async fn nonce_is_fresh_on_every_write() {
        let state = temp_state("nonce");
        state
            .insert(vec![StoredKey::new(
                "sk-nonce-check".to_string(),
                sample_entry("id-1", "sk-no***heck"),
            )])
            .await;
        state.enable("correct horse battery").await.unwrap();

        let path = state.paths().unwrap().vault.clone();
        let first = std::fs::read_to_string(&path).unwrap();
        let first: VaultFile = serde_json::from_str(&first).unwrap();

        state.persist_if_enabled().await.unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        let second: VaultFile = serde_json::from_str(&second).unwrap();

        assert_ne!(first.nonce, second.nonce, "每次加密都必须换新 nonce");
        assert_ne!(first.salt, second.salt, "每次加密都应换新盐");
    }

    #[tokio::test]
    async fn locked_vault_rejects_writes() {
        let state = temp_state("locked");
        state
            .insert(vec![StoredKey::new(
                "sk-locked-key".to_string(),
                sample_entry("id-1", "sk-lo***-key"),
            )])
            .await;
        state.enable("correct horse battery").await.unwrap();

        // 重新打开（模拟重启）：启用但未解锁
        let reopened = KeyButlerState::new();
        reopened.configure(&state.paths().unwrap().vault.parent().unwrap().parent().unwrap());
        let status = reopened.status().await;
        assert!(status.enabled && !status.unlocked);
        assert!(reopened.ensure_writable().await.is_err());
        assert!(reopened.entries().await.is_empty());
    }

    #[tokio::test]
    async fn disable_removes_ciphertext_and_keeps_memory() {
        let state = temp_state("disable");
        state
            .insert(vec![StoredKey::new(
                "sk-disable-me".to_string(),
                sample_entry("id-1", "sk-di***e-me"),
            )])
            .await;
        state.enable("correct horse battery").await.unwrap();
        let path = state.paths().unwrap().vault.clone();
        assert!(path.exists());

        let status = state.disable().await.unwrap();
        assert!(!status.enabled);
        assert!(!path.exists(), "关闭后密文文件应被删除");
        // 内存表保留，用户还在用
        assert_eq!(state.entries().await.len(), 1);
    }

    fn sample_snapshot(key_id: &str, ts: u64) -> Snapshot {
        Snapshot {
            key_id: key_id.to_string(),
            ts,
            balance: Some(1.5),
            status: "active".to_string(),
        }
    }

    #[tokio::test]
    async fn snapshots_stay_in_memory_when_vault_disabled() {
        // 「默认不持久化」红线：未启用加密保存时，快照也不能写盘
        let state = temp_state("snap-nodisk");
        state
            .append_snapshot(sample_snapshot("k1", 1))
            .await
            .unwrap();

        assert_eq!(state.list_snapshots("k1").await.len(), 1, "内存里应当有");
        let snap_path = state.paths().unwrap().snapshots.clone();
        assert!(
            !snap_path.exists(),
            "未启用加密保存时不得写出快照文件：{}",
            snap_path.display()
        );
    }

    #[tokio::test]
    async fn snapshots_are_persisted_once_vault_enabled() {
        let state = temp_state("snap-disk");
        state
            .insert(vec![StoredKey::new(
                "sk-snapshot-check".to_string(),
                sample_entry("id-1", "sk-sn***heck"),
            )])
            .await;
        state.enable("correct horse battery").await.unwrap();

        state
            .append_snapshot(sample_snapshot("id-1", 5))
            .await
            .unwrap();

        let snap_path = state.paths().unwrap().snapshots.clone();
        assert!(snap_path.exists(), "启用后应当写出快照文件");
        assert_eq!(state.list_snapshots("id-1").await.len(), 1);
    }

    #[test]
    fn short_password_is_rejected() {        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let state = temp_state("shortpw");
        let result = runtime.block_on(state.enable("123"));
        assert!(result.is_err());
    }
}
