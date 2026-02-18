#include "tsdb25_config.h"
#include <string.h>
#include <stdlib.h>

tsdb25_config_t tsdb25_default_config(void) {
    tsdb25_config_t config;
    memset(&config, 0, sizeof(config));
    
    strcpy(config.data_dir, "./data");
    config.block_size = TSDB25_DEFAULT_BLOCK_SIZE;
    config.cache_size_mb = 128;
    config.max_open_files = 1000;
    config.enable_compression = true;
    config.compression_level = 3;
    config.write_buffer_size_mb = 32;
    config.max_series_per_measurement = 10000000;
    config.sync_writes = false;
    config.enable_wal = true;
    config.wal_size_mb = 64;
    config.num_threads = 4;
    
    return config;
}

tsdb25_status_t tsdb25_config_validate(const tsdb25_config_t* config) {
    if (!config) return TSDB25_ERR_INVALID_PARAM;
    if (strlen(config->data_dir) == 0) return TSDB25_ERR_INVALID_PARAM;
    if (config->block_size < 1024 || config->block_size > 1024 * 1024) return TSDB25_ERR_INVALID_PARAM;
    if (config->cache_size_mb == 0) return TSDB25_ERR_INVALID_PARAM;
    if (config->compression_level < 0 || config->compression_level > 9) return TSDB25_ERR_INVALID_PARAM;
    return TSDB25_OK;
}
