//! Key 管家：余额历史快照。
//!
//! 每次检测完成后追加一条「时间 + 余额 + 状态」记录，供前端画折线图。
//!
//! 可明文存储的原因：快照只含 `keyId` 引用与数值，不含 Key 明文，
//! 与加密的 Key 本体分离存储；密文库处于锁定状态时历史曲线依然可读。
//!
//! 写入策略：整文件重写 + 临时文件 rename（原子替换）。
//! 快照量不大（每条 Key 上限 500 条），而 JSON Lines 追加写
//! 无法按 Key 裁剪。


use std::path::{Path, PathBuf};

use crate::key_butler::models::Snapshot;

/// 每条 Key 最多保留的快照条数（超出丢弃最旧的）。
pub const MAX_PER_KEY: usize = 500;

/// 快照存储。
#[derive(Debug, Clone)]
pub struct SnapshotStore {
    path: PathBuf,
}

impl SnapshotStore {
    /// 以给定文件路径构造。
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// 读取全部快照；文件不存在或损坏时返回空列表（不报错）。
    fn read_all(&self) -> Vec<Snapshot> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        match serde_json::from_str::<Vec<Snapshot>>(&text) {
            Ok(list) => list,
            Err(err) => {
                // 文件损坏不应让功能整体失败：记一条日志，按空列表处理
                eprintln!(
                    "[ModelScout] 余额快照文件解析失败（{}），本次按空处理：{}",
                    err,
                    self.path.display()
                );
                Vec::new()
            }
        }
    }

    /// 原子写入：先写同目录临时文件，再 rename 覆盖。
    fn write_all(&self, list: &[Snapshot]) -> Result<(), String> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|err| format!("无法创建快照目录 {}：{err}", dir.display()))?;
        }

        let text = serde_json::to_string_pretty(list)
            .map_err(|err| format!("快照序列化失败：{err}"))?;

        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text.as_bytes())
            .map_err(|err| format!("写入快照临时文件失败：{err}"))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|err| format!("替换快照文件失败：{err}"))?;
        Ok(())
    }

    /// 追加一条快照，并对该 Key 做条数裁剪。
    pub fn append(&self, snapshot: Snapshot) -> Result<(), String> {
        let mut list = self.read_all();
        list.push(snapshot);

        // 裁剪：只保留该 key 最新的 MAX_PER_KEY 条。
        // 先按 ts 升序排好，再从头删除多余的。
        let key_id = list
            .last()
            .map(|s| s.key_id.clone())
            .unwrap_or_default();

        let mut indices: Vec<usize> = list
            .iter()
            .enumerate()
            .filter(|(_, s)| s.key_id == key_id)
            .map(|(index, _)| index)
            .collect();

        if indices.len() > MAX_PER_KEY {
            indices.sort_by_key(|&index| list[index].ts);
            let drop_count = indices.len() - MAX_PER_KEY;
            let to_remove: std::collections::HashSet<usize> =
                indices.into_iter().take(drop_count).collect();
            let mut index = 0usize;
            list.retain(|_| {
                let keep = !to_remove.contains(&index);
                index += 1;
                keep
            });
        }

        self.write_all(&list)
    }

    /// 查询某条 Key 的全部快照（按时间升序）。
    pub fn list(&self, key_id: &str) -> Vec<Snapshot> {
        let mut list: Vec<Snapshot> = self
            .read_all()
            .into_iter()
            .filter(|s| s.key_id == key_id)
            .collect();
        list.sort_by_key(|s| s.ts);
        list
    }

    /// 删除某条 Key 的全部快照（Key 被删除时调用）。
    pub fn remove_key(&self, key_id: &str) -> Result<(), String> {
        let mut list = self.read_all();
        let before = list.len();
        list.retain(|s| s.key_id != key_id);
        if list.len() == before {
            return Ok(());
        }
        self.write_all(&list)
    }

    /// 快照文件路径（供排障展示）。
    pub fn path(&self) -> &Path {
        &self.path
    }
}

// ---- 单元测试 ----

#[cfg(test)]
mod tests {
    use super::*;

    /// 用进程内唯一目录，避免与其它测试相互干扰。
    fn temp_store(tag: &str) -> SnapshotStore {
        let dir = std::env::temp_dir().join(format!("modelscout-snap-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        SnapshotStore::new(dir.join("snapshots.json"))
    }

    fn snap(key_id: &str, ts: u64, balance: f64) -> Snapshot {
        Snapshot {
            key_id: key_id.to_string(),
            ts,
            balance: Some(balance),
            status: "active".to_string(),
        }
    }

    #[test]
    fn append_and_list_are_sorted_by_time() {
        let store = temp_store("basic");
        store.append(snap("a", 300, 3.0)).unwrap();
        store.append(snap("a", 100, 1.0)).unwrap();
        store.append(snap("a", 200, 2.0)).unwrap();
        store.append(snap("b", 999, 9.0)).unwrap();

        let list = store.list("a");
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].ts, 100);
        assert_eq!(list[2].ts, 300);

        // 只返回该 key 的快照
        assert_eq!(store.list("b").len(), 1);
        assert_eq!(store.list("missing").len(), 0);
    }

    #[test]
    fn remove_key_drops_only_that_key() {
        let store = temp_store("remove");
        store.append(snap("a", 1, 1.0)).unwrap();
        store.append(snap("b", 2, 2.0)).unwrap();
        store.remove_key("a").unwrap();
        assert!(store.list("a").is_empty());
        assert_eq!(store.list("b").len(), 1);
    }

    #[test]
    fn missing_file_yields_empty_list_without_panic() {
        let store = temp_store("missing");
        assert!(store.list("anything").is_empty());
    }
}
