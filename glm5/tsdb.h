#ifndef TSDB_H
#define TSDB_H

#include "tsdb_types.h"
#include "tsdb_config.h"
#include "tsdb_storage.h"
#include "tsdb_index.h"
#include "tsdb_query.h"
#include "tsdb_compress.h"
#include "tsdb_cache.h"

typedef struct tsdb tsdb_t;

typedef struct {
    uint64_t total_points;
    uint64_t total_series;
    uint64_t total_measurements;
    size_t storage_size;
    size_t index_size;
    size_t cache_size;
    tsdb_cache_stats_t cache_stats;
} tsdb_stats_t;

tsdb_t* tsdb_open(const char* path, const tsdb_config_t* config);
tsdb_status_t tsdb_close(tsdb_t* db);

tsdb_status_t tsdb_write(tsdb_t* db, const tsdb_point_t* point);
tsdb_status_t tsdb_write_batch(tsdb_t* db, const tsdb_point_t* points, size_t count);

tsdb_status_t tsdb_query_data(tsdb_t* db, const tsdb_query_builder_t* query, tsdb_result_set_t* result);
tsdb_status_t tsdb_query_agg(tsdb_t* db, const tsdb_query_builder_t* query, tsdb_agg_result_set_t* result);

tsdb_status_t tsdb_delete_series(tsdb_t* db, uint64_t series_id);
tsdb_status_t tsdb_delete_measurement(tsdb_t* db, const char* measurement);

tsdb_status_t tsdb_flush(tsdb_t* db);
tsdb_status_t tsdb_compact(tsdb_t* db);

tsdb_stats_t tsdb_get_stats(tsdb_t* db);
double tsdb_get_cache_hit_rate(tsdb_t* db);

tsdb_index_t* tsdb_get_index(tsdb_t* db);
tsdb_storage_t* tsdb_get_storage(tsdb_t* db);

tsdb_point_t* tsdb_point_create(const char* measurement, tsdb_timestamp_t timestamp);
void tsdb_point_destroy(tsdb_point_t* point);

tsdb_point_t* tsdb_point_add_tag(tsdb_point_t* point, const char* key, const char* value);
tsdb_point_t* tsdb_point_add_field_float(tsdb_point_t* point, const char* name, double value);
tsdb_point_t* tsdb_point_add_field_int(tsdb_point_t* point, const char* name, int64_t value);
tsdb_point_t* tsdb_point_add_field_string(tsdb_point_t* point, const char* name, const char* value);
tsdb_point_t* tsdb_point_add_field_bool(tsdb_point_t* point, const char* name, bool value);

void tsdb_result_set_destroy(tsdb_result_set_t* result);
void tsdb_agg_result_set_destroy(tsdb_agg_result_set_t* result);
void tsdb_series_list_destroy(tsdb_series_list_t* list);

const char* tsdb_version(void);

#endif
