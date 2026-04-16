//! 存储引擎模块 - Storage Engine Module
//!
//! 本模块是 TSDB 的核心存储层，提供：
//! - 数据写入：传统写入 (`write`) 和 MergeOperator 写入 (`write_merged`)
//! - 数据读取：范围查询 (`read_range`) 和单点查询 (`get_point_merged`)
//! - 生命周期管理：过期 CF 清理 (`cleanup`)
//!
//! ## 写入路径对比
//!
//! | 方法 | Key 格式 | Value 格式 | 适用场景 |
//! |------|----------|-----------|----------|
//! | `write` | RowKey + Qualifier | Raw FieldValue | 简单场景，向后兼容 |
//! | `write_merged` | RowKey (纯) | MergedBlock | 高性能场景，推荐使用 |
//!
//! ## 读取路径优化
//!
//! - `read_range`: 支持 Merged/Raw 双格式兼容读取
//! - `get_point_merged`: 单点查询，1 次 get 获取完整数据点

use crate::error::{Result, TsdbError};
use crate::rowkey::{
    align_to_block_start, compute_tags_hash, timestamp_to_cf_name, Qualifier, RowKey, SEPARATOR,
};
use crate::storage::cf_manager::{CfConfig, CfManager, METADATA_CF};
use crate::storage::merge_operand::{
    detect_value_format, encode_merge_operand, MergedBlock, ValueFormat,
};
use crate::storage::options::TsdbOptions;
use chrono::NaiveDate;
use rocksdb::{MultiThreaded, WriteBatch};
use std::path::Path;
use std::sync::Arc;
use tsdb_compress::codec::{BlockCodec, Codec, CompressedBlock, DataBlock};
use tsdb_types::model::{DataPoint, FieldValue, Tags};

/// TSDB 数据库类型别名
///
/// 使用 `MultiThreaded` 模式，允许跨线程共享 ColumnFamily handle。
type TsdbDB = rocksdb::DBWithThreadMode<MultiThreaded>;

/// 存储引擎 - Storage Engine
///
/// TSDB 的核心存储引擎，封装 RocksDB 操作。
///
/// ## 结构
///
/// - `db`: RocksDB 实例（Arc 共享）
/// - `cf_manager`: ColumnFamily 管理器
///
/// ## 线程安全
///
/// `StorageEngine` 是线程安全的，可以在多线程环境中共享使用。
pub struct StorageEngine {
    /// RocksDB 数据库实例
    db: Arc<TsdbDB>,
    /// ColumnFamily 管理器
    cf_manager: CfManager,
}

impl StorageEngine {
    /// 打开存储引擎
    ///
    /// 初始化 RocksDB 并创建必要的 ColumnFamily。
    ///
    /// # 参数
    ///
    /// - `path`: 数据目录路径
    /// - `cf_config`: ColumnFamily 配置（热数据天数、保留天数）
    ///
    /// # 返回值
    ///
    /// 成功返回 `StorageEngine` 实例，失败返回错误
    ///
    /// # 初始化流程
    ///
    /// 1. 创建默认 Options（包含 MergeOperator 注册）
    /// 2. 创建 metadata CF
    /// 3. 打开数据库
    /// 4. 初始化 CF 管理器
    pub fn open(path: &Path, cf_config: CfConfig) -> Result<Self> {
        // 获取默认配置（已注册 MergeOperator）
        let opts = TsdbOptions::default_opts();

        // 配置 metadata CF
        let metadata_cf_opts = TsdbOptions::metadata_cf_opts();
        let cfs = vec![(METADATA_CF, metadata_cf_opts)];

        // 打开数据库
        let db = TsdbDB::open_cf_descriptors(
            &opts,
            path,
            cfs.into_iter()
                .map(|(name, opts)| rocksdb::ColumnFamilyDescriptor::new(name, opts)),
        )
        .map_err(|e| TsdbError::Storage(format!("failed to open DB: {}", e)))?;

        let db = Arc::new(db);
        let cf_manager = CfManager::new(Arc::clone(&db), cf_config);

        Ok(Self { db, cf_manager })
    }

    /// 写入数据点（传统格式）
    ///
    /// 使用 RowKey + Qualifier 格式写入，每个字段一个 KV 对。
    /// 这是向后兼容的写入路径，不使用 MergeOperator。
    ///
    /// # 参数
    ///
    /// - `dp`: 数据点引用
    ///
    /// # 返回值
    ///
    /// 成功返回 `Ok(())`，失败返回错误
    ///
    /// # Key 格式
    ///
    /// ```text
    /// [RowKey bytes] | 0x00 | [Qualifier bytes]
    /// ```
    ///
    /// # Value 格式
    ///
    /// ```text
    /// [type:1B] | [payload:variable]
    /// ```
    pub fn write(&self, dp: &DataPoint) -> Result<()> {
        let row_key = RowKey::from_data_point(dp);
        let block_start = row_key.block_start_timestamp;
        let cf_name = timestamp_to_cf_name(dp.timestamp);

        let date = micros_to_date(dp.timestamp);
        self.cf_manager.ensure_cf_for_date(date)?;

        let cf = self.cf_manager.cf_handle(&cf_name)?;
        let rk_bytes = row_key.encode();
        let rk_prefix_len = rk_bytes.len();

        for (field_name, field_value) in &dp.fields {
            let qualifier = Qualifier::new(field_name, dp.timestamp, block_start);
            let q_bytes = qualifier.encode();

            let mut key_bytes = Vec::with_capacity(rk_prefix_len + 1 + q_bytes.len());
            key_bytes.extend_from_slice(&rk_bytes);
            key_bytes.push(0u8);
            key_bytes.extend_from_slice(&q_bytes);

            let value_bytes = encode_field_value(field_value);

            self.db
                .put_cf(&cf, &key_bytes, &value_bytes)
                .map_err(|e| TsdbError::Storage(format!("write failed: {}", e)))?;
        }

        Ok(())
    }

    /// 批量写入数据点（传统格式）
    ///
    /// 使用 WriteBatch 批量提交，减少 WAL 刷盘次数。
    ///
    /// # 参数
    ///
    /// - `data_points`: 数据点数组
    ///
    /// # 返回值
    ///
    /// 成功返回 `Ok(())`，失败返回错误
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

        // 批量提交
        self.db
            .write(batch)
            .map_err(|e| TsdbError::Storage(format!("batch write failed: {}", e)))?;

        Ok(())
    }

    /// 写入数据点（MergeOperator 格式）⭐ 推荐使用
    ///
    /// 使用 MergeOperator 写入，同一 RowKey 的所有字段合并为一个 MergedBlock。
    /// 这是高性能写入路径，查询时只需 1 次 get 即可获得全部字段。
    ///
    /// # 参数
    ///
    /// - `dp`: 数据点引用
    ///
    /// # 返回值
    ///
    /// 成功返回 `Ok(())`，失败返回错误
    ///
    /// # Key 格式
    ///
    /// ```text
    /// [RowKey bytes]  // 注意：没有 Qualifier！
    /// ```
    ///
    /// # Value 格式
    ///
    /// ```text
    /// [0xFEED] | [field_count:2B] | [field1] | [field2] | ...
    /// ```
    ///
    /// # 性能优势
    ///
    /// - merge 比 put 快约 2x（延迟写 WAL）
    /// - 查询时 1 次 get 获取全部字段
    /// - SST 文件 KV 数量减少 F 倍（F = 字段数）
    pub fn write_merged(&self, dp: &DataPoint) -> Result<()> {
        // 构建 RowKey
        let row_key = RowKey::from_data_point(dp);
        let block_start = row_key.block_start_timestamp;
        let cf_name = timestamp_to_cf_name(dp.timestamp);

        // 确保对应日期的 CF 存在
        let date = micros_to_date(dp.timestamp);
        self.cf_manager.ensure_cf_for_date(date)?;

        // 获取 CF handle
        let cf = self.cf_manager.cf_handle(&cf_name)?;
        let rk_bytes = row_key.encode();

        // 逐字段执行 merge
        for (field_name, field_value) in &dp.fields {
            // 构建 Qualifier（用于计算偏移量）
            let qualifier = Qualifier::new(field_name, dp.timestamp, block_start);

            // 编码 MergeOperand
            let operand =
                encode_merge_operand(field_name, qualifier.microsecond_offset, field_value);

            // 执行 merge 操作
            // RocksDB 会自动调用 tsdb_block_merge 函数合并
            self.db
                .merge_cf(&cf, &rk_bytes, operand)
                .map_err(|e| TsdbError::Storage(format!("merge failed: {}", e)))?;
        }

        Ok(())
    }

    /// 批量写入数据点（MergeOperator 格式）⭐ 最高效路径
    ///
    /// 使用 WriteBatch 批量 merge，最高吞吐量的写入方式。
    ///
    /// # 参数
    ///
    /// - `data_points`: 数据点数组
    ///
    /// # 返回值
    ///
    /// 成功返回 `Ok(())`，失败返回错误
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
                let operand =
                    encode_merge_operand(field_name, qualifier.microsecond_offset, field_value);
                batch.merge_cf(&cf, &rk_bytes, operand);
            }
        }

        // 批量提交
        self.db
            .write(batch)
            .map_err(|e| TsdbError::Storage(format!("merged batch write failed: {}", e)))?;

        Ok(())
    }

    /// 范围查询
    ///
    /// 查询指定时间范围内的所有数据点。
    /// 自动检测并处理 Merged/Raw 两种格式，向后兼容。
    ///
    /// # 参数
    ///
    /// - `measurement`: 指标名称
    /// - `tags`: 标签集合
    /// - `start_micros`: 起始时间戳（微秒）
    /// - `end_micros`: 结束时间戳（微秒）
    ///
    /// # 返回值
    ///
    /// 成功返回数据点列表（按时间戳排序），失败返回错误
    ///
    /// # 查询流程
    ///
    /// 1. 计算标签哈希
    /// 2. 遍历日期范围内的所有 CF
    /// 3. 使用前缀迭代器扫描匹配的 Key
    /// 4. 检测 Value 格式（Merged/Raw）
    /// 5. 解码并过滤时间范围
    /// 6. 按时间戳排序返回
    pub fn read_range(
        &self,
        measurement: &str,
        tags: &Tags,
        start_micros: i64,
        end_micros: i64,
    ) -> Result<Vec<DataPoint>> {
        // 计算标签哈希
        let tags_hash = compute_tags_hash(tags);
        let mut results = Vec::new();

        // 计算日期范围
        let start_date = micros_to_date(start_micros);
        let end_date = micros_to_date(end_micros);

        // 遍历每一天的 CF
        let mut current_date = start_date;
        while current_date <= end_date {
            let cf_name = self.cf_manager.get_cf_name(current_date);

            // 尝试获取 CF handle
            if let Ok(cf) = self.cf_manager.cf_handle(&cf_name) {
                // 构建前缀 Key: measurement | tags_hash |
                let prefix_key = {
                    let mut buf = measurement.as_bytes().to_vec();
                    buf.push(SEPARATOR);
                    buf.extend_from_slice(&tags_hash.to_be_bytes());
                    buf.push(SEPARATOR);
                    buf
                };

                // 使用前缀迭代器扫描
                let iter = self.db.prefix_iterator_cf(&cf, &prefix_key);
                for item in iter {
                    let (key, value) = match item {
                        Ok(kv) => kv,
                        Err(_) => break,
                    };

                    // 检查前缀是否匹配
                    if !key.starts_with(&prefix_key) {
                        break;
                    }

                    // 检测 Value 格式并分别处理
                    match detect_value_format(&value) {
                        // MergedBlock 格式（新版）
                        ValueFormat::Merged => {
                            // 查找 RowKey 和 Qualifier 的分隔符
                            let sep_pos = match key.iter().rposition(|&b| b == 0) {
                                Some(pos) => pos,
                                None => {
                                    // 纯 RowKey（MergeOperator 写入）
                                    if let Some(rk) = RowKey::decode(&key) {
                                        if let Some(block) = MergedBlock::decode(&value) {
                                            let dps = block.to_data_points(
                                                &rk.measurement,
                                                rk.block_start_timestamp,
                                                tags.clone(),
                                            );
                                            for dp in dps {
                                                if dp.timestamp >= start_micros
                                                    && dp.timestamp <= end_micros
                                                {
                                                    results.push(dp);
                                                }
                                            }
                                        }
                                    }
                                    continue;
                                }
                            };

                            // 解码 RowKey 和 MergedBlock
                            let rk_data = &key[..sep_pos];
                            if let Some(rk) = RowKey::decode(rk_data) {
                                if let Some(block) = MergedBlock::decode(&value) {
                                    let dps = block.to_data_points(
                                        &rk.measurement,
                                        rk.block_start_timestamp,
                                        tags.clone(),
                                    );
                                    for dp in dps {
                                        if dp.timestamp >= start_micros
                                            && dp.timestamp <= end_micros
                                        {
                                            results.push(dp);
                                        }
                                    }
                                }
                            }
                        }
                        // Raw 格式（旧版，向后兼容）
                        ValueFormat::Raw => {
                            // 查找 RowKey 和 Qualifier 的分隔符
                            let sep_pos = match key.iter().rposition(|&b| b == 0) {
                                Some(pos) => pos,
                                None => continue,
                            };

                            let rk_data = &key[..sep_pos];
                            let qual_data = &key[sep_pos + 1..];

                            // 解码 RowKey 和 Qualifier
                            if let Some(rk) = RowKey::decode(rk_data) {
                                if let Some(qual) = Qualifier::decode(qual_data) {
                                    // 计算绝对时间戳
                                    let ts =
                                        rk.block_start_timestamp + qual.microsecond_offset as i64;

                                    // 检查时间范围
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

            // 移动到下一天
            current_date += chrono::Duration::days(1);
        }

        // 按时间戳排序
        results.sort_by_key(|dp| dp.timestamp);
        Ok(results)
    }

    /// 单点查询（MergeOperator 格式）⭐ 最高效查询
    ///
    /// 查询指定时间戳的完整数据点。
    /// 使用 MergeOperator 格式时，只需 1 次 get 即可获得全部字段。
    ///
    /// # 参数
    ///
    /// - `measurement`: 指标名称
    /// - `tags`: 标签集合
    /// - `timestamp`: 时间戳（微秒）
    ///
    /// # 返回值
    ///
    /// 找到返回 `Some(DataPoint)`，未找到返回 `None`
    ///
    /// # 性能优势
    ///
    /// - 传统方式：需要 F 次 get（F = 字段数）
    /// - MergeOperator 方式：只需 1 次 get
    pub fn get_point_merged(
        &self,
        measurement: &str,
        tags: &Tags,
        timestamp: i64,
    ) -> Result<Option<DataPoint>> {
        // 计算标签哈希和块起始时间戳
        let tags_hash = compute_tags_hash(tags);
        let block_start = align_to_block_start(timestamp);
        let cf_name = timestamp_to_cf_name(timestamp);

        // 获取 CF handle
        let cf = self.cf_manager.cf_handle(&cf_name)?;

        // 构建 Key: measurement | tags_hash | block_start
        let key = {
            let mut buf = measurement.as_bytes().to_vec();
            buf.push(SEPARATOR);
            buf.extend_from_slice(&tags_hash.to_be_bytes());
            buf.push(SEPARATOR);
            buf.extend_from_slice(&block_start.to_be_bytes());
            buf
        };

        // 执行 get 操作
        match self.db.get_cf(&cf, &key) {
            Ok(Some(value)) => {
                // 解码 MergedBlock
                if let Some(block) = MergedBlock::decode(&value) {
                    // 获取指定时间戳的数据点
                    Ok(block.get_data_point_at(measurement, block_start, timestamp, tags.clone()))
                } else {
                    Ok(None)
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(TsdbError::Storage(format!("get failed: {}", e))),
        }
    }

    /// 清理过期的 ColumnFamily
    ///
    /// 删除超过保留天数的日期 CF，释放磁盘空间。
    ///
    /// # 返回值
    ///
    /// 成功返回被删除的 CF 名称列表，失败返回错误
    pub fn cleanup(&self) -> Result<Vec<String>> {
        self.cf_manager.cleanup_expired_cfs()
    }

    /// 压缩写入数据块
    ///
    /// 将 DataBlock 通过 BlockCodec 压缩后写入 RocksDB。
    /// 适用于批量写入场景，压缩后可节省 50-80% 存储空间。
    ///
    /// # 参数
    ///
    /// - `measurement`: 指标名称
    /// - `tags`: 标签集合
    /// - `block`: 数据块
    ///
    /// # Key 格式
    ///
    /// ```text
    /// [measurement] | [tags_hash:8B] | [block_start_ts:8B] | 0xFF (compressed marker)
    /// ```
    pub fn write_compressed_block(
        &self,
        measurement: &str,
        tags: &Tags,
        block: &DataBlock,
    ) -> Result<()> {
        if block.timestamps.is_empty() {
            return Ok(());
        }

        let tags_hash = compute_tags_hash(tags);
        let block_start = align_to_block_start(block.timestamps[0]);
        let cf_name = timestamp_to_cf_name(block.timestamps[0]);

        let date = micros_to_date(block.timestamps[0]);
        self.cf_manager.ensure_cf_for_date(date)?;

        let cf = self.cf_manager.cf_handle(&cf_name)?;

        let mut key = measurement.as_bytes().to_vec();
        key.push(SEPARATOR);
        key.extend_from_slice(&tags_hash.to_be_bytes());
        key.push(SEPARATOR);
        key.extend_from_slice(&block_start.to_be_bytes());
        key.push(0xFF);

        let codec = BlockCodec;
        let compressed = codec
            .compress_block(block)
            .map_err(|e| TsdbError::Storage(format!("block compression failed: {}", e)))?;

        let compressed_bytes = bincode::serialize(&compressed)
            .map_err(|e| TsdbError::Storage(format!("block serialization failed: {}", e)))?;

        self.db
            .put_cf(&cf, &key, &compressed_bytes)
            .map_err(|e| TsdbError::Storage(format!("compressed write failed: {}", e)))?;

        Ok(())
    }

    /// 压缩读取数据块
    ///
    /// 从 RocksDB 读取压缩的 DataBlock 并解码。
    /// 返回时间范围内的数据点。
    ///
    /// # 参数
    ///
    /// - `measurement`: 指标名称
    /// - `tags`: 标签集合
    /// - `block_start_ts`: 块起始时间戳（微秒）
    /// - `start_micros`: 查询起始时间
    /// - `end_micros`: 查询结束时间
    pub fn read_compressed_block(
        &self,
        measurement: &str,
        tags: &Tags,
        block_start_ts: i64,
        start_micros: i64,
        end_micros: i64,
    ) -> Result<Vec<DataPoint>> {
        let tags_hash = compute_tags_hash(tags);
        let cf_name = timestamp_to_cf_name(block_start_ts);

        let cf = match self.cf_manager.cf_handle(&cf_name) {
            Ok(cf) => cf,
            Err(_) => return Ok(Vec::new()),
        };

        let mut key = measurement.as_bytes().to_vec();
        key.push(SEPARATOR);
        key.extend_from_slice(&tags_hash.to_be_bytes());
        key.push(SEPARATOR);
        key.extend_from_slice(&block_start_ts.to_be_bytes());
        key.push(0xFF);

        match self.db.get_cf(&cf, &key) {
            Ok(Some(value)) => {
                let compressed: CompressedBlock = bincode::deserialize(&value).map_err(|e| {
                    TsdbError::Storage(format!("block deserialization failed: {}", e))
                })?;
                let codec = BlockCodec;
                let block = codec.decompress_block(&compressed).map_err(|e| {
                    TsdbError::Storage(format!("block decompression failed: {}", e))
                })?;

                let mut results = Vec::new();
                for (i, &ts) in block.timestamps.iter().enumerate() {
                    if ts >= start_micros && ts <= end_micros {
                        let mut dp = DataPoint::new(measurement, ts);
                        dp.tags = tags.clone();
                        for (field_name, field_values) in &block.fields {
                            if let Some(fv) = field_values.get(i) {
                                dp.fields.insert(field_name.clone(), fv.clone());
                            }
                        }
                        results.push(dp);
                    }
                }
                Ok(results)
            }
            Ok(None) => Ok(Vec::new()),
            Err(e) => Err(TsdbError::Storage(format!("compressed read failed: {}", e))),
        }
    }

    /// 压缩范围查询
    ///
    /// 查询指定时间范围内的压缩数据块，自动检测压缩格式。
    /// 优先读取压缩块，回退到普通格式。
    pub fn read_range_compressed(
        &self,
        measurement: &str,
        tags: &Tags,
        start_micros: i64,
        end_micros: i64,
    ) -> Result<Vec<DataPoint>> {
        let mut results = Vec::new();
        let start_date = micros_to_date(start_micros);
        let end_date = micros_to_date(end_micros);

        let mut current_date = start_date;
        while current_date <= end_date {
            let cf_name = self.cf_manager.get_cf_name(current_date);

            if let Ok(cf) = self.cf_manager.cf_handle(&cf_name) {
                let tags_hash = compute_tags_hash(tags);
                let mut prefix_key = measurement.as_bytes().to_vec();
                prefix_key.push(SEPARATOR);
                prefix_key.extend_from_slice(&tags_hash.to_be_bytes());
                prefix_key.push(SEPARATOR);

                let iter = self.db.prefix_iterator_cf(&cf, &prefix_key);
                for item in iter {
                    let (key, value) = match item {
                        Ok(kv) => kv,
                        Err(_) => break,
                    };

                    if !key.starts_with(&prefix_key) {
                        break;
                    }

                    if key.last() == Some(&0xFF) {
                        if let Ok(compressed) = bincode::deserialize::<CompressedBlock>(&value) {
                            let codec = BlockCodec;
                            if let Ok(block) = codec.decompress_block(&compressed) {
                                for (i, &ts) in block.timestamps.iter().enumerate() {
                                    if ts >= start_micros && ts <= end_micros {
                                        let mut dp = DataPoint::new(measurement, ts);
                                        dp.tags = tags.clone();
                                        for (field_name, field_values) in &block.fields {
                                            if let Some(fv) = field_values.get(i) {
                                                dp.fields.insert(field_name.clone(), fv.clone());
                                            }
                                        }
                                        results.push(dp);
                                    }
                                }
                            }
                        }
                    } else {
                        match detect_value_format(&value) {
                            ValueFormat::Merged => {
                                if let Some(rk) = RowKey::decode(&key) {
                                    if let Some(block) = MergedBlock::decode(&value) {
                                        let dps = block.to_data_points(
                                            &rk.measurement,
                                            rk.block_start_timestamp,
                                            tags.clone(),
                                        );
                                        for dp in dps {
                                            if dp.timestamp >= start_micros
                                                && dp.timestamp <= end_micros
                                            {
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
                                        let ts = rk.block_start_timestamp
                                            + qual.microsecond_offset as i64;
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
            }

            current_date += chrono::Duration::days(1);
        }

        results.sort_by_key(|dp| dp.timestamp);
        Ok(results)
    }

    /// 持久化索引数据到 metadata CF
    ///
    /// 将 IndexManager 的所有索引数据序列化后写入 RocksDB 的 metadata 列族。
    /// 建议在服务关闭前或定期（如每 5 分钟）调用。
    pub fn persist_index(&self, index_manager: &tsdb_index::IndexManager) -> Result<()> {
        let cf = self.cf_manager.cf_handle(METADATA_CF)?;
        let serialized = index_manager.serialize_all();

        let mut batch = WriteBatch::default();
        for (key, value) in &serialized {
            batch.put_cf(&cf, key.as_bytes(), value);
        }

        self.db
            .write(batch)
            .map_err(|e| TsdbError::Storage(format!("index persist failed: {}", e)))?;

        Ok(())
    }

    /// 从 metadata CF 恢复索引数据
    ///
    /// 启动时调用，从 RocksDB 的 metadata 列族读取序列化的索引数据，
    /// 逐条反序列化并填充到 IndexManager 中。
    pub fn restore_index(&self, index_manager: &mut tsdb_index::IndexManager) -> Result<usize> {
        let cf = self.cf_manager.cf_handle(METADATA_CF)?;
        let mut restored_count = 0;

        let iter = self.db.iterator_cf(
            &cf,
            rocksdb::IteratorMode::From(b"index:", rocksdb::Direction::Forward),
        );

        for item in iter {
            let (key, value) = match item {
                Ok(kv) => kv,
                Err(_) => break,
            };

            let key_str = String::from_utf8_lossy(&key);
            if !key_str.starts_with("index:") {
                break;
            }

            if index_manager.deserialize_entry(&key_str, &value) {
                restored_count += 1;
            }
        }

        Ok(restored_count)
    }

    /// 获取数据库实例引用
    ///
    /// 用于高级操作，如创建快照、手动 Compaction 等。
    pub fn db(&self) -> Arc<TsdbDB> {
        Arc::clone(&self.db)
    }

    /// 获取 CF 管理器引用
    ///
    /// 用于查询 CF 状态、手动创建 CF 等。
    pub fn cf_manager(&self) -> &CfManager {
        &self.cf_manager
    }
}

/// 微秒时间戳转日期
///
/// # 参数
///
/// - `micros`: 微秒时间戳
///
/// # 返回值
///
/// 对应的日期
fn micros_to_date(micros: i64) -> NaiveDate {
    // 微秒转秒
    let secs = micros / 1_000_000;

    // 秒转日期时间
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| dt.date_naive())
        .unwrap_or_else(|| chrono::Local::now().date_naive())
}

/// 编码字段值
///
/// 将 FieldValue 编码为二进制格式，用于 RocksDB 存储。
///
/// # 格式
///
/// ```text
/// Float:    [0x00] | [f64:8B BE]
/// Integer:  [0x01] | [i64:8B BE]
/// String:   [0x02] | [len:4B BE] | [bytes]
/// Boolean:  [0x03] | [0x00/0x01]
/// ```
fn encode_field_value(v: &FieldValue) -> Vec<u8> {
    match v {
        FieldValue::Float(f) => {
            let mut buf = vec![0u8]; // 类型标识
            buf.extend_from_slice(&f.to_be_bytes()); // 大端序
            buf
        }
        FieldValue::Integer(i) => {
            let mut buf = vec![1u8];
            buf.extend_from_slice(&i.to_be_bytes());
            buf
        }
        FieldValue::String(s) => {
            let mut buf = vec![2u8];
            buf.extend_from_slice(&(s.len() as u32).to_be_bytes()); // 长度前缀
            buf.extend_from_slice(s.as_bytes());
            buf
        }
        FieldValue::Boolean(b) => {
            vec![3u8, if *b { 1 } else { 0 }]
        }
    }
}

/// 解码字段值
///
/// 从二进制格式解析 FieldValue。
///
/// # 参数
///
/// - `data`: 二进制数据
///
/// # 返回值
///
/// 解析成功返回 `Some(FieldValue)`，失败返回 `None`
fn decode_field_value(data: &[u8]) -> Option<FieldValue> {
    if data.is_empty() {
        return None;
    }

    match data[0] {
        // Float: 类型(1) + f64(8)
        0 => {
            let f = f64::from_be_bytes(data[1..9].try_into().ok()?);
            Some(FieldValue::Float(f))
        }
        // Integer: 类型(1) + i64(8)
        1 => {
            let i = i64::from_be_bytes(data[1..9].try_into().ok()?);
            Some(FieldValue::Integer(i))
        }
        // String: 类型(1) + len(4) + bytes
        2 => {
            let len = u32::from_be_bytes(data[1..5].try_into().ok()?) as usize;
            let s = String::from_utf8_lossy(&data[5..5 + len]).to_string();
            Some(FieldValue::String(s))
        }
        // Boolean: 类型(1) + value(1)
        3 => Some(FieldValue::Boolean(data.get(1)? == &1)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsdb_types::model::FieldValue;

    /// 测试字段值编解码
    #[test]
    fn test_field_value_encode_decode() {
        let cases = vec![
            FieldValue::Float(std::f64::consts::PI),
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
