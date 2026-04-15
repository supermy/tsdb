use crate::error::{Result, TsdbError};
use crate::rowkey::{RowKey, Qualifier, timestamp_to_cf_name, compute_tags_hash, align_to_block_start, SEPARATOR};
use crate::storage::cf_manager::{CfManager, CfConfig, METADATA_CF};
use crate::storage::merge_operand::{MergedBlock, MergedField, encode_merge_operand, detect_value_format, ValueFormat};
use crate::storage::options::TsdbOptions;
use tsdb_types::model::{DataPoint, FieldValue, Tags};
use rocksdb::{WriteBatch, MultiThreaded};
use std::path::Path;
use std::sync::Arc;
use chrono::NaiveDate;

type TsdbDB = rocksdb::DBWithThreadMode<MultiThreaded>;

pub struct StorageEngine {
    db: Arc<TsdbDB>,
    cf_manager: CfManager,
}

impl StorageEngine {
    pub fn open(path: &Path, cf_config: CfConfig) -> Result<Self> {
        let opts = TsdbOptions::default_opts();

        let metadata_cf_opts = TsdbOptions::metadata_cf_opts();
        let cfs = vec![(
            METADATA_CF,
            metadata_cf_opts,
        )];

        let db = TsdbDB::open_cf_descriptors(&opts, path, cfs.into_iter().map(|(name, opts)| {
            rocksdb::ColumnFamilyDescriptor::new(name, opts)
        })).map_err(|e| TsdbError::Storage(format!("failed to open DB: {}", e)))?;

        let db = Arc::new(db);
        let cf_manager = CfManager::new(Arc::clone(&db), cf_config);

        Ok(Self { db, cf_manager })
    }

    pub fn write(&self, dp: &DataPoint) -> Result<()> {
        let row_key = RowKey::from_data_point(dp);
        let block_start = row_key.block_start_timestamp;
        let cf_name = timestamp_to_cf_name(dp.timestamp);

        let date = micros_to_date(dp.timestamp);
        self.cf_manager.ensure_cf_for_date(date)?;

        let cf = self.cf_manager.cf_handle(&cf_name)?;
        let rk_bytes = row_key.encode();

        for (field_name, field_value) in &dp.fields {
            let qualifier = Qualifier::new(field_name, dp.timestamp, block_start);
            let mut key_bytes = rk_bytes.clone();
            key_bytes.push(0u8);
            key_bytes.extend_from_slice(&qualifier.encode());
            let value_bytes = encode_field_value(field_value);

            self.db.put_cf(&cf, &key_bytes, &value_bytes)
                .map_err(|e| TsdbError::Storage(format!("write failed: {}", e)))?;
        }

        Ok(())
    }

    pub fn write_batch(&self, data_points: &[DataPoint]) -> Result<()> {
        let mut batch = WriteBatch::default();

        for dp in data_points {
            let row_key = RowKey::from_data_point(dp);
            let block_start = row_key.block_start_timestamp;
            let cf_name = timestamp_to_cf_name(dp.timestamp);

            let date = micros_to_date(dp.timestamp);
            self.cf_manager.ensure_cf_for_date(date)?;

            let cf = self.cf_manager.cf_handle(&cf_name)?;
            let rk_bytes = row_key.encode();

            for (field_name, field_value) in &dp.fields {
                let qualifier = Qualifier::new(field_name, dp.timestamp, block_start);
                let mut key_bytes = rk_bytes.clone();
                key_bytes.push(0u8);
                key_bytes.extend_from_slice(&qualifier.encode());
                let value_bytes = encode_field_value(field_value);

                batch.put_cf(&cf, &key_bytes, &value_bytes);
            }
        }

        self.db.write(batch)
            .map_err(|e| TsdbError::Storage(format!("batch write failed: {}", e)))?;

        Ok(())
    }

    pub fn write_merged(&self, dp: &DataPoint) -> Result<()> {
        let row_key = RowKey::from_data_point(dp);
        let block_start = row_key.block_start_timestamp;
        let cf_name = timestamp_to_cf_name(dp.timestamp);

        let date = micros_to_date(dp.timestamp);
        self.cf_manager.ensure_cf_for_date(date)?;

        let cf = self.cf_manager.cf_handle(&cf_name)?;
        let rk_bytes = row_key.encode();

        for (field_name, field_value) in &dp.fields {
            let qualifier = Qualifier::new(field_name, dp.timestamp, block_start);
            let operand = encode_merge_operand(
                field_name,
                qualifier.microsecond_offset,
                field_value,
            );

            self.db.merge_cf(&cf, &rk_bytes, operand)
                .map_err(|e| TsdbError::Storage(format!("merge failed: {}", e)))?;
        }

        Ok(())
    }

    pub fn write_merged_batch(&self, data_points: &[DataPoint]) -> Result<()> {
        let mut batch = WriteBatch::default();

        for dp in data_points {
            let row_key = RowKey::from_data_point(dp);
            let block_start = row_key.block_start_timestamp;
            let cf_name = timestamp_to_cf_name(dp.timestamp);

            let date = micros_to_date(dp.timestamp);
            self.cf_manager.ensure_cf_for_date(date)?;

            let cf = self.cf_manager.cf_handle(&cf_name)?;
            let rk_bytes = row_key.encode();

            for (field_name, field_value) in &dp.fields {
                let qualifier = Qualifier::new(field_name, dp.timestamp, block_start);
                let operand = encode_merge_operand(
                    field_name,
                    qualifier.microsecond_offset,
                    field_value,
                );
                batch.merge_cf(&cf, &rk_bytes, operand);
            }
        }

        self.db.write(batch)
            .map_err(|e| TsdbError::Storage(format!("merged batch write failed: {}", e)))?;

        Ok(())
    }

    pub fn read_range(
        &self,
        measurement: &str,
        tags: &Tags,
        start_micros: i64,
        end_micros: i64,
    ) -> Result<Vec<DataPoint>> {
        let tags_hash = compute_tags_hash(tags);
        let mut results = Vec::new();

        let start_date = micros_to_date(start_micros);
        let end_date = micros_to_date(end_micros);

        let mut current_date = start_date;
        while current_date <= end_date {
            let cf_name = self.cf_manager.get_cf_name(current_date);
            if let Ok(cf) = self.cf_manager.cf_handle(&cf_name) {
                let prefix_key = {
                    let mut buf = measurement.as_bytes().to_vec();
                    buf.push(SEPARATOR);
                    buf.extend_from_slice(&tags_hash.to_be_bytes());
                    buf.push(SEPARATOR);
                    buf
                };

                let iter = self.db.prefix_iterator_cf(&cf, &prefix_key);
                for item in iter {
                    let (key, value) = match item {
                        Ok(kv) => kv,
                        Err(_) => break,
                    };
                    if !key.starts_with(&prefix_key) {
                        break;
                    }

                    match detect_value_format(&value) {
                        ValueFormat::Merged => {
                            let sep_pos = match key.iter().rposition(|&b| b == 0) {
                                Some(pos) => pos,
                                None => {
                                    if let Some(rk) = RowKey::decode(&key) {
                                        if let Some(block) = MergedBlock::decode(&value) {
                                            let dps = block.to_data_points(&rk.measurement, rk.block_start_timestamp, tags.clone());
                                            for dp in dps {
                                                if dp.timestamp >= start_micros && dp.timestamp <= end_micros {
                                                    results.push(dp);
                                                }
                                            }
                                        }
                                    }
                                    continue;
                                }
                            };

                            let rk_data = &key[..sep_pos];
                            if let Some(rk) = RowKey::decode(rk_data) {
                                if let Some(block) = MergedBlock::decode(&value) {
                                    let dps = block.to_data_points(&rk.measurement, rk.block_start_timestamp, tags.clone());
                                    for dp in dps {
                                        if dp.timestamp >= start_micros && dp.timestamp <= end_micros {
                                            results.push(dp);
                                        }
                                    }
                                }
                            }
                        }
                        ValueFormat::Raw => {
                            let sep_pos = match key.iter().rposition(|&b| b == 0) {
                                Some(pos) => pos,
                                None => continue,
                            };

                            let rk_data = &key[..sep_pos];
                            let qual_data = &key[sep_pos + 1..];

                            if let Some(rk) = RowKey::decode(rk_data) {
                                if let Some(qual) = Qualifier::decode(qual_data) {
                                    let ts = rk.block_start_timestamp + qual.microsecond_offset as i64;
                                    if ts >= start_micros && ts <= end_micros {
                                        let fv = decode_field_value(&value);
                                        let mut dp = DataPoint::new(&rk.measurement, ts);
                                        dp.tags = tags.clone();
                                        if let Some(v) = fv {
                                            dp.fields.insert(qual.field_name, v);
                                        }
                                        results.push(dp);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            current_date += chrono::Duration::days(1);
        }

        results.sort_by_key(|dp| dp.timestamp);
        Ok(results)
    }

    pub fn get_point_merged(
        &self,
        measurement: &str,
        tags: &Tags,
        timestamp: i64,
    ) -> Result<Option<DataPoint>> {
        let tags_hash = compute_tags_hash(tags);
        let block_start = align_to_block_start(timestamp);
        let cf_name = timestamp_to_cf_name(timestamp);

        let cf = self.cf_manager.cf_handle(&cf_name)?;

        let key = {
            let mut buf = measurement.as_bytes().to_vec();
            buf.push(SEPARATOR);
            buf.extend_from_slice(&tags_hash.to_be_bytes());
            buf.push(SEPARATOR);
            buf.extend_from_slice(&block_start.to_be_bytes());
            buf
        };

        match self.db.get_cf(&cf, &key) {
            Ok(Some(value)) => {
                if let Some(block) = MergedBlock::decode(&value) {
                    Ok(block.get_data_point_at(measurement, block_start, timestamp, tags.clone()))
                } else {
                    Ok(None)
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(TsdbError::Storage(format!("get failed: {}", e))),
        }
    }

    pub fn cleanup(&self) -> Result<Vec<String>> {
        self.cf_manager.cleanup_expired_cfs()
    }

    pub fn db(&self) -> Arc<TsdbDB> {
        Arc::clone(&self.db)
    }

    pub fn cf_manager(&self) -> &CfManager {
        &self.cf_manager
    }
}

fn micros_to_date(micros: i64) -> NaiveDate {
    let secs = micros / 1_000_000;
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| dt.date_naive())
        .unwrap_or_else(|| chrono::Local::now().date_naive())
}

fn encode_field_value(v: &FieldValue) -> Vec<u8> {
    match v {
        FieldValue::Float(f) => {
            let mut buf = vec![0u8];
            buf.extend_from_slice(&f.to_be_bytes());
            buf
        }
        FieldValue::Integer(i) => {
            let mut buf = vec![1u8];
            buf.extend_from_slice(&i.to_be_bytes());
            buf
        }
        FieldValue::String(s) => {
            let mut buf = vec![2u8];
            buf.extend_from_slice(&(s.len() as u32).to_be_bytes());
            buf.extend_from_slice(s.as_bytes());
            buf
        }
        FieldValue::Boolean(b) => {
            vec![3u8, if *b { 1 } else { 0 }]
        }
    }
}

fn decode_field_value(data: &[u8]) -> Option<FieldValue> {
    if data.is_empty() {
        return None;
    }
    match data[0] {
        0 => {
            let f = f64::from_be_bytes(data[1..9].try_into().ok()?);
            Some(FieldValue::Float(f))
        }
        1 => {
            let i = i64::from_be_bytes(data[1..9].try_into().ok()?);
            Some(FieldValue::Integer(i))
        }
        2 => {
            let len = u32::from_be_bytes(data[1..5].try_into().ok()?) as usize;
            let s = String::from_utf8_lossy(&data[5..5 + len]).to_string();
            Some(FieldValue::String(s))
        }
        3 => {
            Some(FieldValue::Boolean(data.get(1)? == &1))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsdb_types::model::FieldValue;

    #[test]
    fn test_field_value_encode_decode() {
        let cases = vec![
            FieldValue::Float(3.14),
            FieldValue::Integer(-42),
            FieldValue::String("hello".to_string()),
            FieldValue::Boolean(true),
        ];
        for fv in cases {
            let encoded = encode_field_value(&fv);
            let decoded = decode_field_value(&encoded).unwrap();
            assert_eq!(fv, decoded);
        }
    }
}
