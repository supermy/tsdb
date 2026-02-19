#ifndef RTSDB_QUERY_H
#define RTSDB_QUERY_H

#include "rtsdb_types.h"
#include "rtsdb_index.h"
#include "rtsdb_storage.h"

typedef struct rtsdb_query_builder rtsdb_query_builder_t;
typedef struct rtsdb_query_engine rtsdb_query_engine_t;

rtsdb_query_engine_t* rtsdb_query_engine_create(rtsdb_storage_t* storage, rtsdb_index_t* index);
void                  rtsdb_query_engine_destroy(rtsdb_query_engine_t* engine);

rtsdb_status_t rtsdb_query_execute(
    rtsdb_query_engine_t* engine,
    const char*            measurement,
    rtsdb_time_range_t*   range,
    size_t                 limit,
    rtsdb_result_set_t*   result
);

rtsdb_status_t rtsdb_query_aggregate(
    rtsdb_query_engine_t*   engine,
    const char*              measurement,
    rtsdb_time_range_t*     range,
    rtsdb_agg_type_t        agg,
    const char*              field,
    rtsdb_agg_result_set_t* result
);

#endif
