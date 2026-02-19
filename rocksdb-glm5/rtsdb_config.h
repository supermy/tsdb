#ifndef RTSDB_CONFIG_H
#define RTSDB_CONFIG_H

#include "rtsdb_types.h"

typedef struct {
    char    data_dir[RTSDB_MAX_PATH_LEN];
    size_t  cache_size_mb;
    size_t  max_open_files;
    bool    enable_compression;
    int     compression_level;
    size_t  write_buffer_mb;
    size_t  max_bytes_for_level_base;
    int     num_levels;
    size_t  block_size;
    bool    bloom_filter;
    size_t  bloom_bits_per_key;
    bool    sync_writes;
} rtsdb_config_t;

rtsdb_config_t rtsdb_default_config(void);
rtsdb_status_t rtsdb_config_validate(const rtsdb_config_t* config);

#endif
