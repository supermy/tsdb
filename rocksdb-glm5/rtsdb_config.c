#include "rtsdb_config.h"
#include <string.h>
#include <stdlib.h>

rtsdb_config_t rtsdb_default_config(void) {
    rtsdb_config_t config;
    memset(&config, 0, sizeof(config));
    
    strcpy(config.data_dir, "./data");
    config.cache_size_mb = 256;
    config.max_open_files = 10000;
    config.enable_compression = true;
    config.compression_level = 6;
    config.write_buffer_mb = 64;
    config.max_bytes_for_level_base = 256 * 1024 * 1024;
    config.num_levels = 7;
    config.block_size = 4096;
    config.bloom_filter = true;
    config.bloom_bits_per_key = 10;
    config.sync_writes = false;
    
    return config;
}

rtsdb_status_t rtsdb_config_validate(const rtsdb_config_t* config) {
    if (!config) return RTSDB_ERR_INVALID_PARAM;
    if (strlen(config->data_dir) == 0) return RTSDB_ERR_INVALID_PARAM;
    if (config->cache_size_mb == 0) return RTSDB_ERR_INVALID_PARAM;
    return RTSDB_OK;
}
