#ifndef TSDB25_CONFIG_H
#define TSDB25_CONFIG_H

#include "tsdb25_types.h"

typedef struct {
    char    data_dir[TSDB25_MAX_PATH_LEN];
    size_t  block_size;
    size_t  cache_size_mb;
    size_t  max_open_files;
    bool    enable_compression;
    int     compression_level;
    size_t  write_buffer_size_mb;
    size_t  max_series_per_measurement;
    bool    sync_writes;
    bool    enable_wal;
    size_t  wal_size_mb;
    int     num_threads;
} tsdb25_config_t;

tsdb25_config_t tsdb25_default_config(void);
tsdb25_status_t tsdb25_config_validate(const tsdb25_config_t* config);

#endif
