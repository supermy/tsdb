#include "rtsdb_query.h"
#include <stdlib.h>
#include <string.h>
#include <math.h>

extern void rtsdb_result_set_destroy(rtsdb_result_set_t* result);

struct rtsdb_query_engine {
    rtsdb_storage_t* storage;
    rtsdb_index_t*   index;
};

rtsdb_query_engine_t* rtsdb_query_engine_create(rtsdb_storage_t* storage, rtsdb_index_t* index) {
    rtsdb_query_engine_t* e = (rtsdb_query_engine_t*)calloc(1, sizeof(rtsdb_query_engine_t));
    if (!e) return NULL;
    e->storage = storage;
    e->index = index;
    return e;
}

void rtsdb_query_engine_destroy(rtsdb_query_engine_t* engine) {
    free(engine);
}

rtsdb_status_t rtsdb_query_execute(rtsdb_query_engine_t* engine, const char* measurement,
                                    rtsdb_time_range_t* range, size_t limit,
                                    rtsdb_result_set_t* result) {
    if (!engine || !measurement || !result) return RTSDB_ERR_INVALID_PARAM;
    return rtsdb_storage_read_series(engine->storage, measurement, range, limit, result);
}

rtsdb_status_t rtsdb_query_aggregate(rtsdb_query_engine_t* engine, const char* measurement,
                                      rtsdb_time_range_t* range, rtsdb_agg_type_t agg,
                                      const char* field, rtsdb_agg_result_set_t* result) {
    (void)field;
    
    if (!engine || !measurement || !result) return RTSDB_ERR_INVALID_PARAM;
    
    memset(result, 0, sizeof(rtsdb_agg_result_set_t));
    
    rtsdb_result_set_t data;
    rtsdb_status_t status = rtsdb_storage_read_series(engine->storage, measurement, range, 1000000, &data);
    if (status != RTSDB_OK) return status;
    
    if (data.count == 0) {
        rtsdb_result_set_destroy(&data);
        return RTSDB_OK;
    }
    
    result->capacity = 1;
    result->results = (rtsdb_agg_result_t*)calloc(result->capacity, sizeof(rtsdb_agg_result_t));
    if (!result->results) {
        rtsdb_result_set_destroy(&data);
        return RTSDB_ERR_NO_MEMORY;
    }
    
    rtsdb_agg_result_t* res = &result->results[0];
    res->count = data.count;
    
    switch (agg) {
        case RTSDB_AGG_COUNT:
            res->value = (rtsdb_value_t)data.count;
            break;
        case RTSDB_AGG_SUM:
            for (size_t i = 0; i < data.count; i++) {
                if (data.points[i].fields_count > 0)
                    res->value += data.points[i].fields[0].value.float_val;
            }
            break;
        case RTSDB_AGG_AVG:
            for (size_t i = 0; i < data.count; i++) {
                if (data.points[i].fields_count > 0)
                    res->value += data.points[i].fields[0].value.float_val;
            }
            res->value /= data.count;
            break;
        case RTSDB_AGG_MIN:
            res->value = INFINITY;
            for (size_t i = 0; i < data.count; i++) {
                if (data.points[i].fields_count > 0) {
                    double v = data.points[i].fields[0].value.float_val;
                    if (v < res->value) res->value = v;
                }
            }
            break;
        case RTSDB_AGG_MAX:
            res->value = -INFINITY;
            for (size_t i = 0; i < data.count; i++) {
                if (data.points[i].fields_count > 0) {
                    double v = data.points[i].fields[0].value.float_val;
                    if (v > res->value) res->value = v;
                }
            }
            break;
        case RTSDB_AGG_FIRST:
            if (data.points[0].fields_count > 0) {
                res->value = data.points[0].fields[0].value.float_val;
                res->timestamp = data.points[0].timestamp;
            }
            break;
        case RTSDB_AGG_LAST:
            if (data.points[data.count - 1].fields_count > 0) {
                res->value = data.points[data.count - 1].fields[0].value.float_val;
                res->timestamp = data.points[data.count - 1].timestamp;
            }
            break;
        case RTSDB_AGG_STDDEV: {
            double sum = 0, sum_sq = 0;
            for (size_t i = 0; i < data.count; i++) {
                if (data.points[i].fields_count > 0) {
                    double v = data.points[i].fields[0].value.float_val;
                    sum += v;
                    sum_sq += v * v;
                }
            }
            double mean = sum / data.count;
            res->value = sqrt(sum_sq / data.count - mean * mean);
            break;
        }
        default:
            break;
    }
    
    result->count = 1;
    rtsdb_result_set_destroy(&data);
    return RTSDB_OK;
}
