//! RocksDB 选项配置模块 - RocksDB Options Configuration Module
//!
//! 本模块提供针对时序数据库场景优化的 RocksDB 配置：
//! - `default_opts()`: 全局默认配置，包含 MergeOperator 注册
//! - `hot_cf_opts()`: 热数据 ColumnFamily 配置（LZ4 压缩）
//! - `cold_cf_opts()`: 冷数据 ColumnFamily 配置（ZSTD 压缩）
//! - `metadata_cf_opts()`: 元数据 ColumnFamily 配置
//!
//! ## 优化策略
//!
//! | 场景 | 配置 | 原因 |
//! |------|------|------|
//! | 热数据 | LZ4 + DynamicLevel | 快速压缩，频繁读写 |
//! | 冷数据 | ZSTD + 禁用自动压缩 | 高压缩比，只读场景 |
//! | 元数据 | 小 buffer | 数据量小，快速持久化 |

use rocksdb::{Options, BlockBasedOptions, BlockBasedIndexType, DataBlockIndexType, WriteBufferManager};

/// TSDB RocksDB 选项工厂
///
/// 提供针对不同场景优化的 RocksDB 配置选项。
/// 所有配置都经过针对时序数据特性的调优。
pub struct TsdbOptions;

impl TsdbOptions {
    /// 获取默认 RocksDB 选项
    ///
    /// 这是数据库级别的基础配置，包含：
    /// - MemTable 配置：64MB buffer，最多 4 个
    /// - Compaction 配置：动态层级大小，4 个子压缩线程
    /// - BlockBasedTable 配置：16KB block，二级索引，LRU 缓存
    /// - MergeOperator 注册：tsdb.block_merge
    ///
    /// # 返回值
    ///
    /// 配置好的 RocksDB Options 对象
    ///
    /// # 配置详解
    ///
    /// ## MemTable 配置
    /// - `write_buffer_size = 64MB`: 单个 MemTable 大小，平衡内存占用和写入性能
    /// - `max_write_buffer_number = 4`: 最多 4 个 MemTable，允许并发写入
    /// - `min_write_buffer_number_to_merge = 2`: 至少 2 个 MemTable 才触发合并
    ///
    /// ## Compaction 配置
    /// - `level_compaction_dynamic_level_bytes = true`: 动态调整层级大小
    /// - `max_bytes_for_level_base = 256MB`: L1 层目标大小
    /// - `target_file_size_base = 64MB`: SST 文件目标大小
    /// - `max_subcompactions = 4`: 并发压缩线程数
    ///
    /// ## BlockBasedTable 配置
    /// - `block_size = 16KB`: 适合时序数据的小 KV 特性
    /// - `TwoLevelIndexSearch`: 二级索引，减少内存占用
    /// - `BinaryAndHash`: 数据块内使用二分+哈希混合查找
    /// - `block_cache = 128MB`: LRU 缓存，缓存热点数据块
    pub fn default_opts() -> Options {
        let mut opts = Options::default();

        // 数据库基础配置
        opts.create_if_missing(true);              // 如果数据库不存在则创建
        opts.create_missing_column_families(true); // 自动创建缺失的 CF

        // MemTable 配置
        opts.set_write_buffer_size(64 * 1024 * 1024);  // 64MB MemTable
        opts.set_max_write_buffer_number(4);            // 最多 4 个 MemTable
        opts.set_min_write_buffer_number_to_merge(2);   // 至少 2 个才合并

        // Compaction 配置
        opts.set_level_compaction_dynamic_level_bytes(true);  // 动态层级大小
        opts.set_max_bytes_for_level_base(256 * 1024 * 1024); // L1 层 256MB
        opts.set_target_file_size_base(64 * 1024 * 1024);     // SST 文件 64MB
        opts.set_max_subcompactions(4);                        // 4 个并发压缩线程

        // BlockBasedTable 配置（针对时序数据优化）
        let mut block_opts = BlockBasedOptions::default();

        // 16KB block size：时序数据 KV 较小，小 block 提高缓存效率
        block_opts.set_block_size(16 * 1024);

        // 缓存索引和过滤器到 BlockCache，减少内存占用
        block_opts.set_cache_index_and_filter_blocks(true);

        // L0 层索引和过滤器常驻缓存，加速冷启动
        block_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);

        // 使用最新的 SST 格式版本（v5），支持更多优化
        block_opts.set_format_version(5);

        // 二级索引：减少大文件场景的内存占用
        // 适合时序数据库的长时间范围数据
        block_opts.set_index_type(BlockBasedIndexType::TwoLevelIndexSearch);

        // 数据块内使用二分+哈希混合查找
        // 对于点查询更高效
        block_opts.set_data_block_index_type(DataBlockIndexType::BinaryAndHash);
        block_opts.set_data_block_hash_ratio(0.75);  // 75% 的数据块使用哈希索引

        // 创建 128MB LRU BlockCache
        let cache = rocksdb::Cache::new_lru_cache(128 * 1024 * 1024);
        block_opts.set_block_cache(&cache);

        // 应用 BlockBasedTable 配置
        opts.set_block_based_table_factory(&block_opts);

        crate::storage::merge_operator::register_merge_operator(&mut opts);

        crate::storage::comparator::register_comparator(&mut opts);

        opts
    }

    /// 获取热数据 ColumnFamily 选项
    ///
    /// 热数据是指最近 N 天（默认 7 天）的数据，特点：
    /// - 频繁写入和更新
    /// - 需要快速查询响应
    /// - 压缩效率相对次要
    ///
    /// # 配置策略
    ///
    /// - **压缩**: None → LZ4 → LZ4 → ZSTD
    ///   - L0/L1 不压缩：加速写入
    ///   - L2/L3 使用 LZ4：平衡压缩率和速度
    ///   - L4+ 使用 ZSTD：提高压缩比
    ///
    /// - **MemTable**: 32MB buffer，3 个
    ///   - 比 default 略小，因为热数据 CF 数量多
    ///
    /// - **Compaction**: 动态层级大小
    ///   - 自动调整层级大小，适应数据增长
    ///
    /// # 返回值
    ///
    /// 配置好的 ColumnFamily Options 对象
    pub fn hot_cf_opts() -> Options {
        let mut opts = Options::default();

        // MemTable 配置（比 default 小，因为热数据 CF 多）
        opts.set_write_buffer_size(32 * 1024 * 1024);  // 32MB
        opts.set_max_write_buffer_number(3);

        // 压缩配置：L0/L1 不压缩，L2/L3 LZ4，L4+ ZSTD
        opts.set_compression_per_level(&[
            rocksdb::DBCompressionType::None,   // L0: 不压缩，加速写入
            rocksdb::DBCompressionType::Lz4,    // L1: LZ4 快速压缩
            rocksdb::DBCompressionType::Lz4,    // L2: LZ4
            rocksdb::DBCompressionType::Zstd,   // L3+: ZSTD 高压缩比
        ]);

        // 动态层级大小
        opts.set_level_compaction_dynamic_level_bytes(true);

        opts
    }

    /// 获取冷数据 ColumnFamily 选项
    ///
    /// 冷数据是指超过 N 天（默认 7 天）的数据，特点：
    /// - 几乎不写入，主要是读取
    /// - 数据量大，需要高压缩比
    /// - 可以接受较低的查询性能
    ///
    /// # 配置策略
    ///
    /// - **压缩**: 全部使用 ZSTD
    ///   - 最高压缩比，节省存储空间
    ///   - 冷数据不需要快速压缩
    ///
    /// - **MemTable**: 16MB buffer，2 个
    ///   - 最小配置，因为冷数据几乎不写入
    ///
    /// - **Compaction**: 禁用自动压缩
    ///   - 冷数据稳定后不需要后台压缩
    ///   - 减少不必要的 I/O 开销
    ///
    /// # 返回值
    ///
    /// 配置好的 ColumnFamily Options 对象
    pub fn cold_cf_opts() -> Options {
        let mut opts = Options::default();

        // MemTable 配置（最小配置）
        opts.set_write_buffer_size(16 * 1024 * 1024);  // 16MB
        opts.set_max_write_buffer_number(2);

        // 压缩配置：全部使用 ZSTD
        opts.set_compression_per_level(&[
            rocksdb::DBCompressionType::None,   // L0: 不压缩（临时数据）
            rocksdb::DBCompressionType::Zstd,   // L1+: ZSTD 高压缩比
            rocksdb::DBCompressionType::Zstd,
            rocksdb::DBCompressionType::Zstd,
        ]);

        // 禁用自动压缩：冷数据稳定后不需要
        opts.set_disable_auto_compactions(true);

        opts
    }

    /// 获取元数据 ColumnFamily 选项
    ///
    /// 元数据 CF 用于存储：
    /// - 索引数据（SkipList、InvertedIndex 序列化）
    /// - 维度表数据
    /// - 系统配置和状态
    ///
    /// # 配置策略
    ///
    /// - **MemTable**: 4MB buffer，3 个
    ///   - 元数据量小，不需要大 buffer
    ///   - 快速持久化，减少数据丢失风险
    ///
    /// - **压缩**: 使用默认配置
    ///   - 元数据量小，压缩不是关键
    ///
    /// # 返回值
    ///
    /// 配置好的 ColumnFamily Options 对象
    pub fn metadata_cf_opts() -> Options {
        let mut opts = Options::default();

        // 小 buffer 配置
        opts.set_write_buffer_size(4 * 1024 * 1024);  // 4MB
        opts.set_max_write_buffer_number(3);

        opts
    }

    /// 创建共享 WriteBufferManager
    ///
    /// WriteBufferManager 用于跨多个 DB 实例共享写缓冲区配额，
    /// 当总内存使用超过阈值时自动触发刷盘，防止 OOM。
    ///
    /// # 参数
    ///
    /// - `buffer_size`: 缓冲区大小（字节），推荐 256MB~1GB
    /// - `allow_stall`: 是否在超限时阻塞写入（true=阻塞，false=仅触发刷盘）
    ///
    /// # 返回值
    ///
    /// WriteBufferManager 实例，需在创建所有 DB 实例前创建并共享
    pub fn create_write_buffer_manager(buffer_size: usize, allow_stall: bool) -> WriteBufferManager {
        WriteBufferManager::new_write_buffer_manager(buffer_size, allow_stall)
    }

    /// 创建带缓存的 WriteBufferManager
    ///
    /// 与 BlockCache 共享配额管理，当 MemTable 内存使用超过
    /// cache 容量的某个比例时触发刷盘。
    ///
    /// # 参数
    ///
    /// - `buffer_size`: 缓冲区大小（字节）
    /// - `allow_stall`: 是否在超限时阻塞写入
    /// - `cache`: BlockCache 实例引用
    pub fn create_write_buffer_manager_with_cache(
        buffer_size: usize,
        allow_stall: bool,
        cache: &rocksdb::Cache,
    ) -> WriteBufferManager {
        WriteBufferManager::new_write_buffer_manager_with_cache(buffer_size, allow_stall, cache.clone())
    }

    /// 将 WriteBufferManager 应用到 Options
    ///
    /// # 参数
    ///
    /// - `opts`: 需要修改的 Options
    /// - `wbm`: WriteBufferManager 引用
    pub fn apply_write_buffer_manager(opts: &mut Options, wbm: &WriteBufferManager) {
        opts.set_write_buffer_manager(wbm);
    }
}
