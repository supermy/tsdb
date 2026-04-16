//! 批量导入模块 - Bulk Import via SST Files
//!
//! 利用 RocksDB 的 SstFileWriter + IngestExternalFile 实现高速批量导入。
//! 适用于 TSBS 数据加载、历史数据回填等场景。
//!
//! ## 工作流程
//!
//! 1. 创建 SstFileWriter，写入外部 SST 文件
//! 2. 调用 DB::ingest_external_file() 将 SST 文件导入数据库
//! 3. 导入的文件跳过 MemTable 和 WAL，直接成为 L0 层 SST 文件
//!
//! ## 性能优势
//!
//! - 跳过 WAL 写入，减少 I/O
//! - 跳过 MemTable 排序，直接写入有序 SST
//! - 支持并行生成多个 SST 文件后批量导入
//! - 导入期间数据库仍可正常读写

use rocksdb::{SstFileWriter, Options, IngestExternalFileOptions, DB};

pub struct BulkImporter<'a> {
    writer: SstFileWriter<'a>,
    sst_path: String,
    key_count: usize,
    finished: bool,
}

impl<'a> BulkImporter<'a> {
    pub fn new(opts: &'a Options, sst_path: &str) -> Self {
        let writer = SstFileWriter::create(opts);
        writer.open(sst_path).expect("Failed to open SST file for writing");
        BulkImporter {
            writer,
            sst_path: sst_path.to_string(),
            key_count: 0,
            finished: false,
        }
    }

    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), String> {
        self.writer.put(key, value).map_err(|e| format!("SST put failed: {}", e))?;
        self.key_count += 1;
        Ok(())
    }

    pub fn merge(&mut self, key: &[u8], value: &[u8]) -> Result<(), String> {
        self.writer.merge(key, value).map_err(|e| format!("SST merge failed: {}", e))?;
        self.key_count += 1;
        Ok(())
    }

    pub fn finish(&mut self) -> Result<usize, String> {
        if self.finished {
            return Ok(self.key_count);
        }
        self.writer.finish().map_err(|e| format!("SST finish failed: {}", e))?;
        self.finished = true;
        Ok(self.key_count)
    }

    pub fn count(&self) -> usize {
        self.key_count
    }

    pub fn sst_path(&self) -> &str {
        &self.sst_path
    }
}

pub fn ingest_sst_files(
    db: &DB,
    cf_name: Option<&str>,
    sst_paths: &[String],
    move_files: bool,
) -> Result<(), String> {
    let mut opts = IngestExternalFileOptions::default();
    opts.set_move_files(move_files);
    opts.set_snapshot_consistency(true);
    opts.set_allow_blocking_flush(false);
    opts.set_allow_global_seqno(true);

    let paths: Vec<&str> = sst_paths.iter().map(|s| s.as_str()).collect();

    if let Some(cf) = cf_name {
        let cf_handle = db.cf_handle(cf)
            .ok_or_else(|| format!("Column family '{}' not found", cf))?;
        db.ingest_external_file_cf_opts(&cf_handle, &opts, paths)
            .map_err(|e| format!("Ingest external file failed: {}", e))?;
    } else {
        db.ingest_external_file_opts(&opts, paths)
            .map_err(|e| format!("Ingest external file failed: {}", e))?;
    }

    Ok(())
}

pub fn bulk_load_sorted(
    db: &DB,
    opts: &Options,
    cf_name: Option<&str>,
    sst_dir: &str,
    records: &[(Vec<u8>, Vec<u8>)],
    batch_size: usize,
) -> Result<usize, String> {
    if records.is_empty() {
        return Ok(0);
    }

    let mut total_ingested = 0;
    let mut sst_paths = Vec::new();

    for (batch_idx, chunk) in records.chunks(batch_size).enumerate() {
        let sst_path = format!("{}/bulk_{}.sst", sst_dir, batch_idx);
        let mut importer = BulkImporter::new(opts, &sst_path);

        for (key, value) in chunk {
            importer.put(key, value)?;
        }

        let count = importer.finish()?;
        total_ingested += count;
        sst_paths.push(sst_path);
    }

    ingest_sst_files(db, cf_name, &sst_paths, true)?;

    for path in &sst_paths {
        let _ = std::fs::remove_file(path);
    }

    Ok(total_ingested)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::options::TsdbOptions;
    use tempfile::TempDir;

    #[test]
    fn test_bulk_import_basic() {
        let dir = TempDir::new().unwrap();
        let sst_dir = TempDir::new().unwrap();
        let opts = TsdbOptions::default_opts();
        let db = DB::open(&opts, dir.path()).unwrap();

        let sst_path = format!("{}/test.sst", sst_dir.path().display());
        let mut importer = BulkImporter::new(&opts, &sst_path);

        importer.put(b"key1", b"value1").unwrap();
        importer.put(b"key2", b"value2").unwrap();
        importer.put(b"key3", b"value3").unwrap();
        let count = importer.finish().unwrap();
        assert_eq!(count, 3);

        let sst_paths = vec![sst_path];
        ingest_sst_files(&db, None, &sst_paths, true).unwrap();

        assert_eq!(db.get(b"key1").unwrap().unwrap().as_slice(), b"value1");
        assert_eq!(db.get(b"key2").unwrap().unwrap().as_slice(), b"value2");
        assert_eq!(db.get(b"key3").unwrap().unwrap().as_slice(), b"value3");
    }

    #[test]
    fn test_bulk_load_sorted() {
        let dir = TempDir::new().unwrap();
        let sst_dir = TempDir::new().unwrap();
        let opts = TsdbOptions::default_opts();
        let db = DB::open(&opts, dir.path()).unwrap();

        let records: Vec<(Vec<u8>, Vec<u8>)> = (0..100)
            .map(|i| {
                let key = format!("key_{:04}", i);
                let value = format!("value_{}", i);
                (key.into_bytes(), value.into_bytes())
            })
            .collect();

        let count = bulk_load_sorted(
            &db,
            &opts,
            None,
            sst_dir.path().to_str().unwrap(),
            &records,
            50,
        ).unwrap();

        assert_eq!(count, 100);

        for i in 0..100 {
            let key = format!("key_{:04}", i);
            let value = db.get(key.as_bytes()).unwrap().unwrap();
            let expected = format!("value_{}", i);
            assert_eq!(value.as_slice(), expected.as_bytes());
        }
    }
}
