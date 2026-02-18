#ifndef TSDB25_STORAGE_H
#define TSDB25_STORAGE_H

#include "tsdb25_types.h"
#include "tsdb25_config.h"

#define TSDB25_FILE_MAGIC   0x54443235
#define TSDB25_FILE_VERSION  2

typedef struct tsdb25_storage tsdb25_storage_t;

typedef struct {
    uint64_t         series_id;
    char             measurement[256];
    tsdb25_timestamp_t min_time;
    tsdb25_timestamp_t max_time;
    uint64_t         point_count;
    uint64_t         data_offset;
    uint64_t         data_size;
} tsdb25_series_meta_t;

typedef struct {
    uint32_t magic;
    uint32_t version;
    uint64_t create_time;
    uint64_t series_count;
    uint64_t total_points;
    uint64_t index_offset;
    uint64_t data_offset;
    char     reserved[480];
} tsdb25_file_header_t;

typedef struct {
    tsdb25_timestamp_t* timestamps;
    tsdb25_value_t*     values;
    size_t              count;
    size_t              capacity;
} tsdb25_series_block_t;

typedef struct {
    uint64_t  series_id;
    char      file_path[TSDB25_MAX_PATH_LEN];
    int       fd;
    size_t    file_size;
    size_t    num_blocks;
    bool      is_dirty;
} tsdb25_series_file_t;

tsdb25_storage_t* tsdb25_storage_create(const tsdb25_config_t* config);
void              tsdb25_storage_destroy(tsdb25_storage_t* storage);

tsdb25_status_t tsdb25_storage_open(tsdb25_storage_t* storage, const char* path);
tsdb25_status_t tsdb25_storage_close(tsdb25_storage_t* storage);

tsdb25_status_t tsdb25_storage_write_point(tsdb25_storage_t* storage, const tsdb25_point_t* point);
tsdb25_status_t tsdb25_storage_write_batch(tsdb25_storage_t* storage, const tsdb25_point_t* points, size_t count);

tsdb25_status_t tsdb25_storage_read_series(
    tsdb25_storage_t*    storage,
    uint64_t             series_id,
    tsdb25_time_range_t* range,
    tsdb25_result_set_t* result
);

tsdb25_status_t tsdb25_storage_flush(tsdb25_storage_t* storage);
tsdb25_status_t tsdb25_storage_compact(tsdb25_storage_t* storage);

uint64_t tsdb25_storage_get_series_id(const tsdb25_point_t* point);
uint64_t tsdb25_hash_series(const tsdb25_point_t* point);

size_t   tsdb25_storage_total_points(tsdb25_storage_t* storage);
size_t   tsdb25_storage_total_series(tsdb25_storage_t* storage);

#endif
