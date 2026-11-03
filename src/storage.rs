//! 本地存储（redb 单文件数据库）：配对信任表 + 全局历史记录表。
//!
//! 记录主键 = 时间戳（毫秒）左移 24 位 + 序号，天然按时间排序；
//! 另有 id → 主键 的索引表用于按 ID 定位。

use std::path::{Path, PathBuf};

use redb::{Database, ReadableTable, TableDefinition};

use crate::{
    discovery::PairedDevice,
    error::{DriftError, Result},
    history::{Record, RecordId},
    identity::Fingerprint,
};

const TRUST: TableDefinition<&str, &[u8]> = TableDefinition::new("trust");
const RECORDS: TableDefinition<u128, &[u8]> = TableDefinition::new("records");
const RECORD_INDEX: TableDefinition<&str, u128> = TableDefinition::new("record_index");

pub struct Storage {
    db: Database,
    #[allow(dead_code)]
    dir: PathBuf,
}

impl Storage {
    pub fn open(config_dir: &Path) -> Result<Storage> {
        std::fs::create_dir_all(config_dir)
            .map_err(|e| DriftError::Dir(format!("创建数据目录失败: {e}")))?;
        let path = config_dir.join("drift.db");
        let db = Database::create(&path)?;
        // 预建表
        let w = db.begin_write()?;
        {
            let _ = w.open_table(TRUST)?;
            let _ = w.open_table(RECORDS)?;
            let _ = w.open_table(RECORD_INDEX)?;
        }
        w.commit()?;
        Ok(Storage { db, dir: config_dir.to_path_buf() })
    }

    // ---------- 信任（配对设备） ----------

    pub fn add_paired(&self, device: &PairedDevice) -> Result<()> {
        let key = device.fingerprint.to_hex();
        let value = serde_json::to_vec(device)?;
        let w = self.db.begin_write()?;
        {
            let mut t = w.open_table(TRUST)?;
            t.insert(key.as_str(), value.as_slice())?;
        }
        w.commit()?;
        Ok(())
    }

    pub fn remove_paired(&self, fp: &Fingerprint) -> Result<Option<PairedDevice>> {
        let key = fp.to_hex();
        let old = self.get_paired(fp)?;
        let w = self.db.begin_write()?;
        {
            let mut t = w.open_table(TRUST)?;
            t.remove(key.as_str())?;
        }
        w.commit()?;
        Ok(old)
    }

    pub fn get_paired(&self, fp: &Fingerprint) -> Result<Option<PairedDevice>> {
        let key = fp.to_hex();
        let ro = self.db.begin_read()?;
        let t = ro.open_table(TRUST)?;
        if let Some(guard) = t.get(key.as_str())? {
            let v: PairedDevice = serde_json::from_slice(guard.value())?;
            Ok(Some(v))
        } else {
            Ok(None)
        }
    }

    pub fn paired_list(&self) -> Result<Vec<PairedDevice>> {
        let ro = self.db.begin_read()?;
        let t = ro.open_table(TRUST)?;
        let mut out = Vec::new();
        for item in t.iter()? {
            let (_, guard) = item?;
            out.push(serde_json::from_slice(guard.value())?);
        }
        Ok(out)
    }

    // ---------- 记录 ----------

    fn record_key(record: &Record) -> u128 {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let ms = record.timestamp.timestamp_millis().max(0) as u128;
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as u128 & 0xFF_FFFF;
        (ms << 24) | seq
    }

    /// 新增记录（时间戳生成主键）
    pub fn put_record(&self, record: &Record) -> Result<()> {
        let key = Self::record_key(record);
        let value = serde_json::to_vec(record)?;
        let id = record.id.to_string();
        let w = self.db.begin_write()?;
        {
            let mut records = w.open_table(RECORDS)?;
            let mut index = w.open_table(RECORD_INDEX)?;
            records.insert(key, value.as_slice())?;
            index.insert(id.as_str(), key)?;
        }
        w.commit()?;
        Ok(())
    }

    /// 更新记录（保持原主键 / 原时间戳）
    pub fn update_record(&self, record: &Record) -> Result<()> {
        let id = record.id.to_string();
        let value = serde_json::to_vec(record)?;
        let ro = self.db.begin_read()?;
        let existing = ro.open_table(RECORD_INDEX)?.get(id.as_str())?.map(|g| g.value());
        drop(ro);
        let Some(key) = existing else {
            // 没有索引就当新纪录插入
            return self.put_record(record);
        };
        let w = self.db.begin_write()?;
        {
            let mut records = w.open_table(RECORDS)?;
            records.insert(key, value.as_slice())?;
        }
        w.commit()?;
        Ok(())
    }

    pub fn get_record(&self, id: &RecordId) -> Result<Option<Record>> {
        let id_str = id.to_string();
        let ro = self.db.begin_read()?;
        let index = ro.open_table(RECORD_INDEX)?;
        let Some(key) = index.get(id_str.as_str())?.map(|g| g.value()) else {
            return Ok(None);
        };
        let records = ro.open_table(RECORDS)?;
        if let Some(guard) = records.get(key)? {
            Ok(Some(serde_json::from_slice(guard.value())?))
        } else {
            Ok(None)
        }
    }

    /// 全部记录，新 → 旧
    pub fn all_records_desc(&self) -> Result<Vec<Record>> {
        let ro = self.db.begin_read()?;
        let t = ro.open_table(RECORDS)?;
        let mut out = Vec::new();
        for item in t.range(0u128..)? {
            let (_, guard) = item?;
            out.push(serde_json::from_slice(guard.value())?);
        }
        out.reverse();
        Ok(out)
    }

    /// 物理删除（软删除只改 Record.deleted 字段，走 update_record）
    pub fn hard_delete_record(&self, id: &RecordId) -> Result<()> {
        let id_str = id.to_string();
        let ro = self.db.begin_read()?;
        let existing = ro.open_table(RECORD_INDEX)?.get(id_str.as_str())?.map(|g| g.value());
        drop(ro);
        if let Some(key) = existing {
            let w = self.db.begin_write()?;
            {
                let mut records = w.open_table(RECORDS)?;
                let mut index = w.open_table(RECORD_INDEX)?;
                records.remove(key)?;
                index.remove(id_str.as_str())?;
            }
            w.commit()?;
        }
        Ok(())
    }

    /// 裁剪最旧记录到上限，返回删除数量
    pub fn prune_records(&self, keep: usize) -> Result<usize> {
        let all = self.all_records_desc()?;
        if all.len() <= keep {
            return Ok(0);
        }
        let removed = &all[keep..];
        for r in removed {
            self.hard_delete_record(&r.id)?;
        }
        Ok(removed.len())
    }
}
