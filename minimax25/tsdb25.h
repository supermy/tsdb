#ifndef TSDB25_H
#define TSDB25_H

#include "tsdb25_types.h"
#include "tsdb25_config.h"

typedef struct tsdb25        tsdb25_t;
typedef struct tsdb25_storage tsdb25_storage_t;
typedef struct tsdb25_index  tsdb25_index_t;
typedef struct tsdb25_cache  tsdb25_cache_t;
typedef struct tsdb25_wal   tsdb25_wal_t;
typedef struct tsdb25_query  tsdb25_query_t;

#include "tsdb25_storage.h"
#include "tsdb25_index.h"
#include "tsdb25_cache.h"
#include "tsdb25_wal.h"

typedef struct tsdb25_query_engine tsdb25_query_engine_t;

typedef struct {
    uint64_t total_points;
    uint64_t total_series;
    uint64_t total_measurements;
    size_t   storage_size;
    size_t   index_size;
    size_t   cache_size;
    size_t   wal_size;
    tsdb25_cache_stats_t cache_stats;
} tsdb25_stats_t;

tsdb25_t* tsdb25_open(const char* path, const tsdb25_config_t* config);
tsdb25_status_t tsdb25_close(tsdb25_t* db);

tsdb25_status_t tsdb25_write(tsdb25_t* db, const tsdb25_point_t* point);
tsdb25_status_t tsdb25_write_batch(tsdb25_t* db, const tsdb25_point_t* points, size_t count);

tsdb25_status_t tsdb25_query_select(tsdb25_t* db, const tsdb25_query_t* query, tsdb25_result_set_t* result);
tsdb25_status_t tsdb25_query_aggregate(tsdb25_t* db, const tsdb25_query_t* query, tsdb25_agg_result_set_t* result);

tsdb25_status_t tsdb25_delete_series(tsdb25_t* db, uint64_t series_id);
tsdb25_status_t tsdb25_delete_measurement(tsdb25_t* db, const char* measurement);

tsdb25_status_t tsdb25_flush(tsdb25_t* db);
tsdb25_status_t tsdb25_compact(tsdb25_t* db);

tsdb25_stats_t tsdb25_stats(tsdb25_t* db);

tsdb25_index_t*    tsdb25_get_index(tsdb25_t* db);
tsdb25_storage_t*  tsdb25_get_storage(tsdb25_t* db);
tsdb25_cache_t*    tsdb25_get_cache(tsdb25_t* db);
tsdb25_wal_t*      tsdb25_get_wal(tsdb25_t* db);

tsdb25_point_t* tsdb25_point_new(const char* measurement, tsdb25_timestamp_t timestamp);
void             tsdb25_point_free(tsdb25_point_t* point);

tsdb25_point_t* tsdb25_point_tag(tsdb25_point_t* point, const char* key, const char* value);
tsdb25_point_t* tsdb25_point_field_f64(tsdb25_point_t* point, const char* name, double value);
tsdb25_point_t* tsdb25_point_field_i64(tsdb25_point_t* point, const char* name, int64_t value);
tsdb25_point_t* tsdb25_point_field_str(tsdb25_point_t* point, const char* name, const char* value);
tsdb25_point_t* tsdb25_point_field_bool(tsdb25_point_t* point, const char* name, bool value);

void tsdb25_result_free(tsdb25_result_set_t* result);
void tsdb25_agg_result_free(tsdb25_agg_result_set_t* result);

tsdb25_query_t* tsdb25_query_create(void);
void             tsdb25_query_destroy(tsdb25_query_t* query);
tsdb25_query_t* tsdb25_query_measurement(tsdb25_query_t* query, const char* name);
tsdb25_query_t* tsdb25_query_filter(tsdb25_query_t* query, void* filter);
tsdb25_query_t* tsdb25_query_time_range(tsdb25_query_t* query, tsdb25_time_range_t range);
tsdb25_query_t* tsdb25_query_set_agg(tsdb25_query_t* query, tsdb25_agg_type_t agg, const char* field);
tsdb25_query_t* tsdb25_query_group_by(tsdb25_query_t* query, const char* tag);
tsdb25_query_t* tsdb25_query_limit(tsdb25_query_t* query, size_t limit, size_t offset);
tsdb25_query_t* tsdb25_query_order(tsdb25_query_t* query, bool ascending);

tsdb25_query_engine_t* tsdb25_query_engine_create(tsdb25_storage_t* storage, tsdb25_index_t* index);
void                    tsdb25_query_engine_destroy(tsdb25_query_engine_t* engine);

tsdb25_status_t tsdb25_query_execute(tsdb25_query_engine_t* engine, const tsdb25_query_t* query, tsdb25_result_set_t* result);
tsdb25_status_t tsdb25_query_aggregate_engine(tsdb25_query_engine_t* engine, const tsdb25_query_t* query, tsdb25_agg_result_set_t* result);

const char* tsdb25_version(void);

#endif
