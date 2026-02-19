#ifndef RTSDB_H
#define RTSDB_H

#include "rtsdb_types.h"
#include "rtsdb_config.h"
#include "rtsdb_storage.h"
#include "rtsdb_index.h"
#include "rtsdb_query.h"

typedef struct rtsdb rtsdb_t;

typedef struct {
    uint64_t total_points;
    uint64_t total_series;
    uint64_t total_measurements;
    size_t   storage_size;
    size_t   index_size;
} rtsdb_stats_t;

rtsdb_t* rtsdb_open(const char* path, const rtsdb_config_t* config);
rtsdb_status_t rtsdb_close(rtsdb_t* db);

rtsdb_status_t rtsdb_write(rtsdb_t* db, const rtsdb_point_t* point);
rtsdb_status_t rtsdb_write_batch(rtsdb_t* db, const rtsdb_point_t* points, size_t count);

rtsdb_status_t rtsdb_query(rtsdb_t* db, const char* measurement, 
                           rtsdb_time_range_t* range, size_t limit,
                           rtsdb_result_set_t* result);

rtsdb_status_t rtsdb_query_agg(rtsdb_t* db, const char* measurement,
                               rtsdb_time_range_t* range, rtsdb_agg_type_t agg,
                               const char* field, rtsdb_agg_result_set_t* result);

rtsdb_status_t rtsdb_delete_measurement(rtsdb_t* db, const char* measurement);

rtsdb_status_t rtsdb_flush(rtsdb_t* db);
rtsdb_status_t rtsdb_compact(rtsdb_t* db);

rtsdb_stats_t rtsdb_stats(rtsdb_t* db);

rtsdb_point_t* rtsdb_point_create(const char* measurement, rtsdb_timestamp_t timestamp);
void           rtsdb_point_destroy(rtsdb_point_t* point);

rtsdb_point_t* rtsdb_point_add_tag(rtsdb_point_t* point, const char* key, const char* value);
rtsdb_point_t* rtsdb_point_add_field_float(rtsdb_point_t* point, const char* name, double value);
rtsdb_point_t* rtsdb_point_add_field_int(rtsdb_point_t* point, const char* name, int64_t value);
rtsdb_point_t* rtsdb_point_add_field_string(rtsdb_point_t* point, const char* name, const char* value);
rtsdb_point_t* rtsdb_point_add_field_bool(rtsdb_point_t* point, const char* name, bool value);

void rtsdb_result_set_destroy(rtsdb_result_set_t* result);
void rtsdb_agg_result_set_destroy(rtsdb_agg_result_set_t* result);

const char* rtsdb_version(void);

#endif
