#include "tsdb25.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <time.h>

struct tsdb25 {
    tsdb25_config_t       config;
    tsdb25_storage_t*    storage;
    tsdb25_index_t*      index;
    tsdb25_query_engine_t* query_engine;
    tsdb25_cache_t*      cache;
    tsdb25_wal_t*        wal;
    tsdb25_write_buf_t*  write_buf;
    bool                 is_open;
};

const char* tsdb25_version(void) {
    return TSDB25_VERSION;
}

tsdb25_t* tsdb25_open(const char* path, const tsdb25_config_t* config) {
    if (!path) return NULL;
    
    tsdb25_t* db = (tsdb25_t*)calloc(1, sizeof(tsdb25_t));
    if (!db) return NULL;
    
    if (config) {
        db->config = *config;
    } else {
        db->config = tsdb25_default_config();
    }
    
    strncpy(db->config.data_dir, path, TSDB25_MAX_PATH_LEN - 1);
    
    db->storage = tsdb25_storage_create(&db->config);
    if (!db->storage) {
        free(db);
        return NULL;
    }
    
    tsdb25_status_t status = tsdb25_storage_open(db->storage, path);
    if (status != TSDB25_OK) {
        tsdb25_storage_destroy(db->storage);
        free(db);
        return NULL;
    }
    
    db->index = tsdb25_index_create();
    if (!db->index) {
        tsdb25_storage_destroy(db->storage);
        free(db);
        return NULL;
    }
    
    char index_path[TSDB25_MAX_PATH_LEN];
    snprintf(index_path, sizeof(index_path), "%s/index.db", path);
    tsdb25_index_load(db->index, index_path);
    
    db->query_engine = tsdb25_query_engine_create(db->storage, db->index);
    if (!db->query_engine) {
        tsdb25_index_destroy(db->index);
        tsdb25_storage_destroy(db->storage);
        free(db);
        return NULL;
    }
    
    db->cache = tsdb25_cache_create(db->config.cache_size_mb * 1024 * 1024);
    if (!db->cache) {
        tsdb25_query_engine_destroy(db->query_engine);
        tsdb25_index_destroy(db->index);
        tsdb25_storage_destroy(db->storage);
        free(db);
        return NULL;
    }
    
    if (db->config.enable_wal) {
        char wal_path[TSDB25_MAX_PATH_LEN];
        snprintf(wal_path, sizeof(wal_path), "%s/wal.db", path);
        db->wal = tsdb25_wal_create(wal_path, db->config.wal_size_mb * 1024 * 1024);
        if (db->wal) {
            tsdb25_wal_open(db->wal);
        }
    }
    
    db->write_buf = tsdb25_write_buf_create(10000);
    if (!db->write_buf) {
        if (db->wal) tsdb25_wal_destroy(db->wal);
        tsdb25_cache_destroy(db->cache);
        tsdb25_query_engine_destroy(db->query_engine);
        tsdb25_index_destroy(db->index);
        tsdb25_storage_destroy(db->storage);
        free(db);
        return NULL;
    }
    
    db->is_open = true;
    return db;
}

tsdb25_status_t tsdb25_close(tsdb25_t* db) {
    if (!db) return TSDB25_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB25_ERR_INVALID_STATE;
    
    tsdb25_flush(db);
    
    char index_path[TSDB25_MAX_PATH_LEN];
    snprintf(index_path, sizeof(index_path), "%s/index.db", db->config.data_dir);
    tsdb25_index_persist(db->index, index_path);
    
    tsdb25_write_buf_destroy(db->write_buf);
    if (db->wal) tsdb25_wal_destroy(db->wal);
    tsdb25_cache_destroy(db->cache);
    tsdb25_query_engine_destroy(db->query_engine);
    tsdb25_index_destroy(db->index);
    tsdb25_storage_destroy(db->storage);
    
    db->is_open = false;
    free(db);
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_write(tsdb25_t* db, const tsdb25_point_t* point) {
    if (!db || !point) return TSDB25_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB25_ERR_INVALID_STATE;
    
    if (db->wal) {
        tsdb25_wal_append(db->wal, point);
    }
    
    tsdb25_status_t status = tsdb25_storage_write_point(db->storage, point);
    if (status != TSDB25_OK) return status;
    
    uint64_t series_id = tsdb25_storage_get_series_id(point);
    tsdb25_series_t* existing = tsdb25_index_get_series(db->index, series_id);
    if (!existing) {
        tsdb25_series_t series;
        memset(&series, 0, sizeof(series));
        series.series_id = series_id;
        strncpy(series.measurement, point->measurement, sizeof(series.measurement) - 1);
        memcpy(series.tags, point->tags, sizeof(tsdb25_tag_t) * point->tags_count);
        series.tags_count = point->tags_count;
        series.min_time = point->timestamp;
        series.max_time = point->timestamp;
        series.point_count = 1;
        tsdb25_index_add_series(db->index, &series);
    } else {
        tsdb25_index_update_stats(db->index, series_id, point->timestamp, 1);
    }
    
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_write_batch(tsdb25_t* db, const tsdb25_point_t* points, size_t count) {
    if (!db || !points) return TSDB25_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB25_ERR_INVALID_STATE;
    
    for (size_t i = 0; i < count; i++) {
        tsdb25_status_t status = tsdb25_write(db, &points[i]);
        if (status != TSDB25_OK) return status;
    }
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_query_select(tsdb25_t* db, const tsdb25_query_t* query, tsdb25_result_set_t* result) {
    if (!db || !query || !result) return TSDB25_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB25_ERR_INVALID_STATE;
    return tsdb25_query_execute(db->query_engine, query, result);
}

tsdb25_status_t tsdb25_query_aggregate(tsdb25_t* db, const tsdb25_query_t* query, tsdb25_agg_result_set_t* result) {
    if (!db || !query || !result) return TSDB25_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB25_ERR_INVALID_STATE;
    return tsdb25_query_aggregate_engine(db->query_engine, query, result);
}

tsdb25_status_t tsdb25_delete_series(tsdb25_t* db, uint64_t series_id) {
    if (!db) return TSDB25_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB25_ERR_INVALID_STATE;
    return tsdb25_index_remove_series(db->index, series_id);
}

tsdb25_status_t tsdb25_delete_measurement(tsdb25_t* db, const char* measurement) {
    if (!db || !measurement) return TSDB25_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB25_ERR_INVALID_STATE;
    
    tsdb25_series_list_t list;
    tsdb25_status_t status = tsdb25_index_find_by_measurement(db->index, measurement, &list);
    if (status != TSDB25_OK) return status;
    
    for (size_t i = 0; i < list.count; i++) {
        tsdb25_index_remove_series(db->index, list.series_ids[i]);
    }
    tsdb25_series_list_destroy(&list);
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_flush(tsdb25_t* db) {
    if (!db) return TSDB25_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB25_ERR_INVALID_STATE;
    
    tsdb25_storage_flush(db->storage);
    if (db->wal) tsdb25_wal_flush(db->wal);
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_compact(tsdb25_t* db) {
    if (!db) return TSDB25_ERR_INVALID_PARAM;
    if (!db->is_open) return TSDB25_ERR_INVALID_STATE;
    return tsdb25_storage_compact(db->storage);
}

tsdb25_stats_t tsdb25_stats(tsdb25_t* db) {
    tsdb25_stats_t s = {0};
    if (db && db->is_open) {
        s.total_series = tsdb25_index_series_count(db->index);
        s.total_measurements = tsdb25_index_measurement_count(db->index);
        s.cache_stats = tsdb25_cache_stats(db->cache);
        s.cache_size = s.cache_stats.current_size;
    }
    return s;
}

tsdb25_index_t* tsdb25_get_index(tsdb25_t* db) {
    return db ? db->index : NULL;
}

tsdb25_storage_t* tsdb25_get_storage(tsdb25_t* db) {
    return db ? db->storage : NULL;
}

tsdb25_cache_t* tsdb25_get_cache(tsdb25_t* db) {
    return db ? db->cache : NULL;
}

tsdb25_wal_t* tsdb25_get_wal(tsdb25_t* db) {
    return db ? db->wal : NULL;
}

tsdb25_point_t* tsdb25_point_new(const char* measurement, tsdb25_timestamp_t timestamp) {
    if (!measurement) return NULL;
    
    tsdb25_point_t* p = (tsdb25_point_t*)calloc(1, sizeof(tsdb25_point_t));
    if (!p) return NULL;
    
    strncpy(p->measurement, measurement, sizeof(p->measurement) - 1);
    p->timestamp = timestamp > 0 ? timestamp : (tsdb25_timestamp_t)time(NULL) * 1000000000LL;
    return p;
}

void tsdb25_point_free(tsdb25_point_t* point) {
    free(point);
}

tsdb25_point_t* tsdb25_point_tag(tsdb25_point_t* point, const char* key, const char* value) {
    if (!point || !key || !value) return point;
    if (point->tags_count >= TSDB25_MAX_TAGS_COUNT) return point;
    
    strncpy(point->tags[point->tags_count].key, key, TSDB25_MAX_TAG_KEY_LEN - 1);
    strncpy(point->tags[point->tags_count].value, value, TSDB25_MAX_TAG_VALUE_LEN - 1);
    point->tags_count++;
    return point;
}

tsdb25_point_t* tsdb25_point_field_f64(tsdb25_point_t* point, const char* name, double value) {
    if (!point || !name) return point;
    if (point->fields_count >= TSDB25_MAX_FIELDS_COUNT) return point;
    
    strncpy(point->fields[point->fields_count].name, name, TSDB25_MAX_FIELD_NAME_LEN - 1);
    point->fields[point->fields_count].type = TSDB25_FIELD_FLOAT;
    point->fields[point->fields_count].value.float_val = value;
    point->fields_count++;
    return point;
}

tsdb25_point_t* tsdb25_point_field_i64(tsdb25_point_t* point, const char* name, int64_t value) {
    if (!point || !name) return point;
    if (point->fields_count >= TSDB25_MAX_FIELDS_COUNT) return point;
    
    strncpy(point->fields[point->fields_count].name, name, TSDB25_MAX_FIELD_NAME_LEN - 1);
    point->fields[point->fields_count].type = TSDB25_FIELD_INTEGER;
    point->fields[point->fields_count].value.int_val = value;
    point->fields_count++;
    return point;
}

tsdb25_point_t* tsdb25_point_field_str(tsdb25_point_t* point, const char* name, const char* value) {
    if (!point || !name || !value) return point;
    if (point->fields_count >= TSDB25_MAX_FIELDS_COUNT) return point;
    
    strncpy(point->fields[point->fields_count].name, name, TSDB25_MAX_FIELD_NAME_LEN - 1);
    point->fields[point->fields_count].type = TSDB25_FIELD_STRING;
    strncpy(point->fields[point->fields_count].value.str_val, value, 511);
    point->fields_count++;
    return point;
}

tsdb25_point_t* tsdb25_point_field_bool(tsdb25_point_t* point, const char* name, bool value) {
    if (!point || !name) return point;
    if (point->fields_count >= TSDB25_MAX_FIELDS_COUNT) return point;
    
    strncpy(point->fields[point->fields_count].name, name, TSDB25_MAX_FIELD_NAME_LEN - 1);
    point->fields[point->fields_count].type = TSDB25_FIELD_BOOLEAN;
    point->fields[point->fields_count].value.bool_val = value;
    point->fields_count++;
    return point;
}

void tsdb25_result_free(tsdb25_result_set_t* result) {
    if (!result) return;
    free(result->points);
    result->points = NULL;
    result->count = 0;
    result->capacity = 0;
}

void tsdb25_agg_result_free(tsdb25_agg_result_set_t* result) {
    if (!result) return;
    free(result->results);
    result->results = NULL;
    result->count = 0;
    result->capacity = 0;
}
