#ifndef RTSDB_INDEX_H
#define RTSDB_INDEX_H

#include "rtsdb_types.h"

typedef struct rtsdb_index rtsdb_index_t;

typedef struct {
    uint64_t         series_id;
    char             measurement[RTSDB_MAX_MEASUREMENT_LEN];
    rtsdb_tag_t      tags[RTSDB_MAX_TAGS_COUNT];
    size_t           tags_count;
    rtsdb_timestamp_t min_time;
    rtsdb_timestamp_t max_time;
    uint64_t         point_count;
} rtsdb_series_info_t;

typedef struct {
    uint64_t* series_ids;
    size_t    count;
    size_t    capacity;
} rtsdb_series_list_t;

rtsdb_index_t* rtsdb_index_create(void);
void           rtsdb_index_destroy(rtsdb_index_t* index);

rtsdb_status_t rtsdb_index_add_series(rtsdb_index_t* index, const rtsdb_series_info_t* info);
rtsdb_status_t rtsdb_index_remove_series(rtsdb_index_t* index, uint64_t series_id);

rtsdb_series_info_t* rtsdb_index_get_series(rtsdb_index_t* index, uint64_t series_id);

rtsdb_status_t rtsdb_index_find_by_measurement(
    rtsdb_index_t*      index,
    const char*          measurement,
    rtsdb_series_list_t* result
);

rtsdb_status_t rtsdb_index_update_stats(
    rtsdb_index_t*    index,
    uint64_t           series_id,
    rtsdb_timestamp_t timestamp,
    uint64_t           points_added
);

size_t rtsdb_index_series_count(rtsdb_index_t* index);
size_t rtsdb_index_measurement_count(rtsdb_index_t* index);

rtsdb_status_t rtsdb_index_persist(rtsdb_index_t* index, const char* path);
rtsdb_status_t rtsdb_index_load(rtsdb_index_t* index, const char* path);

void rtsdb_series_list_destroy(rtsdb_series_list_t* list);

#endif
