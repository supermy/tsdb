#include "tsdb.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <time.h>

struct tsdb {
    tsdb_config_t config;
    tsdb_storage_t* storage;
    tsdb_index_t* index;
    tsdb_query_engine_t* query_engine;
    tsdb_cache_t* cache;
    tsdb_write_buffer_t* write_buffer;
    bool is_open;
};

const char* tsdb_version(void) {
    return "1.0.0";
}

tsdb_t* tsdb_open(const char* path, const tsdb_config_t* config) {
    if (!path) return NULL;
    
    tsdb_t* db = (tsdb_t*)calloc(1, sizeof(tsdb_t));
    if (!db) return NULL;
    
    if (config) {
        db->config = *config;
    } else {
        db->config = tsdb_default_config();
    }
    
    strncpy(db->config.data_dir, path, TSDB_MAX_PATH_LEN - 1);
    
    db->storage = tsdb_storage_create(&db->config);
    if (!db->storage) {
        free(db);
        return NULL;
    }
    
    tsdb_status_t status = tsdb_storage_open(db->storage, path);
    if (status != TSDB_OK) {
        tsdb_storage_destroy(db->storage);
        free(db);
        return NULL;
    }
    
    db->index = tsdb_index_create();
    if (!db->index) {
        tsdb_storage_destroy(db->storage);
        free(db);
        return NULL;
    }
    
    char index_path[TSDB_MAX_PATH_LEN];
    snprintf(index_path, sizeof(index_path), "%s/index.db", path);
    tsdb_index_load(db->index, index_path);
    
    db->query_engine = tsdb_query_engine_create(db->storage, db->index);
    if (!db->query_engine) {
        tsdb_index_destroy(db->index);
        tsdb_storage_destroy(db->storage);
        free(db);
        return NULL;
    }
    
    db->cache = tsdb_cache_create(db->config.cache_size_mb * 1024 * 1024);
    if (!db->cache) {
        tsdb_query_engine_destroy(db->query_engine);
        tsdb_index_destroy(db->index);
        tsdb_storage_destroy(db->storage);
        free(db);
        return NULL;
    }
    
    db->write_buffer = tsdb_write_buffer_create(10000);
    if (!db->write_buffer) {
        tsdb_cache_destroy(db->cache);
        tsdb_query_engine_destroy(db->query_engine);
        tsdb_index_destroy(db->index);
        tsdb_storage_destroy(db->storage);
        free(db);
        return NULL;
    }
    
    db->is_open = true;
    
    return db;
}

tsdb_status_t tsdb_close(tsdb_t* db) {
    if (!db) return TSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB_ERR_INVALID_STATE;
    
    tsdb_flush(db);
    
    char index_path[TSDB_MAX_PATH_LEN];
    snprintf(index_path, sizeof(index_path), "%s/index.db", db->config.data_dir);
    tsdb_index_persist(db->index, index_path);
    
    tsdb_write_buffer_destroy(db->write_buffer);
    tsdb_cache_destroy(db->cache);
    tsdb_query_engine_destroy(db->query_engine);
    tsdb_index_destroy(db->index);
    tsdb_storage_destroy(db->storage);
    
    db->is_open = false;
    free(db);
    
    return TSDB_OK;
}

tsdb_status_t tsdb_write(tsdb_t* db, const tsdb_point_t* point) {
    if (!db || !point) return TSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB_ERR_INVALID_STATE;
    
    tsdb_status_t status = tsdb_storage_write_point(db->storage, point);
    if (status != TSDB_OK) return status;
    
    uint64_t series_id = tsdb_storage_get_series_id(point);
    
    tsdb_series_info_t* existing = tsdb_index_get_series(db->index, series_id);
    if (!existing) {
        tsdb_series_info_t info;
        memset(&info, 0, sizeof(info));
        info.series_id = series_id;
        strncpy(info.measurement, point->measurement, sizeof(info.measurement) - 1);
        memcpy(info.tags, point->tags, sizeof(tsdb_tag_t) * point->tags_count);
        info.tags_count = point->tags_count;
        info.min_time = point->timestamp;
        info.max_time = point->timestamp;
        info.point_count = 1;
        
        tsdb_index_add_series(db->index, &info);
    } else {
        tsdb_index_update_stats(db->index, series_id, point->timestamp, 1);
    }
    
    return TSDB_OK;
}

tsdb_status_t tsdb_write_batch(tsdb_t* db, const tsdb_point_t* points, size_t count) {
    if (!db || !points) return TSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB_ERR_INVALID_STATE;
    
    for (size_t i = 0; i < count; i++) {
        tsdb_status_t status = tsdb_write(db, &points[i]);
        if (status != TSDB_OK) return status;
    }
    
    return TSDB_OK;
}

tsdb_status_t tsdb_query_data(tsdb_t* db, const tsdb_query_builder_t* query, tsdb_result_set_t* result) {
    if (!db || !query || !result) return TSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB_ERR_INVALID_STATE;
    
    return tsdb_query_execute(db->query_engine, query, result);
}

tsdb_status_t tsdb_query_agg(tsdb_t* db, const tsdb_query_builder_t* query, tsdb_agg_result_set_t* result) {
    if (!db || !query || !result) return TSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB_ERR_INVALID_STATE;
    
    return tsdb_query_aggregate(db->query_engine, query, result);
}

tsdb_status_t tsdb_delete_series(tsdb_t* db, uint64_t series_id) {
    if (!db) return TSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB_ERR_INVALID_STATE;
    
    return tsdb_index_remove_series(db->index, series_id);
}

tsdb_status_t tsdb_delete_measurement(tsdb_t* db, const char* measurement) {
    if (!db || !measurement) return TSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB_ERR_INVALID_STATE;
    
    tsdb_series_list_t list;
    tsdb_status_t status = tsdb_index_find_by_measurement(db->index, measurement, &list);
    if (status != TSDB_OK) return status;
    
    for (size_t i = 0; i < list.count; i++) {
        tsdb_index_remove_series(db->index, list.series_ids[i]);
    }
    
    free(list.series_ids);
    return TSDB_OK;
}

tsdb_status_t tsdb_flush(tsdb_t* db) {
    if (!db) return TSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB_ERR_INVALID_STATE;
    
    return tsdb_storage_flush(db->storage);
}

tsdb_status_t tsdb_compact(tsdb_t* db) {
    if (!db) return TSDB_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB_ERR_INVALID_STATE;
    
    return tsdb_storage_compact(db->storage);
}

tsdb_stats_t tsdb_get_stats(tsdb_t* db) {
    tsdb_stats_t stats = {0};
    
    if (db && db->is_open) {
        stats.total_series = tsdb_index_series_count(db->index);
        stats.total_measurements = tsdb_index_measurement_count(db->index);
        stats.cache_stats = tsdb_cache_get_stats(db->cache);
        stats.cache_size = stats.cache_stats.current_size;
    }
    
    return stats;
}

double tsdb_get_cache_hit_rate(tsdb_t* db) {
    if (!db || !db->cache) return 0.0;
    return tsdb_cache_hit_rate(db->cache);
}

tsdb_index_t* tsdb_get_index(tsdb_t* db) {
    return db ? db->index : NULL;
}

tsdb_storage_t* tsdb_get_storage(tsdb_t* db) {
    return db ? db->storage : NULL;
}

tsdb_point_t* tsdb_point_create(const char* measurement, tsdb_timestamp_t timestamp) {
    if (!measurement) return NULL;
    
    tsdb_point_t* point = (tsdb_point_t*)calloc(1, sizeof(tsdb_point_t));
    if (!point) return NULL;
    
    strncpy(point->measurement, measurement, sizeof(point->measurement) - 1);
    point->timestamp = timestamp > 0 ? timestamp : (tsdb_timestamp_t)time(NULL) * 1000000000;
    
    return point;
}

void tsdb_point_destroy(tsdb_point_t* point) {
    free(point);
}

tsdb_point_t* tsdb_point_add_tag(tsdb_point_t* point, const char* key, const char* value) {
    if (!point || !key || !value) return point;
    if (point->tags_count >= TSDB_MAX_TAGS_COUNT) return point;
    
    strncpy(point->tags[point->tags_count].key, key, TSDB_MAX_TAG_KEY_LEN - 1);
    strncpy(point->tags[point->tags_count].value, value, TSDB_MAX_TAG_VALUE_LEN - 1);
    point->tags_count++;
    
    return point;
}

tsdb_point_t* tsdb_point_add_field_float(tsdb_point_t* point, const char* name, double value) {
    if (!point || !name) return point;
    if (point->fields_count >= TSDB_MAX_FIELDS_COUNT) return point;
    
    strncpy(point->fields[point->fields_count].name, name, TSDB_MAX_FIELD_NAME_LEN - 1);
    point->fields[point->fields_count].type = TSDB_FIELD_FLOAT;
    point->fields[point->fields_count].value.float_val = value;
    point->fields_count++;
    
    return point;
}

tsdb_point_t* tsdb_point_add_field_int(tsdb_point_t* point, const char* name, int64_t value) {
    if (!point || !name) return point;
    if (point->fields_count >= TSDB_MAX_FIELDS_COUNT) return point;
    
    strncpy(point->fields[point->fields_count].name, name, TSDB_MAX_FIELD_NAME_LEN - 1);
    point->fields[point->fields_count].type = TSDB_FIELD_INTEGER;
    point->fields[point->fields_count].value.int_val = value;
    point->fields_count++;
    
    return point;
}

tsdb_point_t* tsdb_point_add_field_string(tsdb_point_t* point, const char* name, const char* value) {
    if (!point || !name || !value) return point;
    if (point->fields_count >= TSDB_MAX_FIELDS_COUNT) return point;
    
    strncpy(point->fields[point->fields_count].name, name, TSDB_MAX_FIELD_NAME_LEN - 1);
    point->fields[point->fields_count].type = TSDB_FIELD_STRING;
    strncpy(point->fields[point->fields_count].value.str_val, value, 255);
    point->fields_count++;
    
    return point;
}

tsdb_point_t* tsdb_point_add_field_bool(tsdb_point_t* point, const char* name, bool value) {
    if (!point || !name) return point;
    if (point->fields_count >= TSDB_MAX_FIELDS_COUNT) return point;
    
    strncpy(point->fields[point->fields_count].name, name, TSDB_MAX_FIELD_NAME_LEN - 1);
    point->fields[point->fields_count].type = TSDB_FIELD_BOOLEAN;
    point->fields[point->fields_count].value.bool_val = value;
    point->fields_count++;
    
    return point;
}

void tsdb_result_set_destroy(tsdb_result_set_t* result) {
    if (!result) return;
    
    free(result->points);
    result->points = NULL;
    result->count = 0;
    result->capacity = 0;
}

void tsdb_agg_result_set_destroy(tsdb_agg_result_set_t* result) {
    if (!result) return;
    
    free(result->results);
    result->results = NULL;
    result->count = 0;
    result->capacity = 0;
}

void tsdb_series_list_destroy(tsdb_series_list_t* list) {
    if (!list) return;
    
    free(list->series_ids);
    list->series_ids = NULL;
    list->count = 0;
    list->capacity = 0;
}
