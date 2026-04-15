use rocksdb::{Options, BlockBasedOptions, BlockBasedIndexType, DataBlockIndexType};

pub struct TsdbOptions;

impl TsdbOptions {
    pub fn default_opts() -> Options {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_write_buffer_size(64 * 1024 * 1024);
        opts.set_max_write_buffer_number(4);
        opts.set_min_write_buffer_number_to_merge(2);
        opts.set_level_compaction_dynamic_level_bytes(true);
        opts.set_max_bytes_for_level_base(256 * 1024 * 1024);
        opts.set_target_file_size_base(64 * 1024 * 1024);
        opts.set_max_subcompactions(4);

        let mut block_opts = BlockBasedOptions::default();
        block_opts.set_block_size(16 * 1024);
        block_opts.set_cache_index_and_filter_blocks(true);
        block_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);
        block_opts.set_format_version(5);
        block_opts.set_index_type(BlockBasedIndexType::TwoLevelIndexSearch);
        block_opts.set_data_block_index_type(DataBlockIndexType::BinaryAndHash);
        block_opts.set_data_block_hash_ratio(0.75);

        let cache = rocksdb::Cache::new_lru_cache(128 * 1024 * 1024);
        block_opts.set_block_cache(&cache);

        opts.set_block_based_table_factory(&block_opts);

        crate::storage::merge_operator::register_merge_operator(&mut opts);

        opts
    }

    pub fn hot_cf_opts() -> Options {
        let mut opts = Options::default();
        opts.set_write_buffer_size(32 * 1024 * 1024);
        opts.set_max_write_buffer_number(3);
        opts.set_compression_per_level(&[
            rocksdb::DBCompressionType::None,
            rocksdb::DBCompressionType::Lz4,
            rocksdb::DBCompressionType::Lz4,
            rocksdb::DBCompressionType::Zstd,
        ]);
        opts.set_level_compaction_dynamic_level_bytes(true);
        opts
    }

    pub fn cold_cf_opts() -> Options {
        let mut opts = Options::default();
        opts.set_write_buffer_size(16 * 1024 * 1024);
        opts.set_max_write_buffer_number(2);
        opts.set_compression_per_level(&[
            rocksdb::DBCompressionType::None,
            rocksdb::DBCompressionType::Zstd,
            rocksdb::DBCompressionType::Zstd,
            rocksdb::DBCompressionType::Zstd,
        ]);
        opts.set_disable_auto_compactions(true);
        opts
    }

    pub fn metadata_cf_opts() -> Options {
        let mut opts = Options::default();
        opts.set_write_buffer_size(4 * 1024 * 1024);
        opts.set_max_write_buffer_number(3);
        opts
    }
}
