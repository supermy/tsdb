#include "rtsdb_storage.h"
#include <rocksdb/c.h>

#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <sys/stat.h>

struct rtsdb_storage {
    char              base_path[RTSDB_MAX_PATH_LEN];
    rtsdb_config_t    config;
    
    rocksdb_t*        db;
    rocksdb_options_t* options;
    rocksdb_writeoptions_t* write_options;
    rocksdb_readoptions_t* read_options;
    
    uint64_t          total_points;
    bool              is_open;
};

static uint64_t fnv1a_hash64(const void* data, size_t len) {
    const uint8_t* bytes = (const uint8_t*)data;
    uint64_t hash = 14695981039346656037ULL;
    for (size_t i = 0; i < len; i++) {
        hash ^= bytes[i];
        hash *= 1099511628211ULL;
    }
    return hash;
}

uint64_t rtsdb_storage_get_series_id(const rtsdb_point_t* point) {
    uint64_t hash = fnv1a_hash64(point->measurement, strlen(point->measurement));
    for (size_t i = 0; i < point->tags_count; i++) {
        hash ^= fnv1a_hash64(point->tags[i].key, strlen(point->tags[i].key));
        hash ^= fnv1a_hash64(point->tags[i].value, strlen(point->tags[i].value));
    }
    return hash;
}

static void make_data_key(char* buffer, size_t buf_size, const char* measurement,
                          uint64_t series_id, rtsdb_timestamp_t ts) {
    snprintf(buffer, buf_size, "D:%s:%lu:%lld", measurement, 
             (unsigned long)series_id, (long long)ts);
}

static void make_series_key(char* buffer, size_t buf_size, uint64_t series_id) {
    snprintf(buffer, buf_size, "S:%lu", (unsigned long)series_id);
}

rtsdb_storage_t* rtsdb_storage_create(const rtsdb_config_t* config) {
    rtsdb_storage_t* s = (rtsdb_storage_t*)calloc(1, sizeof(rtsdb_storage_t));
    if (!s) return NULL;
    
    s->config = *config;
    return s;
}

void rtsdb_storage_destroy(rtsdb_storage_t* storage) {
    if (!storage) return;
    rtsdb_storage_close(storage);
    free(storage);
}

rtsdb_status_t rtsdb_storage_open(rtsdb_storage_t* storage, const char* path) {
    if (!storage || !path) return RTSDB_ERR_INVALID_PARAM;
    
    strncpy(storage->base_path, path, RTSDB_MAX_PATH_LEN - 1);
    
    struct stat st = {0};
    if (stat(path, &st) == -1) {
        if (mkdir(path, 0755) != 0) return RTSDB_ERR_IO;
    }
    
    storage->options = rocksdb_options_create();
    rocksdb_options_set_create_if_missing(storage->options, 1);
    rocksdb_options_set_max_open_files(storage->options, (int)storage->config.max_open_files);
    rocksdb_options_set_write_buffer_size(storage->options, storage->config.write_buffer_mb * 1024 * 1024);
    rocksdb_options_set_max_bytes_for_level_base(storage->options, storage->config.max_bytes_for_level_base);
    rocksdb_options_set_num_levels(storage->options, storage->config.num_levels);
    
    if (storage->config.enable_compression) {
        rocksdb_options_set_compression(storage->options, rocksdb_snappy_compression);
    }
    
    char* err = NULL;
    storage->db = rocksdb_open(storage->options, path, &err);
    
    if (err) {
        free(err);
        rocksdb_options_destroy(storage->options);
        return RTSDB_ERR_IO;
    }
    
    storage->write_options = rocksdb_writeoptions_create();
    storage->read_options = rocksdb_readoptions_create();
    storage->is_open = true;
    
    return RTSDB_OK;
}

rtsdb_status_t rtsdb_storage_close(rtsdb_storage_t* storage) {
    if (!storage) return RTSDB_ERR_INVALID_PARAM;
    
    if (storage->db) {
        rocksdb_close(storage->db);
        storage->db = NULL;
    }
    if (storage->options) {
        rocksdb_options_destroy(storage->options);
        storage->options = NULL;
    }
    if (storage->write_options) {
        rocksdb_writeoptions_destroy(storage->write_options);
        storage->write_options = NULL;
    }
    if (storage->read_options) {
        rocksdb_readoptions_destroy(storage->read_options);
        storage->read_options = NULL;
    }
    
    storage->is_open = false;
    return RTSDB_OK;
}

rtsdb_status_t rtsdb_storage_write_point(rtsdb_storage_t* storage, const rtsdb_point_t* point) {
    if (!storage || !point || !storage->is_open) return RTSDB_ERR_INVALID_PARAM;
    
    char data_key[512];
    uint64_t series_id = rtsdb_storage_get_series_id(point);
    make_data_key(data_key, sizeof(data_key), point->measurement, series_id, point->timestamp);
    
    rtsdb_value_t value = 0.0;
    if (point->fields_count > 0) {
        if (point->fields[0].type == RTSDB_FIELD_FLOAT) {
            value = point->fields[0].value.float_val;
        } else if (point->fields[0].type == RTSDB_FIELD_INTEGER) {
            value = (rtsdb_value_t)point->fields[0].value.int_val;
        }
    }
    
    char value_buf[64];
    snprintf(value_buf, sizeof(value_buf), "%.15g", value);
    
    char* err = NULL;
    rocksdb_put(storage->db, storage->write_options,
                data_key, strlen(data_key),
                value_buf, strlen(value_buf), &err);
    
    if (err) {
        free(err);
        return RTSDB_ERR_IO;
    }
    
    storage->total_points++;
    return RTSDB_OK;
}

rtsdb_status_t rtsdb_storage_write_batch(rtsdb_storage_t* storage, const rtsdb_point_t* points, size_t count) {
    if (!storage || !points || !storage->is_open) return RTSDB_ERR_INVALID_PARAM;
    
    rocksdb_writebatch_t* batch = rocksdb_writebatch_create();
    
    for (size_t i = 0; i < count; i++) {
        const rtsdb_point_t* point = &points[i];
        
        char data_key[512];
        uint64_t series_id = rtsdb_storage_get_series_id(point);
        make_data_key(data_key, sizeof(data_key), point->measurement, series_id, point->timestamp);
        
        rtsdb_value_t value = 0.0;
        if (point->fields_count > 0) {
            if (point->fields[0].type == RTSDB_FIELD_FLOAT) {
                value = point->fields[0].value.float_val;
            } else if (point->fields[0].type == RTSDB_FIELD_INTEGER) {
                value = (rtsdb_value_t)point->fields[0].value.int_val;
            }
        }
        
        char value_buf[64];
        snprintf(value_buf, sizeof(value_buf), "%.15g", value);
        
        rocksdb_writebatch_put(batch, data_key, strlen(data_key),
                               value_buf, strlen(value_buf));
    }
    
    char* err = NULL;
    rocksdb_write(storage->db, storage->write_options, batch, &err);
    rocksdb_writebatch_destroy(batch);
    
    if (err) {
        free(err);
        return RTSDB_ERR_IO;
    }
    
    storage->total_points += count;
    return RTSDB_OK;
}

rtsdb_status_t rtsdb_storage_read_series(rtsdb_storage_t* storage, const char* measurement,
                                          rtsdb_time_range_t* range, size_t limit,
                                          rtsdb_result_set_t* result) {
    if (!storage || !measurement || !result || !storage->is_open) return RTSDB_ERR_INVALID_PARAM;
    
    memset(result, 0, sizeof(rtsdb_result_set_t));
    result->capacity = limit > 0 ? limit : 1000;
    result->points = (rtsdb_point_t*)calloc(result->capacity, sizeof(rtsdb_point_t));
    if (!result->points) return RTSDB_ERR_NO_MEMORY;
    
    rocksdb_iterator_t* iter = rocksdb_create_iterator(storage->db, storage->read_options);
    
    char prefix[256];
    snprintf(prefix, sizeof(prefix), "D:%s:", measurement);
    size_t prefix_len = strlen(prefix);
    
    rocksdb_iter_seek(iter, prefix, prefix_len);
    
    while (rocksdb_iter_valid(iter) && result->count < limit) {
        size_t key_len;
        const char* key = rocksdb_iter_key(iter, &key_len);
        
        if (key_len < prefix_len || memcmp(key, prefix, prefix_len) != 0) {
            break;
        }
        
        size_t val_len;
        const char* val = rocksdb_iter_value(iter, &val_len);
        
        if (result->count >= result->capacity) {
            result->capacity *= 2;
            rtsdb_point_t* new_pts = (rtsdb_point_t*)realloc(result->points, result->capacity * sizeof(rtsdb_point_t));
            if (!new_pts) {
                rocksdb_iter_destroy(iter);
                return RTSDB_ERR_NO_MEMORY;
            }
            result->points = new_pts;
        }
        
        rtsdb_point_t* p = &result->points[result->count];
        strncpy(p->measurement, measurement, RTSDB_MAX_MEASUREMENT_LEN - 1);
        p->timestamp = 0;
        
        const char* key_str = (const char*)key;
        char* last_colon = strrchr(key_str, ':');
        if (last_colon) {
            char* end;
            p->timestamp = strtoll(last_colon + 1, &end, 10);
        }
        
        if (!range || (p->timestamp >= range->start && p->timestamp <= range->end)) {
            p->fields_count = 1;
            p->fields[0].type = RTSDB_FIELD_FLOAT;
            p->fields[0].value.float_val = atof(val);
            result->count++;
        }
        
        rocksdb_iter_next(iter);
    }
    
    rocksdb_iter_destroy(iter);
    return RTSDB_OK;
}

rtsdb_status_t rtsdb_storage_delete_measurement(rtsdb_storage_t* storage, const char* measurement) {
    if (!storage || !measurement || !storage->is_open) return RTSDB_ERR_INVALID_PARAM;
    
    rocksdb_writebatch_t* batch = rocksdb_writebatch_create();
    rocksdb_iterator_t* iter = rocksdb_create_iterator(storage->db, storage->read_options);
    
    char prefix[256];
    snprintf(prefix, sizeof(prefix), "D:%s:", measurement);
    size_t prefix_len = strlen(prefix);
    
    rocksdb_iter_seek(iter, prefix, prefix_len);
    
    while (rocksdb_iter_valid(iter)) {
        size_t key_len;
        const char* key = rocksdb_iter_key(iter, &key_len);
        
        if (key_len < prefix_len || memcmp(key, prefix, prefix_len) != 0) {
            break;
        }
        
        rocksdb_writebatch_delete(batch, key, key_len);
        rocksdb_iter_next(iter);
    }
    
    rocksdb_iter_destroy(iter);
    
    char* err = NULL;
    rocksdb_write(storage->db, storage->write_options, batch, &err);
    rocksdb_writebatch_destroy(batch);
    
    if (err) {
        free(err);
        return RTSDB_ERR_IO;
    }
    
    return RTSDB_OK;
}

rtsdb_status_t rtsdb_storage_flush(rtsdb_storage_t* storage) {
    if (!storage) return RTSDB_ERR_INVALID_PARAM;
    return RTSDB_OK;
}

rtsdb_status_t rtsdb_storage_compact(rtsdb_storage_t* storage) {
    if (!storage || !storage->is_open) return RTSDB_ERR_INVALID_PARAM;
    rocksdb_compact_range(storage->db, NULL, 0, NULL, 0);
    return RTSDB_OK;
}

size_t rtsdb_storage_total_points(rtsdb_storage_t* storage) {
    return storage ? storage->total_points : 0;
}
