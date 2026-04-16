//! # 块写入器（Block Writer）— 缓冲式批量写入
//!
//! ## 设计目标
//!
//! 将零散的数据点（DataPoint）聚合成 **MergedBlock** 后批量刷入 RocksDB，
//! 减少对底层存储的 I/O 调用次数，提升写入吞吐量。
//!
//! ## 写入流程
//!
//! ```text
//! DataPoint 流入
//!      │
//!      ▼
//! ┌─────────────┐
//! │  RowKey 计算  │  ← measurement + tags_hash + block_start_ts
//! └──────┬──────┘
//!        │
//!        ▼
//! ┌──────────────────────┐
//! │   内存缓冲区 (HashMap)  │  ← BlockKey → MergedBlock 映射
//! │   upsert_field() 聚合  │     同一 block 内多字段合并
//! └──────┬───────────────┘
//!        │
//!        │ 字段数 >= max_block_rows ?
//!       ╱╲
//!     是  │  否（继续缓冲）
//!     │   │
//!     ▼   │
//! ┌──────────┐
//! │ flush_block() │  ← 编码为二进制 → put_cf → RocksDB
//! └──────────┘
//! ```
//!

use crate::error::{Result, TsdbError};
use crate::rowkey::{timestamp_to_cf_name, RowKey, SEPARATOR};
use crate::storage::cf_manager::CfManager;
use crate::storage::merge_operand::{MergedBlock, MergedField};
use chrono::NaiveDate;
use rocksdb::MultiThreaded;
use std::collections::HashMap;
use std::sync::Arc;
use tsdb_types::model::DataPoint;

/// RocksDB 多线程实例的类型别名
type TsdbDB = rocksdb::DBWithThreadMode<MultiThreaded>;

/// 块写入器配置参数
///
/// 控制内存缓冲的行为和刷盘策略。
#[derive(Debug, Clone)]
pub struct BlockWriterConfig {
    /// 单个块最大字段数阈值，达到此数量时自动触发 flush
    /// 默认值：1024 个字段（约等于 ~100 个完整数据点，假设每点 10 个字段）
    pub max_block_rows: usize,
    /// 定时刷盘间隔（毫秒），由调用方的外部定时器驱动
    /// 默认值：5000ms（5 秒）
    pub flush_interval_ms: u64,
    /// 是否启用压缩编码（预留参数，当前始终使用 MergedBlock.encode）
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

/// 块唯一标识键（用于 HashMap 分桶）
///
/// 由 measurement、tags_hash 和块起始时间戳三部分组成，
/// 唯一标识一个 30 秒时间窗口内的数据块。
///
/// 实现了 `Hash` + `Eq` 以支持作为 `HashMap` 的 key。
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct BlockKey {
    /// 指标名称（如 `"cpu"`, `"memory"`）
    measurement: String,
    /// 标签组合的哈希签名（通过 DimensionTable.compute_tag_signature 计算）
    tags_hash: u64,
    /// 该块的起始时间戳（微秒，已对齐到 30 秒边界）
    block_start_ts: i64,
}

/// 块写入器 — 缓冲聚合 + 批量刷盘的核心组件
///
/// 维护一个 `HashMap<BlockKey, MergedBlock>` 内存缓冲区：
/// - 每个 `BlockKey` 对应一个唯一的 (measurement, tags, 时间窗口) 组合
/// - 对应的 `MergedBlock` 存储该窗口内所有字段的 upsert 聚合结果
///
/// ## 使用模式
///
/// 创建 BlockWriter 后逐个调用 write() 缓冲数据点，
/// 最后调用 flush_all() 将所有缓冲数据刷入 RocksDB。
///
pub struct BlockWriter {
    /// 底层 RocksDB 实例（Arc 共享引用）
    db: Arc<TsdbDB>,
    /// 列族管理器（负责按需创建日期 CF）
    cf_manager: CfManager,
    /// 写入器配置参数
    config: BlockWriterConfig,
    /// 内存缓冲区：每个 BlockKey 对应一个待刷盘的 MergedBlock
    buffers: HashMap<BlockKey, MergedBlock>,
}

impl BlockWriter {
    /// 创建新的块写入器实例
    ///
    /// # 参数
    /// - `db`: RocksDB 数据库实例
    /// - `cf_manager`: 列族管理器（用于按需创建目标日期的 CF）
    /// - `config`: 写入器配置（最大行数、刷新间隔等）
    pub fn new(db: Arc<TsdbDB>, cf_manager: CfManager, config: BlockWriterConfig) -> Self {
        Self {
            db,
            cf_manager,
            config,
            buffers: HashMap::new(),
        }
    }

    /// 写入单个数据点到缓冲区
    ///
    /// 处理步骤：
    /// 1. 从 DataPoint 提取 RowKey，确定所属的时间块
    /// 2. 计算该点在块内的微秒偏移量（qualifier_offset）
    /// 3. 在对应 MergedBlock 中 upsert 各字段值
    /// 4. 如果块内字段数达到阈值，自动触发 flush 并返回 `Ok(true)`
    ///
    /// # 参数
    /// - `dp`: 待写入的数据点
    ///
    /// # 返回
    /// - `Ok(true)`: 本次写触发了块刷盘
    /// - `Ok(false)`: 数据仅缓冲在内存中，未触发刷盘
    /// - `Err(TsdbError)`: 刷盘过程中发生错误
    pub fn write(&mut self, dp: &DataPoint) -> Result<bool> {
        let row_key = RowKey::from_data_point(dp);
        let block_start = row_key.block_start_timestamp;

        let bk = BlockKey {
            measurement: row_key.measurement.clone(),
            tags_hash: row_key.tags_hash,
            block_start_ts: block_start,
        };

        let qualifier_offset = dp.timestamp - block_start;
        assert!(
            qualifier_offset >= 0
                && qualifier_offset <= crate::rowkey::BLOCK_DURATION_MICROS as i64,
            "qualifier_offset {} out of range [0, {}]",
            qualifier_offset,
            crate::rowkey::BLOCK_DURATION_MICROS
        );
        let qualifier_offset = qualifier_offset as u32;

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

    /// 刷新所有缓冲区中的数据块到磁盘
    ///
    /// 排空整个 `buffers` HashMap，将每个 MergedBlock 逐一写入 RocksDB。
    /// 通常在以下场景调用：
    /// - 定时器触发的周期性刷盘
    /// - 服务关闭前的 graceful shutdown
    /// - 内存压力时的主动释放
    ///
    /// # 返回
    /// - `Ok(usize)`: 成功刷出的数据块数量
    pub fn flush_all(&mut self) -> Result<usize> {
        let blocks: Vec<(BlockKey, MergedBlock)> = self.buffers.drain().collect();
        let count = blocks.len();
        for (bk, block) in blocks {
            self.flush_block(&bk, &block)?;
        }
        Ok(count)
    }

    /// 将单个数据块刷新到 RocksDB
    ///
    /// 内部流程：
    /// 1. 根据块起始时间戳确定目标列族名称和日期
    /// 2. 通过 CfManager 确保目标 CF 已创建
    /// 3. 构造 RowKey 格式的存储键：`measurement|tags_hash|timestamp`
    /// 4. 将 MergedBlock 编码为二进制值
    /// 5. 调用 `put_cf` 写入 RocksDB
    ///
    /// # 参数
    /// - `bk`: 块的唯一标识键
    /// - `block`: 待写入的聚合数据块
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

        self.db
            .put_cf(&cf, &key, &value)
            .map_err(|e| TsdbError::Storage(format!("flush block failed: {}", e)))?;

        Ok(())
    }

    /// 返回当前内存中缓冲的数据块数量（用于监控和调试）
    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }
}

/// 将微秒级时间戳转换为日期（NaiveDate）
///
/// 用于根据数据时间戳确定应该写入哪个日期的列族。
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
