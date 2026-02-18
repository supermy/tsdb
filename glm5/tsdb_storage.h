#ifndef TSDB_STORAGE_H
#define TSDB_STORAGE_H

#include "tsdb_types.h"
#include "tsdb_config.h"

#define TSDB_FILE_MAGIC 0x54534442
#define TSDB_FILE_VERSION 1

typedef struct tsdb_storage tsdb_storage_t;

typedef struct {
    uint64_t series_id;
    tsdb_timestamp_t min_time;
    tsdb_timestamp_t max_time;
    uint64_t point_count;
    uint64_t data_offset;
    uint64_t data_size;
} tsdb_series_meta_t;

typedef struct {
    uint32_t magic;
    uint32_t version;
    uint64_t create_time;
    uint64_t series_count;
    uint64_t total_points;
    uint64_t index_offset;
    uint64_t data_offset;
    char reserved[464];
} tsdb_file_header_t;

tsdb_storage_t* tsdb_storage_create(const tsdb_config_t* config);
void tsdb_storage_destroy(tsdb_storage_t* storage);

tsdb_status_t tsdb_storage_open(tsdb_storage_t* storage, const char* path);
tsdb_status_t tsdb_storage_close(tsdb_storage_t* storage);

tsdb_status_t tsdb_storage_write_point(tsdb_storage_t* storage, const tsdb_point_t* point);
tsdb_status_t tsdb_storage_write_batch(tsdb_storage_t* storage, const tsdb_point_t* points, size_t count);

tsdb_status_t tsdb_storage_read_series(
    tsdb_storage_t* storage,
    uint64_t series_id,
    tsdb_time_range_t* range,
    tsdb_result_set_t* result
);

tsdb_status_t tsdb_storage_flush(tsdb_storage_t* storage);
tsdb_status_t tsdb_storage_compact(tsdb_storage_t* storage);

uint64_t tsdb_storage_get_series_id(const tsdb_point_t* point);
uint64_t tsdb_hash_tags(const tsdb_tag_t* tags, size_t count);

#endif
