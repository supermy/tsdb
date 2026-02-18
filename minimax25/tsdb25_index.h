#ifndef TSDB25_INDEX_H
#define TSDB25_INDEX_H

#include "tsdb25_types.h"

#define TSDB25_INDEX_ORDER 16

typedef struct tsdb25_index      tsdb25_index_t;
typedef struct tsdb25_series     tsdb25_series_t;

struct tsdb25_series {
    uint64_t         series_id;
    char             measurement[256];
    tsdb25_tag_t     tags[TSDB25_MAX_TAGS_COUNT];
    size_t           tags_count;
    tsdb25_timestamp_t min_time;
    tsdb25_timestamp_t max_time;
    uint64_t         point_count;
    tsdb25_series_t* next;
};

typedef struct {
    uint64_t* series_ids;
    size_t    count;
    size_t    capacity;
} tsdb25_series_list_t;

tsdb25_index_t* tsdb25_index_create(void);
void            tsdb25_index_destroy(tsdb25_index_t* index);

tsdb25_status_t tsdb25_index_add_series(tsdb25_index_t* index, const tsdb25_series_t* series);
tsdb25_status_t tsdb25_index_remove_series(tsdb25_index_t* index, uint64_t series_id);

tsdb25_series_t* tsdb25_index_get_series(tsdb25_index_t* index, uint64_t series_id);

tsdb25_status_t tsdb25_index_find_by_measurement(
    tsdb25_index_t*      index,
    const char*           measurement,
    tsdb25_series_list_t* result
);

tsdb25_status_t tsdb25_index_find_by_tags(
    tsdb25_index_t*      index,
    const char*           measurement,
    const tsdb25_tag_t*  tags,
    size_t               tags_count,
    tsdb25_series_list_t* result
);

tsdb25_status_t tsdb25_index_find_by_time_range(
    tsdb25_index_t*      index,
    const char*           measurement,
    tsdb25_time_range_t* range,
    tsdb25_series_list_t* result
);

tsdb25_status_t tsdb25_index_update_stats(
    tsdb25_index_t*    index,
    uint64_t           series_id,
    tsdb25_timestamp_t timestamp,
    uint64_t           points_added
);

size_t tsdb25_index_series_count(tsdb25_index_t* index);
size_t tsdb25_index_measurement_count(tsdb25_index_t* index);

tsdb25_status_t tsdb25_index_persist(tsdb25_index_t* index, const char* path);
tsdb25_status_t tsdb25_index_load(tsdb25_index_t* index, const char* path);

void tsdb25_series_list_destroy(tsdb25_series_list_t* list);

#endif
