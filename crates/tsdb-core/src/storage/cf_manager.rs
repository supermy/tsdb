//! # 列族（Column Family）管理器
//!
//! ## 架构设计
//!
//! TSDB 采用 **按日期分列族** 的数据组织策略，将时间序列数据按天分散存储到不同的 RocksDB Column Family 中：
//!
//! ```text
//! RocksDB 实例
//! ├── metadata (元数据 CF，永久保留)
//! ├── data_20260415 (今日数据，HOT 模式)
//! ├── data_20260414 (昨日数据，HOT 模式)
//! ├── data_20260408 (7天前，COLD 模式)
//! └── data_20260316 (30天前，已过期删除)
//! ```
//!
//! ## 分层存储策略
//!
//! | 数据年龄 | CF 配置 | 压缩策略 | 说明 |
//! |---------|---------|----------|------|
//! | 0~7 天  | HOT     | LZ4 + Dynamic Level | 高频写入，低延迟查询 |
//! | 8~30 天 | COLD    | ZSTD + Universal | 归档数据，高压缩比 |
//! | >30 天  | —       | 自动清理             | 超出保留期，自动删除 |
//!

use crate::error::{Result, TsdbError};
use crate::storage::options::TsdbOptions;
use chrono::NaiveDate;
use rocksdb::MultiThreaded;
use std::sync::Arc;
use tracing::info;

/// 元数据列族名称（固定不变，用于存储 schema、索引等元信息）
pub const METADATA_CF: &str = "metadata";

/// 数据列族命名前缀，完整格式为 `data_YYYYMMDD`
pub const CF_PREFIX: &str = "data_";

/// 默认热数据天数：最近 7 天的数据使用 HOT 配置（LZ4 压缩、动态层级压缩）
pub const DEFAULT_HOT_DAYS: u64 = 7;

/// 默认数据保留天数：超过 30 天的旧 CF 将被自动清理释放磁盘空间
pub const DEFAULT_RETENTION_DAYS: u64 = 30;

/// RocksDB 多线程实例的类型别名
type TsdbDB = rocksdb::DBWithThreadMode<MultiThreaded>;

/// 列族管理配置参数
///
/// 控制热/冷数据的分界线和数据保留周期。
/// 可通过外部配置覆盖默认值以适应不同业务场景。
#[derive(Debug, Clone)]
pub struct CfConfig {
    /// 热数据窗口大小（天数），在此范围内的 CF 使用高性能 HOT 配置
    pub hot_days: u64,
    /// 数据保留周期（天数），超出此期限的 CF 将被自动删除
    pub retention_days: u64,
}

impl Default for CfConfig {
    fn default() -> Self {
        Self {
            hot_days: DEFAULT_HOT_DAYS,
            retention_days: DEFAULT_RETENTION_DAYS,
        }
    }
}

/// 列族生命周期管理器
///
/// 负责按需创建日期列族、获取列族句柄、以及定期清理过期列族。
/// 核心职责：
/// - **按需创建**：写入时检查目标日期的 CF 是否存在，不存在则自动创建
/// - **分层配置**：根据数据年龄自动选择 HOT/COLD 存储优化策略
/// - **过期清理**：定期扫描并删除超出保留期的旧列族
///
/// ## 线程安全
///
/// `known_cfs` 使用 `Mutex<Vec<String>>` 保护已创建 CF 名称列表，
/// 支持多线程并发调用 `ensure_cf_for_date`。
pub struct CfManager {
    /// 底层 RocksDB 实例（Arc 共享引用）
    db: Arc<TsdbDB>,
    /// 列族管理配置（热数据天数、保留天数）
    config: CfConfig,
    /// 已创建的列族名称列表（用于过期清理时遍历）
    known_cfs: std::sync::Mutex<Vec<String>>,
}

impl CfManager {
    /// 创建新的列族管理器实例
    ///
    /// # 参数
    /// - `db`: RocksDB 数据库实例的共享引用
    /// - `config`: 列族配置（热数据天数和保留天数）
    pub fn new(db: Arc<TsdbDB>, config: CfConfig) -> Self {
        let known_cfs = std::sync::Mutex::new(Vec::new());
        Self { db, config, known_cfs }
    }

    /// 确保指定日期的数据列族已存在
    ///
    /// 如果目标日期的 CF 尚未创建，则根据数据年龄选择合适的存储策略：
    /// - **热数据**（最近 `hot_days` 天）：使用 `TsdbOptions::hot_cf_opts()`（LZ4 + 动态压缩）
    /// - **冷数据**（更早的数据）：使用 `TsdbOptions::cold_cf_opts()`（ZSTD + Universal 压缩）
    ///
    /// # 参数
    /// - `date`: 目标日期（如写入数据的时间戳所属日期）
    ///
    /// # 返回
    /// - `Ok(())`: CF 已存在或成功创建
    /// - `Err(TsdbError::Storage)`: RocksDB 创建 CF 失败
    pub fn ensure_cf_for_date(&self, date: NaiveDate) -> Result<()> {
        let cf_name = format!("{}{}", CF_PREFIX, date.format("%Y%m%d"));

        if self.db.cf_handle(&cf_name).is_some() {
            return Ok(());
        }

        let is_hot = self.is_hot_date(date);
        let cf_opts = if is_hot {
            TsdbOptions::hot_cf_opts()
        } else {
            TsdbOptions::cold_cf_opts()
        };

        info!("creating CF {} (hot={})", cf_name, is_hot);
        self.db.create_cf(&cf_name, &cf_opts)
            .map_err(|e| TsdbError::Storage(format!("failed to create CF {}: {}", cf_name, e)))?;

        self.known_cfs.lock().unwrap().push(cf_name);
        Ok(())
    }

    /// 根据日期生成完整的列族名称
    ///
    /// 格式：`data_YYYYMMDD`，例如 `data_20260415`
    pub fn get_cf_name(&self, date: NaiveDate) -> String {
        format!("{}{}", CF_PREFIX, date.format("%Y%m%d"))
    }

    /// 获取指定名称的列族句柄
    ///
    /// # 参数
    /// - `cf_name`: 列族名称（如 `"data_20260415"` 或 `"metadata"`）
    ///
    /// # 返回
    /// - `Ok(Arc<BoundColumnFamily>)`: 列族句柄，用于后续的 put/get/scan 操作
    /// - `Err(TsdbError::ColumnFamilyNotFound)`: 指定名称的列族不存在
    pub fn cf_handle(&self, cf_name: &str) -> Result<Arc<rocksdb::BoundColumnFamily<'_>>> {
        self.db.cf_handle(cf_name)
            .ok_or_else(|| TsdbError::ColumnFamilyNotFound(cf_name.to_string()))
    }

    /// 清理所有过期的数据列族
    ///
    /// 遍历已知列族列表，删除日期早于 `(当前日期 - retention_days)` 的 CF，
    /// 释放对应的磁盘空间。`metadata` CF 永远不会被清理。
    ///
    /// # 返回
    /// - `Ok(Vec<String>)`: 被删除的列族名称列表
    pub fn cleanup_expired_cfs(&self) -> Result<Vec<String>> {
        let cutoff = chrono::Local::now().date_naive()
            - chrono::Duration::days(self.config.retention_days as i64);
        let mut dropped = Vec::new();

        let cfs_to_check: Vec<String> = self.known_cfs.lock().unwrap().clone();
        for cf_name in &cfs_to_check {
            if cf_name == METADATA_CF {
                continue;
            }
            if let Some(date_str) = cf_name.strip_prefix(CF_PREFIX) {
                if let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y%m%d") {
                    if date < cutoff {
                        info!("dropping expired CF: {}", cf_name);
                        self.db.drop_cf(cf_name)
                            .map_err(|e| TsdbError::Storage(format!("failed to drop CF {}: {}", cf_name, e)))?;
                        dropped.push(cf_name.clone());
                    }
                }
            }
        }

        if !dropped.is_empty() {
            let mut known = self.known_cfs.lock().unwrap();
            known.retain(|n| !dropped.contains(n));
        }

        Ok(dropped)
    }

    /// 判断给定日期是否属于热数据范围
    ///
    /// 热数据定义为：`0 <= (今天 - 目标日期) <= hot_days`
    fn is_hot_date(&self, date: NaiveDate) -> bool {
        let today = chrono::Local::now().date_naive();
        let diff = (today - date).num_days();
        diff >= 0 && (diff as u64) <= self.config.hot_days
    }
}
