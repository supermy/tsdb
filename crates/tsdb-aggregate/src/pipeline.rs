//! # 轻度汇总管道 — 写入时自动触发异步聚合
//!
//! ## 数据流
//!
//! ```text
//! DataPoint 写入 StorageEngine
//!     │
//!     ▼ (触发回调)
//! LightAggregationPipeline::on_write()
//!     │
//!     ▼ (内存缓冲)
//! Aggregator::accumulate()
//!     │
//!     │ 缓冲区满 / 定时器到期 ?
//!     ▼
//! Aggregator::finalize() → AggregationStore::write_batch()
//! ```

use crate::aggregator::{Aggregator, TimeDimension, AggregationResult};
use crate::store::AggregationStoreManager;
use tsdb_types::model::DataPoint;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{info, debug};

/// 轻度汇总管道配置
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// 缓冲区最大数据点数量（达到后自动 flush）
    pub buffer_size: usize,
    /// 定时 flush 间隔（秒）
    pub flush_interval_secs: u64,
    /// 需要计算的维度列表
    pub dimensions: Vec<TimeDimension>,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            buffer_size: 10000,
            flush_interval_secs: 60,
            dimensions: vec![TimeDimension::Hour, TimeDimension::Day, TimeDimension::Week, TimeDimension::Month],
        }
    }
}

/// 轻度汇总管道 — 在数据写入时自动触发聚合计算
///
/// 核心职责：
/// 1. 接收写入的 DataPoint 并缓冲
/// 2. 当缓冲区满或定时器到期时触发聚合
/// 3. 将聚合结果持久化到 AggregationStore
pub struct LightAggregationPipeline {
    /// 管道配置
    config: PipelineConfig,
    /// 聚合存储管理器
    store_manager: Arc<AggregationStoreManager>,
    /// 按 (business, measurement) 分桶的聚合器
    aggregators: Mutex<HashMap<String, Aggregator>>,
    /// 各桶的缓冲计数
    buffer_counts: Mutex<HashMap<String, usize>>,
    /// 上次 flush 时间
    last_flush: Mutex<Instant>,
}

impl LightAggregationPipeline {
    /// 创建新的轻度汇总管道
    pub fn new(config: PipelineConfig, store_manager: Arc<AggregationStoreManager>) -> Self {
        Self {
            config,
            store_manager,
            aggregators: Mutex::new(HashMap::new()),
            buffer_counts: Mutex::new(HashMap::new()),
            last_flush: Mutex::new(Instant::now()),
        }
    }

    /// 数据点写入回调 — 在 StorageEngine.write() 成功后调用
    ///
    /// 将数据点缓冲到对应 (business, measurement) 的聚合器中，
    /// 并在缓冲区满时自动触发 flush。
    pub fn on_write(&self, business: &str, dp: &DataPoint) {
        let bucket_key = format!("{}:{}", business, dp.measurement);

        {
            let mut aggregators = self.aggregators.lock().unwrap();
            aggregators
                .entry(bucket_key.clone())
                .or_insert_with(Aggregator::new)
                .accumulate(dp);
        }

        let should_flush = {
            let mut counts = self.buffer_counts.lock().unwrap();
            let count = counts.entry(bucket_key.clone()).or_insert(0);
            *count += 1;
            *count >= self.config.buffer_size
        };

        if should_flush {
            if let Err(e) = self.flush_bucket(&bucket_key, business) {
                debug!("flush bucket '{}' failed: {}", bucket_key, e);
            }
        }

        let should_timer_flush = {
            let mut last = self.last_flush.lock().unwrap();
            if last.elapsed() >= Duration::from_secs(self.config.flush_interval_secs) {
                *last = Instant::now();
                true
            } else {
                false
            }
        };

        if should_timer_flush {
            if let Err(e) = self.flush_all() {
                debug!("timer flush failed: {}", e);
            }
        }
    }

    /// 刷新指定桶的聚合结果到存储
    fn flush_bucket(&self, bucket_key: &str, business: &str) -> Result<(), String> {
        let mut aggregators = self.aggregators.lock().unwrap();
        let mut counts = self.buffer_counts.lock().unwrap();

        if let Some(mut aggregator) = aggregators.remove(bucket_key) {
            counts.remove(bucket_key);

            let store = self.store_manager.get_store(business)?;

            for &dim in &self.config.dimensions {
                let results = aggregator.finalize(
                    bucket_key.split(':').nth(1).unwrap_or("unknown"),
                    dim,
                );
                if !results.is_empty() {
                    store.write_batch(&results)?;
                    debug!("flushed {} {} results for {}", results.len(), dim.name(), bucket_key);
                }
            }

            aggregator.reset();
        }

        Ok(())
    }

    /// 刷新所有桶的聚合结果
    pub fn flush_all(&self) -> Result<(), String> {
        let bucket_keys: Vec<String> = self.aggregators.lock().unwrap().keys().cloned().collect();

        for key in &bucket_keys {
            let business = key.split(':').next().unwrap_or("default").to_string();
            self.flush_bucket(key, &business)?;
        }

        info!("flushed all aggregation buckets");
        Ok(())
    }

    /// 返回当前缓冲的数据点总数
    pub fn buffered_count(&self) -> usize {
        self.buffer_counts.lock().unwrap().values().sum()
    }

    /// 返回当前活跃的桶数量
    pub fn bucket_count(&self) -> usize {
        self.aggregators.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_accumulate_and_flush() {
        let dir = tempfile::TempDir::new().unwrap();
        let store_mgr = Arc::new(AggregationStoreManager::new(dir.path().to_path_buf()));
        let config = PipelineConfig {
            buffer_size: 5,
            flush_interval_secs: 3600,
            dimensions: vec![TimeDimension::Day],
        };
        let pipeline = LightAggregationPipeline::new(config, store_mgr);

        for i in 0..6 {
            let mut dp = DataPoint::new("cpu", (1713158400 + i * 30) * 1_000_000);
            dp.fields.insert("usage".to_string(), tsdb_types::model::FieldValue::Float(50.0 + i as f64));
            pipeline.on_write("test", &dp);
        }

        assert!(pipeline.buffered_count() <= 1);
    }

    #[test]
    fn test_pipeline_flush_all() {
        let dir = tempfile::TempDir::new().unwrap();
        let store_mgr = Arc::new(AggregationStoreManager::new(dir.path().to_path_buf()));
        let config = PipelineConfig {
            buffer_size: 10000,
            flush_interval_secs: 3600,
            dimensions: vec![TimeDimension::Day],
        };
        let pipeline = LightAggregationPipeline::new(config, store_mgr);

        let mut dp = DataPoint::new("mem", 1713158400_000000);
        dp.fields.insert("used".to_string(), tsdb_types::model::FieldValue::Float(60.0));
        pipeline.on_write("iot", &dp);

        assert_eq!(pipeline.bucket_count(), 1);
        pipeline.flush_all().unwrap();
        assert_eq!(pipeline.bucket_count(), 0);
    }
}
