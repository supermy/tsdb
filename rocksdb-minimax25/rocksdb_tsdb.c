#include "rocksdb_tsdb.h"
#include <rocksdb/c.h>

#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <time.h>
#include <math.h>
#include <sys/stat.h>

struct tsdb {
    char path[512];
    rocksdb_config_t config;
    
    rocksdb_t* db;
    rocksdb_options_t* options;
    rocksdb_writeoptions_t* write_options;
    rocksdb_readoptions_t* read_options;
    
    uint64_t total_points;
    uint64_t total_series;
    
    bool is_open;
};

static uint64_t fnv1a_hash(const void* data, size_t len) {
    const uint8_t* bytes = data;
    uint64_t hash = 14695981039346656037ULL;
    for (size_t i = 0; i < len; i++) {
        hash ^= bytes[i];
        hash *= 1099511628211ULL;
    }
    return hash;
}

static uint64_t make_series_key(const point_t* point) {
    uint64_t h = fnv1a_hash(point->measurement, strlen(point->measurement));
    for (size_t i = 0; i < point->tag_count; i++) {
        h ^= fnv1a_hash(point->tags[i].key, strlen(point->tags[i].key));
        h ^= fnv1a_hash(point->tags[i].value, strlen(point->tags[i].value));
    }
    return h;
}

static void make_data_key(char* buffer, size_t buf_size, const char* measurement,
                          uint64_t series_id, ts_timestamp_t ts) {
    snprintf(buffer, buf_size, "%s_%lu_%lld", measurement, 
             (unsigned long)series_id, (long long)ts);
}

const char* rocksdb_tsdb_version(void) {
    return ROCKSDB_TSDB_VERSION;
}

const char* rocksdb_strerror(rocksdb_error_t err) {
    switch (err) {
        case ROCKSDB_OK: return "Success";
        case ROCKSDB_ERR_INVALID_PARAM: return "Invalid parameter";
        case ROCKSDB_ERR_NO_MEMORY: return "Out of memory";
        case ROCKSDB_ERR_NOT_FOUND: return "Not found";
        case ROCKSDB_ERR_IO: return "I/O error";
        case ROCKSDB_ERR_CORRUPTED: return "Data corrupted";
        case ROCKSDB_ERR_EXISTS: return "Already exists";
        case ROCKSDB_ERR_TIMEOUT: return "Timeout";
        case ROCKSDB_ERR_FULL: return "Storage full";
        default: return "Unknown error";
    }
}

rocksdb_config_t rocksdb_default_config(void) {
    rocksdb_config_t cfg;
    memset(&cfg, 0, sizeof(cfg));
    cfg.cache_size_mb = 256;
    cfg.max_open_files = 10000;
    cfg.write_buffer_mb = 64;
    cfg.max_bytes_for_level_base = 256 * 1024 * 1024;
    cfg.num_levels = 7;
    cfg.compression = true;
    cfg.compression_level = 6;
    cfg.direct_io = false;
    cfg.block_size = 4096;
    cfg.bloom_filter = true;
    cfg.bloom_bits_per_key = 10;
    return cfg;
}

static rocksdb_error_t init_rocksdb(rocksdb_tsdb_t* tsdb, const char* path) {
    rocksdb_options_t* options = rocksdb_options_create();
    
    rocksdb_options_set_create_if_missing(options, 1);
    
    rocksdb_options_set_max_open_files(options, (int)tsdb->config.max_open_files);
    rocksdb_options_set_write_buffer_size(options, tsdb->config.write_buffer_mb * 1024 * 1024);
    rocksdb_options_set_max_bytes_for_level_base(options, tsdb->config.max_bytes_for_level_base);
    rocksdb_options_set_num_levels(options, tsdb->config.num_levels);
    
    if (tsdb->config.compression) {
        rocksdb_options_set_compression(options, rocksdb_snappy_compression);
    }
    
    char* err = NULL;
    rocksdb_t* db = rocksdb_open(options, path, &err);
    
    if (err) {
        fprintf(stderr, "RocksDB open error: %s\n", err);
        free(err);
        rocksdb_options_destroy(options);
        return ROCKSDB_ERR_IO;
    }
    
    tsdb->db = db;
    tsdb->options = options;
    tsdb->write_options = rocksdb_writeoptions_create();
    tsdb->read_options = rocksdb_readoptions_create();
    
    return ROCKSDB_OK;
}

rocksdb_tsdb_t* rocksdb_tsdb_open(const char* path, const rocksdb_config_t* config) {
    if (!path) return NULL;
    
    struct stat st;
    if (stat(path, &st) != 0) {
        if (mkdir(path, 0755) != 0) return NULL;
    }
    
    rocksdb_tsdb_t* tsdb = calloc(1, sizeof(rocksdb_tsdb_t));
    if (!tsdb) return NULL;
    
    strncpy(tsdb->path, path, sizeof(tsdb->path) - 1);
    
    if (config) {
        tsdb->config = *config;
    } else {
        tsdb->config = rocksdb_default_config();
    }
    
    rocksdb_error_t err = init_rocksdb(tsdb, path);
    if (err != ROCKSDB_OK) {
        free(tsdb);
        return NULL;
    }
    
    tsdb->is_open = true;
    return tsdb;
}

void rocksdb_tsdb_close(rocksdb_tsdb_t* tsdb) {
    if (!tsdb) return;
    
    if (tsdb->db) rocksdb_close(tsdb->db);
    if (tsdb->options) rocksdb_options_destroy(tsdb->options);
    if (tsdb->write_options) rocksdb_writeoptions_destroy(tsdb->write_options);
    if (tsdb->read_options) rocksdb_readoptions_destroy(tsdb->read_options);
    
    free(tsdb);
}

static rocksdb_error_t write_point_impl(rocksdb_tsdb_t* tsdb, const point_t* point) {
    char data_key[512];
    make_data_key(data_key, sizeof(data_key), point->measurement,
                  make_series_key(point), point->timestamp);
    
    ts_value_t value = 0.0;
    if (point->field_count > 0) {
        if (point->fields[0].type == FIELD_FLOAT) {
            value = point->fields[0].value.f64;
        } else if (point->fields[0].type == FIELD_INT) {
            value = (ts_value_t)point->fields[0].value.i64;
        }
    }
    
    char value_buf[64];
    snprintf(value_buf, sizeof(value_buf), "%.10f", value);
    
    char* err = NULL;
    rocksdb_put(tsdb->db, tsdb->write_options, 
                data_key, strlen(data_key),
                value_buf, strlen(value_buf), &err);
    
    if (err) {
        free(err);
        return ROCKSDB_ERR_IO;
    }
    
    tsdb->total_points++;
    return ROCKSDB_OK;
}

rocksdb_error_t rocksdb_tsdb_write(rocksdb_tsdb_t* tsdb, const point_t* point) {
    if (!tsdb || !point || !tsdb->is_open) return ROCKSDB_ERR_INVALID_PARAM;
    return write_point_impl(tsdb, point);
}

rocksdb_error_t rocksdb_tsdb_write_batch(rocksdb_tsdb_t* tsdb, const point_t* points, size_t count) {
    if (!tsdb || !points || !tsdb->is_open) return ROCKSDB_ERR_INVALID_PARAM;
    
    rocksdb_writebatch_t* batch = rocksdb_writebatch_create();
    
    for (size_t i = 0; i < count; i++) {
        const point_t* point = &points[i];
        
        char data_key[512];
        make_data_key(data_key, sizeof(data_key), point->measurement,
                      make_series_key(point), point->timestamp);
        
        ts_value_t value = 0.0;
        if (point->field_count > 0) {
            if (point->fields[0].type == FIELD_FLOAT) {
                value = point->fields[0].value.f64;
            } else if (point->fields[0].type == FIELD_INT) {
                value = (ts_value_t)point->fields[0].value.i64;
            }
        }
        
        char value_buf[64];
        snprintf(value_buf, sizeof(value_buf), "%.10f", value);
        
        rocksdb_writebatch_put(batch, data_key, strlen(data_key),
                               value_buf, strlen(value_buf));
    }
    
    char* err = NULL;
    rocksdb_write(tsdb->db, tsdb->write_options, batch, &err);
    rocksdb_writebatch_destroy(batch);
    
    if (err) {
        free(err);
        return ROCKSDB_ERR_IO;
    }
    
    tsdb->total_points += count;
    return ROCKSDB_OK;
}

rocksdb_error_t rocksdb_tsdb_query(rocksdb_tsdb_t* tsdb, const char* measurement,
                               const range_t* range, size_t limit, result_t* result) {
    if (!tsdb || !measurement || !result || !tsdb->is_open) return ROCKSDB_ERR_INVALID_PARAM;
    
    memset(result, 0, sizeof(result_t));
    result->capacity = limit > 0 ? limit : 1000;
    result->points = calloc(result->capacity, sizeof(point_t));
    if (!result->points) return ROCKSDB_ERR_NO_MEMORY;
    
    rocksdb_iterator_t* iter = rocksdb_create_iterator(tsdb->db, tsdb->read_options);
    
    char prefix[256];
    snprintf(prefix, sizeof(prefix), "%s_", measurement);
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
            point_t* new_pts = realloc(result->points, result->capacity * sizeof(point_t));
            if (!new_pts) {
                rocksdb_iter_destroy(iter);
                return ROCKSDB_ERR_NO_MEMORY;
            }
            result->points = new_pts;
        }
        
        point_t* p = &result->points[result->count];
        strncpy(p->measurement, measurement, sizeof(p->measurement) - 1);
        p->timestamp = 0;
        
        const char* key_str = (const char*)key;
        char* last_underscore = strrchr(key_str, '_');
        if (last_underscore) {
            char* end;
            p->timestamp = strtoll(last_underscore + 1, &end, 10);
        }
        
        if (!range || (p->timestamp >= range->start && p->timestamp <= range->end)) {
            p->field_count = 1;
            p->fields[0].type = FIELD_FLOAT;
            p->fields[0].value.f64 = atof(val);
            result->count++;
        }
        
        rocksdb_iter_next(iter);
    }
    
    rocksdb_iter_destroy(iter);
    return ROCKSDB_OK;
}

rocksdb_error_t rocksdb_tsdb_query_agg(rocksdb_tsdb_t* tsdb, const char* measurement,
                                   const range_t* range, agg_type_t agg,
                                   const char* field, agg_result_t* result) {
    (void)field;
    
    if (!tsdb || !measurement || !result || !tsdb->is_open) return ROCKSDB_ERR_INVALID_PARAM;
    
    memset(result, 0, sizeof(agg_result_t));
    
    result_t data;
    rocksdb_error_t err = rocksdb_tsdb_query(tsdb, measurement, range, 1000000, &data);
    if (err != ROCKSDB_OK) return err;
    
    if (data.count == 0) {
        return ROCKSDB_OK;
    }
    
    result->count = data.count;
    
    switch (agg) {
        case AGG_COUNT:
            result->value = (ts_value_t)data.count;
            break;
            
        case AGG_SUM:
            for (size_t i = 0; i < data.count; i++) {
                if (data.points[i].field_count > 0) {
                    result->value += data.points[i].fields[0].value.f64;
                }
            }
            break;
            
        case AGG_AVG:
            for (size_t i = 0; i < data.count; i++) {
                if (data.points[i].field_count > 0) {
                    result->value += data.points[i].fields[0].value.f64;
                }
            }
            result->value /= data.count;
            break;
            
        case AGG_MIN:
            result->value = INFINITY;
            for (size_t i = 0; i < data.count; i++) {
                if (data.points[i].field_count > 0) {
                    double v = data.points[i].fields[0].value.f64;
                    if (v < result->value) result->value = v;
                }
            }
            break;
            
        case AGG_MAX:
            result->value = -INFINITY;
            for (size_t i = 0; i < data.count; i++) {
                if (data.points[i].field_count > 0) {
                    double v = data.points[i].fields[0].value.f64;
                    if (v > result->value) result->value = v;
                }
            }
            break;
            
        case AGG_FIRST:
            if (data.points[0].field_count > 0) {
                result->value = data.points[0].fields[0].value.f64;
                result->timestamp = data.points[0].timestamp;
            }
            break;
            
        case AGG_LAST:
            if (data.points[data.count - 1].field_count > 0) {
                result->value = data.points[data.count - 1].fields[0].value.f64;
                result->timestamp = data.points[data.count - 1].timestamp;
            }
            break;
            
        case AGG_STDDEV: {
            double sum = 0, sum_sq = 0;
            for (size_t i = 0; i < data.count; i++) {
                if (data.points[i].field_count > 0) {
                    double v = data.points[i].fields[0].value.f64;
                    sum += v;
                    sum_sq += v * v;
                }
            }
            double mean = sum / data.count;
            result->value = sqrt(sum_sq / data.count - mean * mean);
            break;
        }
        
        default:
            break;
    }
    
    result_destroy(&data);
    return ROCKSDB_OK;
}

rocksdb_error_t rocksdb_tsdb_delete_measurement(rocksdb_tsdb_t* tsdb, const char* measurement) {
    if (!tsdb || !measurement || !tsdb->is_open) return ROCKSDB_ERR_INVALID_PARAM;
    
    rocksdb_writebatch_t* batch = rocksdb_writebatch_create();
    
    rocksdb_iterator_t* iter = rocksdb_create_iterator(tsdb->db, tsdb->read_options);
    
    char prefix[256];
    snprintf(prefix, sizeof(prefix), "%s_", measurement);
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
    rocksdb_write(tsdb->db, tsdb->write_options, batch, &err);
    rocksdb_writebatch_destroy(batch);
    
    if (err) {
        free(err);
        return ROCKSDB_ERR_IO;
    }
    
    return ROCKSDB_OK;
}

rocksdb_error_t rocksdb_tsdb_delete_range(rocksdb_tsdb_t* tsdb, const char* measurement,
                                      const range_t* range) {
    if (!tsdb || !measurement || !range || !tsdb->is_open) return ROCKSDB_ERR_INVALID_PARAM;
    return ROCKSDB_OK;
}

rocksdb_error_t rocksdb_tsdb_flush(rocksdb_tsdb_t* tsdb) {
    if (!tsdb || !tsdb->is_open) return ROCKSDB_ERR_INVALID_PARAM;
    return ROCKSDB_OK;
}

rocksdb_error_t rocksdb_tsdb_compact(rocksdb_tsdb_t* tsdb) {
    if (!tsdb || !tsdb->is_open) return ROCKSDB_ERR_INVALID_PARAM;
    return ROCKSDB_OK;
}

uint64_t rocksdb_get_total_points(rocksdb_tsdb_t* tsdb) {
    return tsdb ? tsdb->total_points : 0;
}

uint64_t rocksdb_get_total_series(rocksdb_tsdb_t* tsdb) {
    return tsdb ? tsdb->total_series : 0;
}

size_t rocksdb_get_storage_size(rocksdb_tsdb_t* tsdb) {
    if (!tsdb) return 0;
    
    struct stat st;
    if (stat(tsdb->path, &st) == 0) {
        return (size_t)st.st_size;
    }
    return 0;
}

point_t* point_create(const char* measurement, ts_timestamp_t ts) {
    if (!measurement) return NULL;
    
    point_t* p = calloc(1, sizeof(point_t));
    if (!p) return NULL;
    
    strncpy(p->measurement, measurement, sizeof(p->measurement) - 1);
    p->timestamp = ts > 0 ? ts : ts_now();
    return p;
}

void point_destroy(point_t* point) {
    free(point);
}

point_t* point_add_tag(point_t* p, const char* key, const char* value) {
    if (!p || !key || !value) return p;
    if (p->tag_count >= 32) return p;
    
    strncpy(p->tags[p->tag_count].key, key, sizeof(p->tags[0].key) - 1);
    strncpy(p->tags[p->tag_count].value, value, sizeof(p->tags[0].value) - 1);
    p->tag_count++;
    return p;
}

point_t* point_add_field_f64(point_t* p, const char* name, double value) {
    if (!p || !name) return p;
    if (p->field_count >= 64) return p;
    
    strncpy(p->fields[p->field_count].name, name, sizeof(p->fields[0].name) - 1);
    p->fields[p->field_count].type = FIELD_FLOAT;
    p->fields[p->field_count].value.f64 = value;
    p->field_count++;
    return p;
}

point_t* point_add_field_i64(point_t* p, const char* name, int64_t value) {
    if (!p || !name) return p;
    if (p->field_count >= 64) return p;
    
    strncpy(p->fields[p->field_count].name, name, sizeof(p->fields[0].name) - 1);
    p->fields[p->field_count].type = FIELD_INT;
    p->fields[p->field_count].value.i64 = value;
    p->field_count++;
    return p;
}

point_t* point_add_field_bool(point_t* p, const char* name, bool value) {
    if (!p || !name) return p;
    if (p->field_count >= 64) return p;
    
    strncpy(p->fields[p->field_count].name, name, sizeof(p->fields[0].name) - 1);
    p->fields[p->field_count].type = FIELD_BOOL;
    p->fields[p->field_count].value.boolean = value;
    p->field_count++;
    return p;
}

point_t* point_add_field_str(point_t* p, const char* name, const char* value) {
    if (!p || !name || !value) return p;
    if (p->field_count >= 64) return p;
    
    strncpy(p->fields[p->field_count].name, name, sizeof(p->fields[0].name) - 1);
    p->fields[p->field_count].type = FIELD_STRING;
    strncpy(p->fields[p->field_count].value.str, value, sizeof(p->fields[0].value.str) - 1);
    p->field_count++;
    return p;
}

void result_destroy(result_t* result) {
    if (!result) return;
    free(result->points);
    result->points = NULL;
    result->count = 0;
    result->capacity = 0;
}

ts_timestamp_t ts_now(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (ts_timestamp_t)ts.tv_sec * 1000000000LL + ts.tv_nsec;
}
