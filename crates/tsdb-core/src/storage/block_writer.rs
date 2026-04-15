use crate::error::{Result, TsdbError};
use crate::rowkey::{RowKey, timestamp_to_cf_name, SEPARATOR};
use crate::storage::cf_manager::CfManager;
use crate::storage::merge_operand::{MergedBlock, MergedField};
use tsdb_types::model::DataPoint;
use rocksdb::MultiThreaded;
use std::collections::HashMap;
use std::sync::Arc;
use chrono::NaiveDate;

type TsdbDB = rocksdb::DBWithThreadMode<MultiThreaded>;

#[derive(Debug, Clone)]
pub struct BlockWriterConfig {
    pub max_block_rows: usize,
    pub flush_interval_ms: u64,
    pub compression_enabled: bool,
}

impl Default for BlockWriterConfig {
    fn default() -> Self {
        Self {
            max_block_rows: 1024,
            flush_interval_ms: 5000,
            compression_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct BlockKey {
    measurement: String,
    tags_hash: u64,
    block_start_ts: i64,
}

pub struct BlockWriter {
    db: Arc<TsdbDB>,
    cf_manager: CfManager,
    config: BlockWriterConfig,
    buffers: HashMap<BlockKey, MergedBlock>,
}

impl BlockWriter {
    pub fn new(db: Arc<TsdbDB>, cf_manager: CfManager, config: BlockWriterConfig) -> Self {
        Self { db, cf_manager, config, buffers: HashMap::new() }
    }

    pub fn write(&mut self, dp: &DataPoint) -> Result<bool> {
        let row_key = RowKey::from_data_point(dp);
        let block_start = row_key.block_start_timestamp;
        let bk = BlockKey {
            measurement: row_key.measurement.clone(),
            tags_hash: row_key.tags_hash,
            block_start_ts: block_start,
        };

        let qualifier_offset = (dp.timestamp - block_start) as u32;

        let block = self.buffers.entry(bk.clone()).or_default();
        for (field_name, field_value) in &dp.fields {
            block.upsert_field(MergedField {
                name: field_name.clone(),
                micro_offset: qualifier_offset,
                value: field_value.clone(),
            });
        }

        if block.fields.len() >= self.config.max_block_rows {
            let block = self.buffers.remove(&bk).unwrap();
            self.flush_block(&bk, &block)?;
            return Ok(true);
        }

        Ok(false)
    }

    pub fn flush_all(&mut self) -> Result<usize> {
        let blocks: Vec<(BlockKey, MergedBlock)> = self.buffers.drain().collect();
        let count = blocks.len();
        for (bk, block) in blocks {
            self.flush_block(&bk, &block)?;
        }
        Ok(count)
    }

    fn flush_block(&self, bk: &BlockKey, block: &MergedBlock) -> Result<()> {
        let cf_name = timestamp_to_cf_name(bk.block_start_ts);
        let date = micros_to_date(bk.block_start_ts);
        self.cf_manager.ensure_cf_for_date(date)?;

        let cf = self.cf_manager.cf_handle(&cf_name)?;

        let key = {
            let mut buf = bk.measurement.as_bytes().to_vec();
            buf.push(SEPARATOR);
            buf.extend_from_slice(&bk.tags_hash.to_be_bytes());
            buf.push(SEPARATOR);
            buf.extend_from_slice(&bk.block_start_ts.to_be_bytes());
            buf
        };

        let value = block.encode();

        self.db.put_cf(&cf, &key, &value)
            .map_err(|e| TsdbError::Storage(format!("flush block failed: {}", e)))?;

        Ok(())
    }

    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }
}

fn micros_to_date(micros: i64) -> NaiveDate {
    let secs = micros / 1_000_000;
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| dt.date_naive())
        .unwrap_or_else(|| chrono::Local::now().date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_writer_config_default() {
        let config = BlockWriterConfig::default();
        assert_eq!(config.max_block_rows, 1024);
        assert!(config.compression_enabled);
    }
}
