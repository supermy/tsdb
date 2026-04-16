//! # 轻度汇总存储 — 聚合结果的独立持久化
//!
//! 每种业务一个汇总 DB，每个维度一个 CF，长期保存。
//! 聚合结果写入后按时间线性增加，不会修改，避免重复计算。

use crate::aggregator::{AggregationResult, TimeDimension};
use rocksdb::{DB, Options, WriteBatch};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::info;

/// 轻度汇总存储 — 将聚合结果持久化到独立的 RocksDB 实例
///
/// ## 存储结构
///
/// ```text
/// aggregation_data/<business>/
/// ├── hour (CF) — 按小时聚合的结果
/// ├── day  (CF) — 按天聚合的结果
/// ├── week (CF) — 按周聚合的结果
/// └── month (CF) — 按月聚合的结果
///
/// Key格式: <measurement>|<window_start_timestamp>
/// Value: JSON 编码的 { field_name: value, ... }
/// ```
pub struct AggregationStore {
    db: Arc<DB>,
    business: String,
}

impl AggregationStore {
    /// 打开或创建指定业务的聚合存储
    ///
    /// # 参数
    /// - `data_dir`: 聚合数据根目录
    /// - `business`: 业务名称（如 "stocks", "iot"）
    pub fn open(data_dir: &Path, business: &str) -> Result<Self, String> {
        let db_path = data_dir.join(business);
        std::fs::create_dir_all(&db_path).map_err(|e| format!("create dir failed: {}", e))?;

        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cfs = vec!["hour", "day", "week", "month"];
        let cf_descriptors: Vec<rocksdb::ColumnFamilyDescriptor> = cfs.iter()
            .map(|&name| {
                let mut cf_opts = Options::default();
                cf_opts.set_compression_type(rocksdb::DBCompressionType::Zstd);
                cf_opts.set_disable_auto_compactions(true);
                rocksdb::ColumnFamilyDescriptor::new(name, cf_opts)
            })
            .collect();

        let db = DB::open_cf_descriptors(&opts, &db_path, cf_descriptors)
            .map_err(|e| format!("open aggregation db failed: {}", e))?;

        info!("AggregationStore opened for '{}' at {:?}", business, db_path);
        Ok(Self {
            db: Arc::new(db),
            business: business.to_string(),
        })
    }

    /// 写入单条聚合结果
    pub fn write_result(&self, result: &AggregationResult) -> Result<(), String> {
        let cf_name = result.dimension.name();
        let cf = self.db.cf_handle(cf_name)
            .ok_or_else(|| format!("CF '{}' not found", cf_name))?;

        let key = format!("{}|{}", result.measurement, result.window_start);
        let value = serde_json::to_string(&result.values)
            .map_err(|e| format!("serialize failed: {}", e))?;

        self.db.put_cf(&cf, key.as_bytes(), value.as_bytes())
            .map_err(|e| format!("write failed: {}", e))?;

        Ok(())
    }

    /// 批量写入聚合结果
    pub fn write_batch(&self, results: &[AggregationResult]) -> Result<(), String> {
        let mut batch = WriteBatch::default();

        for result in results {
            let cf_name = result.dimension.name();
            if let Some(cf) = self.db.cf_handle(cf_name) {
                let key = format!("{}|{}", result.measurement, result.window_start);
                if let Ok(value) = serde_json::to_string(&result.values) {
                    batch.put_cf(&cf, key.as_bytes(), value.as_bytes());
                }
            }
        }

        self.db.write(batch)
            .map_err(|e| format!("batch write failed: {}", e))?;

        Ok(())
    }

    /// 查询指定维度和时间范围的聚合结果
    pub fn query(
        &self,
        dimension: TimeDimension,
        measurement: &str,
        start_ts: i64,
        end_ts: i64,
    ) -> Result<Vec<AggregationResult>, String> {
        let cf_name = dimension.name();
        let cf = self.db.cf_handle(cf_name)
            .ok_or_else(|| format!("CF '{}' not found", cf_name))?;

        let prefix = format!("{}|", measurement);
        let mut results = Vec::new();

        let iter = self.db.prefix_iterator_cf(&cf, prefix.as_bytes());
        for item in iter {
            let (key, value) = item.map_err(|e| format!("iterator error: {}", e))?;
            let key_str = String::from_utf8_lossy(&key);
            let parts: Vec<&str> = key_str.splitn(2, '|').collect();
            if parts.len() < 2 { continue; }

            let ts: i64 = parts[1].parse().unwrap_or(0);
            if ts < start_ts { continue; }
            if ts > end_ts { break; }

            let values: HashMap<String, f64> = serde_json::from_slice(&value)
                .unwrap_or_default();

            results.push(AggregationResult {
                measurement: measurement.to_string(),
                dimension,
                window_start: ts,
                values,
            });
        }

        Ok(results)
    }

    /// 返回业务名称
    pub fn business(&self) -> &str { &self.business }
}

/// 聚合存储管理器 — 管理多个业务的 AggregationStore
pub struct AggregationStoreManager {
    /// 聚合数据根目录
    data_dir: PathBuf,
    /// 业务名称 → AggregationStore 的映射
    stores: Mutex<HashMap<String, Arc<AggregationStore>>>,
}

impl AggregationStoreManager {
    /// 创建新的聚合存储管理器
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir, stores: Mutex::new(HashMap::new()) }
    }

    /// 获取或创建指定业务的聚合存储
    pub fn get_store(&self, business: &str) -> Result<Arc<AggregationStore>, String> {
        {
            let stores = self.stores.lock().unwrap();
            if let Some(store) = stores.get(business) {
                return Ok(Arc::clone(store));
            }
        }

        let store = AggregationStore::open(&self.data_dir, business)?;
        let store = Arc::new(store);

        self.stores.lock().unwrap().insert(business.to_string(), Arc::clone(&store));
        Ok(store)
    }

    /// 列出所有已打开的业务存储
    pub fn list_businesses(&self) -> Vec<String> {
        self.stores.lock().unwrap().keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregation_store_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = AggregationStore::open(dir.path(), "test_business").unwrap();

        let result = AggregationResult {
            measurement: "cpu".to_string(),
            dimension: TimeDimension::Hour,
            window_start: 1713158400_000000,
            values: {
                let mut m = HashMap::new();
                m.insert("usage".to_string(), 78.5);
                m
            },
        };

        store.write_result(&result).unwrap();

        let queried = store.query(TimeDimension::Hour, "cpu", 0, i64::MAX).unwrap();
        assert_eq!(queried.len(), 1);
        assert!((queried[0].values.get("usage").unwrap() - 78.5).abs() < 0.001);
    }

    #[test]
    fn test_aggregation_store_batch() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = AggregationStore::open(dir.path(), "batch_test").unwrap();

        let results: Vec<AggregationResult> = (0..5).map(|i| AggregationResult {
            measurement: "cpu".to_string(),
            dimension: TimeDimension::Day,
            window_start: (1713158400 + i * 86400) * 1_000_000,
            values: {
                let mut m = HashMap::new();
                m.insert("usage".to_string(), 50.0 + i as f64);
                m
            },
        }).collect();

        store.write_batch(&results).unwrap();

        let queried = store.query(TimeDimension::Day, "cpu", 0, i64::MAX).unwrap();
        assert_eq!(queried.len(), 5);
    }

    #[test]
    fn test_store_manager() {
        let dir = tempfile::TempDir::new().unwrap();
        let manager = AggregationStoreManager::new(dir.path().to_path_buf());

        let s1 = manager.get_store("stocks").unwrap();
        let s2 = manager.get_store("iot").unwrap();
        let s1_again = manager.get_store("stocks").unwrap();

        assert_eq!(manager.list_businesses().len(), 2);
    }
}
