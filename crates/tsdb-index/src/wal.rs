//! # 索引 WAL (Write-Ahead Log) — 崩溃恢复保障
//!
//! 在索引数据写入内存的同时，异步记录 WAL 日志。
//! 服务崩溃后重启时，先加载最新 Checkpoint，再回放 WAL 日志，
//! 保证索引数据不丢失。
//!
//! ## WAL 格式
//!
//! ```text
//! [Entry]
//! ├── entry_len: u32 LE    — 整条 Entry 的字节长度（不含自身）
//! ├── entry_type: u8       — 0=Insert, 1=Delete, 2=Checkpoint
//! ├── sequence: u64 LE     — 单调递增序列号
//! ├── payload_len: u32 LE  — payload 字节数
//! ├── payload: Vec<u8>     — 序列化的操作数据
//! └── crc32: u32 LE        — payload 的 CRC32 校验
//! ```

use std::io::{Write, Read, BufWriter, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::fs::{File, OpenOptions};

const ENTRY_INSERT: u8 = 0;
const ENTRY_DELETE: u8 = 1;
const ENTRY_CHECKPOINT: u8 = 2;

pub struct IndexWAL {
    path: PathBuf,
    writer: BufWriter<File>,
    sequence: AtomicU64,
    bytes_written: AtomicU64,
}

impl IndexWAL {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        let seq = Self::recover_max_sequence(path)?;

        Ok(Self {
            path: path.to_path_buf(),
            writer: BufWriter::new(file),
            sequence: AtomicU64::new(seq),
            bytes_written: AtomicU64::new(0),
        })
    }

    pub fn append_insert(&self, payload: &[u8]) -> std::io::Result<u64> {
        self.append_entry(ENTRY_INSERT, payload)
    }

    pub fn append_delete(&self, payload: &[u8]) -> std::io::Result<u64> {
        self.append_entry(ENTRY_DELETE, payload)
    }

    pub fn append_checkpoint(&self, payload: &[u8]) -> std::io::Result<u64> {
        self.append_entry(ENTRY_CHECKPOINT, payload)
    }

    fn append_entry(&self, entry_type: u8, payload: &[u8]) -> std::io::Result<u64> {
        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);
        let crc = crc32fast::hash(payload);

        let mut buf = Vec::with_capacity(1 + 8 + 4 + payload.len() + 4);
        buf.push(entry_type);
        buf.extend_from_slice(&seq.to_le_bytes());
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(payload);
        buf.extend_from_slice(&crc.to_le_bytes());

        let entry_len = buf.len() as u32;

        let mut writer = self.writer.get_ref().try_clone()?;
        writer.write_all(&entry_len.to_le_bytes())?;
        writer.write_all(&buf)?;
        writer.flush()?;

        self.bytes_written.fetch_add(4 + buf.len() as u64, Ordering::Relaxed);

        Ok(seq)
    }

    pub fn replay(path: &Path) -> std::io::Result<Vec<WALEntry>> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut entries = Vec::new();

        loop {
            let mut len_buf = [0u8; 4];
            match reader.read_exact(&mut len_buf) {
                Ok(()) => {},
                Err(_) => break,
            }
            let entry_len = u32::from_le_bytes(len_buf) as usize;
            if entry_len == 0 || entry_len > 16 * 1024 * 1024 {
                break;
            }

            let mut entry_buf = vec![0u8; entry_len];
            match reader.read_exact(&mut entry_buf) {
                Ok(()) => {},
                Err(_) => break,
            }

            if entry_buf.len() < 1 + 8 + 4 + 4 {
                break;
            }

            let entry_type = entry_buf[0];
            let sequence = u64::from_le_bytes(entry_buf[1..9].try_into().unwrap_or([0; 8]));
            let payload_len = u32::from_le_bytes(entry_buf[9..13].try_into().unwrap_or([0; 4])) as usize;

            if entry_buf.len() < 13 + payload_len + 4 {
                break;
            }

            let payload = entry_buf[13..13 + payload_len].to_vec();
            let stored_crc = u32::from_le_bytes(
                entry_buf[13 + payload_len..13 + payload_len + 4]
                    .try_into()
                    .unwrap_or([0; 4])
            );

            let computed_crc = crc32fast::hash(&payload);
            if stored_crc != computed_crc {
                break;
            }

            entries.push(WALEntry {
                entry_type,
                sequence,
                payload,
            });
        }

        Ok(entries)
    }

    fn recover_max_sequence(path: &Path) -> std::io::Result<u64> {
        let entries = Self::replay(path)?;
        Ok(entries.iter().map(|e| e.sequence).max().unwrap_or(0))
    }

    pub fn rotate(&self) -> std::io::Result<()> {
        let rotated_path = self.path.with_extension("wal.old");
        let _ = std::fs::rename(&self.path, &rotated_path);
        let _ = std::fs::remove_file(&rotated_path);
        Ok(())
    }

    pub fn current_sequence(&self) -> u64 {
        self.sequence.load(Ordering::SeqCst)
    }

    pub fn bytes_written(&self) -> u64 {
        self.bytes_written.load(Ordering::Relaxed)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug, Clone)]
pub struct WALEntry {
    pub entry_type: u8,
    pub sequence: u64,
    pub payload: Vec<u8>,
}

impl WALEntry {
    pub fn is_insert(&self) -> bool { self.entry_type == ENTRY_INSERT }
    pub fn is_delete(&self) -> bool { self.entry_type == ENTRY_DELETE }
    pub fn is_checkpoint(&self) -> bool { self.entry_type == ENTRY_CHECKPOINT }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_wal_append_and_replay() {
        let dir = TempDir::new().unwrap();
        let wal_path = dir.path().join("test.wal");

        let wal = IndexWAL::open(&wal_path).unwrap();
        wal.append_insert(b"cpu:12345:1000").unwrap();
        wal.append_insert(b"cpu:12345:2000").unwrap();
        wal.append_delete(b"cpu:12345:1000").unwrap();

        let entries = IndexWAL::replay(&wal_path).unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries[0].is_insert());
        assert!(entries[2].is_delete());
        assert_eq!(entries[0].payload, b"cpu:12345:1000");
    }

    #[test]
    fn test_wal_checkpoint() {
        let dir = TempDir::new().unwrap();
        let wal_path = dir.path().join("checkpoint.wal");

        let wal = IndexWAL::open(&wal_path).unwrap();
        wal.append_insert(b"data1").unwrap();
        wal.append_checkpoint(b"full_snapshot").unwrap();
        wal.append_insert(b"data2").unwrap();

        let entries = IndexWAL::replay(&wal_path).unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries[1].is_checkpoint());
    }

    #[test]
    fn test_wal_crc_validation() {
        let dir = TempDir::new().unwrap();
        let wal_path = dir.path().join("crc.wal");

        let wal = IndexWAL::open(&wal_path).unwrap();
        wal.append_insert(b"valid_data").unwrap();

        let mut data = std::fs::read(&wal_path).unwrap();
        let corrupt_pos = data.len() - 2;
        data[corrupt_pos] ^= 0xFF;
        std::fs::write(&wal_path, &data).unwrap();

        let entries = IndexWAL::replay(&wal_path).unwrap();
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_wal_sequence_monotonic() {
        let dir = TempDir::new().unwrap();
        let wal_path = dir.path().join("seq.wal");

        let wal = IndexWAL::open(&wal_path).unwrap();
        let s1 = wal.append_insert(b"a").unwrap();
        let s2 = wal.append_insert(b"b").unwrap();
        let s3 = wal.append_insert(b"c").unwrap();

        assert!(s1 < s2);
        assert!(s2 < s3);
    }
}
