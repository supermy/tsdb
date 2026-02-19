#ifndef RTSDB_STORAGE_H
#define RTSDB_STORAGE_H

#include "rtsdb_types.h"
#include "rtsdb_config.h"

typedef struct rtsdb_storage rtsdb_storage_t;

rtsdb_storage_t* rtsdb_storage_create(const rtsdb_config_t* config);
void             rtsdb_storage_destroy(rtsdb_storage_t* storage);

rtsdb_status_t rtsdb_storage_open(rtsdb_storage_t* storage, const char* path);
rtsdb_status_t rtsdb_storage_close(rtsdb_storage_t* storage);

rtsdb_status_t rtsdb_storage_write_point(rtsdb_storage_t* storage, const rtsdb_point_t* point);
rtsdb_status_t rtsdb_storage_write_batch(rtsdb_storage_t* storage, const rtsdb_point_t* points, size_t count);

rtsdb_status_t rtsdb_storage_read_series(
    rtsdb_storage_t*    storage,
    const char*          measurement,
    rtsdb_time_range_t* range,
    size_t               limit,
    rtsdb_result_set_t* result
);

rtsdb_status_t rtsdb_storage_delete_measurement(rtsdb_storage_t* storage, const char* measurement);

rtsdb_status_t rtsdb_storage_flush(rtsdb_storage_t* storage);
rtsdb_status_t rtsdb_storage_compact(rtsdb_storage_t* storage);

uint64_t rtsdb_storage_get_series_id(const rtsdb_point_t* point);

size_t rtsdb_storage_total_points(rtsdb_storage_t* storage);

#endif
