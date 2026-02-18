#ifndef TSDB_INDEX_H
#define TSDB_INDEX_H

#include "tsdb_types.h"

#define TSDB_INDEX_ORDER 8

typedef struct tsdb_index tsdb_index_t;

typedef struct {
    uint64_t series_id;
    char measurement[128];
    tsdb_tag_t tags[TSDB_MAX_TAGS_COUNT];
    size_t tags_count;
    tsdb_timestamp_t min_time;
    tsdb_timestamp_t max_time;
    uint64_t point_count;
} tsdb_series_info_t;

typedef struct {
    uint64_t* series_ids;
    size_t count;
    size_t capacity;
} tsdb_series_list_t;

tsdb_index_t* tsdb_index_create(void);
void tsdb_index_destroy(tsdb_index_t* index);

tsdb_status_t tsdb_index_add_series(tsdb_index_t* index, const tsdb_series_info_t* info);
tsdb_status_t tsdb_index_remove_series(tsdb_index_t* index, uint64_t series_id);

tsdb_series_info_t* tsdb_index_get_series(tsdb_index_t* index, uint64_t series_id);

tsdb_status_t tsdb_index_find_by_measurement(
    tsdb_index_t* index,
    const char* measurement,
    tsdb_series_list_t* result
);

tsdb_status_t tsdb_index_find_by_tags(
    tsdb_index_t* index,
    const char* measurement,
    const tsdb_tag_t* tags,
    size_t tags_count,
    tsdb_series_list_t* result
);

tsdb_status_t tsdb_index_find_by_time_range(
    tsdb_index_t* index,
    const char* measurement,
    tsdb_time_range_t* range,
    tsdb_series_list_t* result
);

tsdb_status_t tsdb_index_update_stats(
    tsdb_index_t* index,
    uint64_t series_id,
    tsdb_timestamp_t timestamp,
    uint64_t points_added
);

size_t tsdb_index_series_count(tsdb_index_t* index);
size_t tsdb_index_measurement_count(tsdb_index_t* index);

tsdb_status_t tsdb_index_persist(tsdb_index_t* index, const char* path);
tsdb_status_t tsdb_index_load(tsdb_index_t* index, const char* path);

#endif
