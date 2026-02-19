#include "rtsdb.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <time.h>

struct rtsdb {
    rtsdb_config_t       config;
    rtsdb_storage_t*     storage;
    rtsdb_index_t*       index;
    rtsdb_query_engine_t* query_engine;
    bool                 is_open;
};

const char* rtsdb_version(void) {
    return RTSDB_VERSION;
}

rtsdb_t* rtsdb_open(const char* path, const rtsdb_config_t* config) {
    if (!path) return NULL;
    
    rtsdb_t* db = (rtsdb_t*)calloc(1, sizeof(rtsdb_t));
    if (!db) return NULL;
    
    if (config) {
        db->config = *config;
    } else {
        db->config = rtsdb_default_config();
    }
    
    strncpy(db->config.data_dir, path, RTSDB_MAX_PATH_LEN - 1);
    
    db->storage = rtsdb_storage_create(&db->config);
    if (!db->storage) {
        free(db);
        return NULL;
    }
    
    rtsdb_status_t status = rtsdb_storage_open(db->storage, path);
    if (status != RTSDB_OK) {
        rtsdb_storage_destroy(db->storage);
        free(db);
        return NULL;
    }
    
    db->index = rtsdb_index_create();
    if (!db->index) {
        rtsdb_storage_destroy(db->storage);
        free(db);
        return NULL;
    }
    
    char index_path[RTSDB_MAX_PATH_LEN];
    snprintf(index_path, sizeof(index_path), "%s/index.db", path);
    rtsdb_index_load(db->index, index_path);
    
    db->query_engine = rtsdb_query_engine_create(db->storage, db->index);
    if (!db->query_engine) {
        rtsdb_index_destroy(db->index);
        rtsdb_storage_destroy(db->storage);
        free(db);
        return NULL;
    }
    
    db->is_open = true;
    return db;
}

rtsdb_status_t rtsdb_close(rtsdb_t* db) {
    if (!db) return RTSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return RTSDB_ERR_INVALID_STATE;
    
    rtsdb_flush(db);
    
    char index_path[RTSDB_MAX_PATH_LEN];
    snprintf(index_path, sizeof(index_path), "%s/index.db", db->config.data_dir);
    rtsdb_index_persist(db->index, index_path);
    
    rtsdb_query_engine_destroy(db->query_engine);
    rtsdb_index_destroy(db->index);
    rtsdb_storage_destroy(db->storage);
    
    db->is_open = false;
    free(db);
    return RTSDB_OK;
}

rtsdb_status_t rtsdb_write(rtsdb_t* db, const rtsdb_point_t* point) {
    if (!db || !point) return RTSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return RTSDB_ERR_INVALID_STATE;
    
    rtsdb_status_t status = rtsdb_storage_write_point(db->storage, point);
    if (status != RTSDB_OK) return status;
    
    uint64_t series_id = rtsdb_storage_get_series_id(point);
    rtsdb_series_info_t* existing = rtsdb_index_get_series(db->index, series_id);
    
    if (!existing) {
        rtsdb_series_info_t info;
        memset(&info, 0, sizeof(info));
        info.series_id = series_id;
        strncpy(info.measurement, point->measurement, RTSDB_MAX_MEASUREMENT_LEN - 1);
        memcpy(info.tags, point->tags, sizeof(rtsdb_tag_t) * point->tags_count);
        info.tags_count = point->tags_count;
        info.min_time = point->timestamp;
        info.max_time = point->timestamp;
        info.point_count = 1;
        rtsdb_index_add_series(db->index, &info);
    } else {
        rtsdb_index_update_stats(db->index, series_id, point->timestamp, 1);
    }
    
    return RTSDB_OK;
}

rtsdb_status_t rtsdb_write_batch(rtsdb_t* db, const rtsdb_point_t* points, size_t count) {
    if (!db || !points) return RTSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return RTSDB_ERR_INVALID_STATE;
    
    for (size_t i = 0; i < count; i++) {
        rtsdb_status_t status = rtsdb_write(db, &points[i]);
        if (status != RTSDB_OK) return status;
    }
    return RTSDB_OK;
}

rtsdb_status_t rtsdb_query(rtsdb_t* db, const char* measurement,
                           rtsdb_time_range_t* range, size_t limit,
                           rtsdb_result_set_t* result) {
    if (!db || !measurement || !result) return RTSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return RTSDB_ERR_INVALID_STATE;
    return rtsdb_query_execute(db->query_engine, measurement, range, limit, result);
}

rtsdb_status_t rtsdb_query_agg(rtsdb_t* db, const char* measurement,
                               rtsdb_time_range_t* range, rtsdb_agg_type_t agg,
                               const char* field, rtsdb_agg_result_set_t* result) {
    if (!db || !measurement || !result) return RTSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return RTSDB_ERR_INVALID_STATE;
    return rtsdb_query_aggregate(db->query_engine, measurement, range, agg, field, result);
}

rtsdb_status_t rtsdb_delete_measurement(rtsdb_t* db, const char* measurement) {
    if (!db || !measurement) return RTSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return RTSDB_ERR_INVALID_STATE;
    return rtsdb_storage_delete_measurement(db->storage, measurement);
}

rtsdb_status_t rtsdb_flush(rtsdb_t* db) {
    if (!db) return RTSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return RTSDB_ERR_INVALID_STATE;
    return rtsdb_storage_flush(db->storage);
}

rtsdb_status_t rtsdb_compact(rtsdb_t* db) {
    if (!db) return RTSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return RTSDB_ERR_INVALID_STATE;
    return rtsdb_storage_compact(db->storage);
}

rtsdb_stats_t rtsdb_stats(rtsdb_t* db) {
    rtsdb_stats_t s = {0};
    if (db && db->is_open) {
        s.total_series = rtsdb_index_series_count(db->index);
        s.total_measurements = rtsdb_index_measurement_count(db->index);
        s.total_points = rtsdb_storage_total_points(db->storage);
    }
    return s;
}

rtsdb_point_t* rtsdb_point_create(const char* measurement, rtsdb_timestamp_t timestamp) {
    if (!measurement) return NULL;
    
    rtsdb_point_t* p = (rtsdb_point_t*)calloc(1, sizeof(rtsdb_point_t));
    if (!p) return NULL;
    
    strncpy(p->measurement, measurement, RTSDB_MAX_MEASUREMENT_LEN - 1);
    p->timestamp = timestamp > 0 ? timestamp : (rtsdb_timestamp_t)time(NULL) * 1000000000LL;
    return p;
}

void rtsdb_point_destroy(rtsdb_point_t* point) {
    free(point);
}

rtsdb_point_t* rtsdb_point_add_tag(rtsdb_point_t* point, const char* key, const char* value) {
    if (!point || !key || !value) return point;
    if (point->tags_count >= RTSDB_MAX_TAGS_COUNT) return point;
    
    strncpy(point->tags[point->tags_count].key, key, RTSDB_MAX_TAG_KEY_LEN - 1);
    strncpy(point->tags[point->tags_count].value, value, RTSDB_MAX_TAG_VALUE_LEN - 1);
    point->tags_count++;
    return point;
}

rtsdb_point_t* rtsdb_point_add_field_float(rtsdb_point_t* point, const char* name, double value) {
    if (!point || !name) return point;
    if (point->fields_count >= RTSDB_MAX_FIELDS_COUNT) return point;
    
    strncpy(point->fields[point->fields_count].name, name, RTSDB_MAX_FIELD_NAME_LEN - 1);
    point->fields[point->fields_count].type = RTSDB_FIELD_FLOAT;
    point->fields[point->fields_count].value.float_val = value;
    point->fields_count++;
    return point;
}

rtsdb_point_t* rtsdb_point_add_field_int(rtsdb_point_t* point, const char* name, int64_t value) {
    if (!point || !name) return point;
    if (point->fields_count >= RTSDB_MAX_FIELDS_COUNT) return point;
    
    strncpy(point->fields[point->fields_count].name, name, RTSDB_MAX_FIELD_NAME_LEN - 1);
    point->fields[point->fields_count].type = RTSDB_FIELD_INTEGER;
    point->fields[point->fields_count].value.int_val = value;
    point->fields_count++;
    return point;
}

rtsdb_point_t* rtsdb_point_add_field_string(rtsdb_point_t* point, const char* name, const char* value) {
    if (!point || !name || !value) return point;
    if (point->fields_count >= RTSDB_MAX_FIELDS_COUNT) return point;
    
    strncpy(point->fields[point->fields_count].name, name, RTSDB_MAX_FIELD_NAME_LEN - 1);
    point->fields[point->fields_count].type = RTSDB_FIELD_STRING;
    strncpy(point->fields[point->fields_count].value.str_val, value, 511);
    point->fields_count++;
    return point;
}

rtsdb_point_t* rtsdb_point_add_field_bool(rtsdb_point_t* point, const char* name, bool value) {
    if (!point || !name) return point;
    if (point->fields_count >= RTSDB_MAX_FIELDS_COUNT) return point;
    
    strncpy(point->fields[point->fields_count].name, name, RTSDB_MAX_FIELD_NAME_LEN - 1);
    point->fields[point->fields_count].type = RTSDB_FIELD_BOOLEAN;
    point->fields[point->fields_count].value.bool_val = value;
    point->fields_count++;
    return point;
}

void rtsdb_result_set_destroy(rtsdb_result_set_t* result) {
    if (!result) return;
    free(result->points);
    result->points = NULL;
    result->count = 0;
    result->capacity = 0;
}

void rtsdb_agg_result_set_destroy(rtsdb_agg_result_set_t* result) {
    if (!result) return;
    free(result->results);
    result->results = NULL;
    result->count = 0;
    result->capacity = 0;
}
