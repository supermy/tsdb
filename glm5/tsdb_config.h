#ifndef TSDB_CONFIG_H
#define TSDB_CONFIG_H

#include "tsdb_types.h"

typedef struct {
    char data_dir[TSDB_MAX_PATH_LEN];
    size_t block_size;
    size_t cache_size_mb;
    size_t max_open_files;
    bool enable_compression;
    int compression_level;
    size_t write_buffer_size_mb;
    size_t max_series_per_measurement;
    bool sync_writes;
} tsdb_config_t;

tsdb_config_t tsdb_default_config(void);
tsdb_status_t tsdb_config_validate(const tsdb_config_t* config);

#endif
