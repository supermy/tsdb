#include "tsdb_config.h"
#include <string.h>
#include <stdlib.h>

tsdb_config_t tsdb_default_config(void) {
    tsdb_config_t config;
    memset(&config, 0, sizeof(config));
    
    strcpy(config.data_dir, "./data");
    config.block_size = TSDB_DEFAULT_BLOCK_SIZE;
    config.cache_size_mb = 64;
    config.max_open_files = 1000;
    config.enable_compression = true;
    config.compression_level = 3;
    config.write_buffer_size_mb = 16;
    config.max_series_per_measurement = 1000000;
    config.sync_writes = false;
    
    return config;
}

tsdb_status_t tsdb_config_validate(const tsdb_config_t* config) {
    if (!config) {
        return TSDB_ERR_INVALID_PARAM;
    }
    
    if (strlen(config->data_dir) == 0) {
        return TSDB_ERR_INVALID_PARAM;
    }
    
    if (config->block_size < 1024 || config->block_size > 1024 * 1024) {
        return TSDB_ERR_INVALID_PARAM;
    }
    
    if (config->cache_size_mb == 0) {
        return TSDB_ERR_INVALID_PARAM;
    }
    
    if (config->compression_level < 0 || config->compression_level > 9) {
        return TSDB_ERR_INVALID_PARAM;
    }
    
    return TSDB_OK;
}
