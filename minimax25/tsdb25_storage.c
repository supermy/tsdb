#include "tsdb25_storage.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <time.h>

#define SERIES_BLOCK_CAPACITY 8192
#define MAX_CACHED_SERIES 1024

typedef struct {
    uint64_t         series_id;
    tsdb25_timestamp_t* timestamps;
    tsdb25_value_t*     values;
    size_t              count;
    size_t              capacity;
    char                file_path[TSDB25_MAX_PATH_LEN];
    int                 fd;
    bool                is_dirty;
    tsdb25_timestamp_t  min_time;
    tsdb25_timestamp_t  max_time;
} series_data_t;

struct tsdb25_storage {
    char              base_path[TSDB25_MAX_PATH_LEN];
    tsdb25_config_t   config;
    series_data_t**   series_cache;
    size_t            series_count;
    size_t            series_capacity;
    tsdb25_file_header_t header;
    int               meta_fd;
    uint64_t          total_points;
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

uint64_t tsdb25_hash_series(const tsdb25_point_t* point) {
    uint64_t hash = fnv1a_hash64(point->measurement, strlen(point->measurement));
    for (size_t i = 0; i < point->tags_count; i++) {
        hash ^= fnv1a_hash64(point->tags[i].key, strlen(point->tags[i].key));
        hash ^= fnv1a_hash64(point->tags[i].value, strlen(point->tags[i].value));
    }
    return hash;
}

uint64_t tsdb25_storage_get_series_id(const tsdb25_point_t* point) {
    return tsdb25_hash_series(point);
}

static series_data_t* series_data_new(uint64_t series_id) {
    series_data_t* sd = (series_data_t*)calloc(1, sizeof(series_data_t));
    if (!sd) return NULL;
    
    sd->series_id = series_id;
    sd->capacity = SERIES_BLOCK_CAPACITY;
    sd->timestamps = (tsdb25_timestamp_t*)malloc(sd->capacity * sizeof(tsdb25_timestamp_t));
    sd->values = (tsdb25_value_t*)malloc(sd->capacity * sizeof(tsdb25_value_t));
    sd->fd = -1;
    
    if (!sd->timestamps || !sd->values) {
        free(sd->timestamps);
        free(sd->values);
        free(sd);
        return NULL;
    }
    return sd;
}

static void series_data_free(series_data_t* sd) {
    if (!sd) return;
    if (sd->fd >= 0) close(sd->fd);
    free(sd->timestamps);
    free(sd->values);
    free(sd);
}

tsdb25_storage_t* tsdb25_storage_create(const tsdb25_config_t* config) {
    tsdb25_storage_t* s = (tsdb25_storage_t*)calloc(1, sizeof(tsdb25_storage_t));
    if (!s) return NULL;
    
    s->config = *config;
    s->series_capacity = 256;
    s->series_cache = (series_data_t**)calloc(s->series_capacity, sizeof(series_data_t*));
    s->meta_fd = -1;
    
    if (!s->series_cache) {
        free(s);
        return NULL;
    }
    return s;
}

void tsdb25_storage_destroy(tsdb25_storage_t* storage) {
    if (!storage) return;
    tsdb25_storage_close(storage);
    for (size_t i = 0; i < storage->series_count; i++) {
        series_data_free(storage->series_cache[i]);
    }
    free(storage->series_cache);
    free(storage);
}

tsdb25_status_t tsdb25_storage_open(tsdb25_storage_t* storage, const char* path) {
    if (!storage || !path) return TSDB25_ERR_INVALID_PARAM;
    
    strncpy(storage->base_path, path, TSDB25_MAX_PATH_LEN - 1);
    
    struct stat st = {0};
    if (stat(path, &st) == -1) {
        if (mkdir(path, 0755) != 0) return TSDB25_ERR_IO_ERROR;
    }
    
    char meta_path[TSDB25_MAX_PATH_LEN];
    snprintf(meta_path, sizeof(meta_path), "%s/meta.db", path);
    
    storage->meta_fd = open(meta_path, O_RDWR | O_CREAT, 0644);
    if (storage->meta_fd < 0) return TSDB25_ERR_IO_ERROR;
    
    if (read(storage->meta_fd, &storage->header, sizeof(storage->header)) == sizeof(storage->header)) {
        if (storage->header.magic != TSDB25_FILE_MAGIC) {
            close(storage->meta_fd);
            storage->meta_fd = -1;
            return TSDB25_ERR_CORRUPTED;
        }
    } else {
        memset(&storage->header, 0, sizeof(storage->header));
        storage->header.magic = TSDB25_FILE_MAGIC;
        storage->header.version = TSDB25_FILE_VERSION;
        storage->header.create_time = (uint64_t)time(NULL);
        
        if (write(storage->meta_fd, &storage->header, sizeof(storage->header)) != sizeof(storage->header)) {
            close(storage->meta_fd);
            storage->meta_fd = -1;
            return TSDB25_ERR_IO_ERROR;
        }
    }
    
    char data_path[TSDB25_MAX_PATH_LEN];
    snprintf(data_path, sizeof(data_path), "%s/data", path);
    if (stat(data_path, &st) == -1) {
        mkdir(data_path, 0755);
    }
    
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_storage_close(tsdb25_storage_t* storage) {
    if (!storage) return TSDB25_ERR_INVALID_PARAM;
    
    tsdb25_storage_flush(storage);
    
    if (storage->meta_fd >= 0) {
        storage->header.total_points = storage->total_points;
        write(storage->meta_fd, &storage->header, sizeof(storage->header));
        close(storage->meta_fd);
        storage->meta_fd = -1;
    }
    return TSDB25_OK;
}

static series_data_t* find_or_create_series(tsdb25_storage_t* storage, uint64_t series_id) {
    for (size_t i = 0; i < storage->series_count; i++) {
        if (storage->series_cache[i]->series_id == series_id) {
            return storage->series_cache[i];
        }
    }
    
    if (storage->series_count >= storage->series_capacity) {
        size_t new_cap = storage->series_capacity * 2;
        series_data_t** new_cache = (series_data_t**)realloc(storage->series_cache, new_cap * sizeof(series_data_t*));
        if (!new_cache) return NULL;
        storage->series_cache = new_cache;
        storage->series_capacity = new_cap;
    }
    
    series_data_t* sd = series_data_new(series_id);
    if (!sd) return NULL;
    
    snprintf(sd->file_path, sizeof(sd->file_path), "%s/data/%lu.tsdb", 
             storage->base_path, (unsigned long)series_id);
    
    storage->series_cache[storage->series_count++] = sd;
    return sd;
}

tsdb25_status_t tsdb25_storage_write_point(tsdb25_storage_t* storage, const tsdb25_point_t* point) {
    if (!storage || !point) return TSDB25_ERR_INVALID_PARAM;
    
    uint64_t series_id = tsdb25_storage_get_series_id(point);
    series_data_t* sd = find_or_create_series(storage, series_id);
    if (!sd) return TSDB25_ERR_NO_MEMORY;
    
    if (sd->count >= sd->capacity) {
        size_t new_cap = sd->capacity * 2;
        tsdb25_timestamp_t* new_ts = (tsdb25_timestamp_t*)realloc(sd->timestamps, new_cap * sizeof(tsdb25_timestamp_t));
        tsdb25_value_t* new_vals = (tsdb25_value_t*)realloc(sd->values, new_cap * sizeof(tsdb25_value_t));
        if (!new_ts || !new_vals) {
            free(new_ts);
            free(new_vals);
            return TSDB25_ERR_NO_MEMORY;
        }
        sd->timestamps = new_ts;
        sd->values = new_vals;
        sd->capacity = new_cap;
    }
    
    sd->timestamps[sd->count] = point->timestamp;
    
    if (point->fields_count > 0) {
        if (point->fields[0].type == TSDB25_FIELD_FLOAT) {
            sd->values[sd->count] = point->fields[0].value.float_val;
        } else if (point->fields[0].type == TSDB25_FIELD_INTEGER) {
            sd->values[sd->count] = (tsdb25_value_t)point->fields[0].value.int_val;
        } else {
            sd->values[sd->count] = 0.0;
        }
    } else {
        sd->values[sd->count] = 0.0;
    }
    
    if (sd->count == 0 || point->timestamp < sd->min_time) sd->min_time = point->timestamp;
    if (sd->count == 0 || point->timestamp > sd->max_time) sd->max_time = point->timestamp;
    
    sd->count++;
    sd->is_dirty = true;
    storage->total_points++;
    
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_storage_write_batch(tsdb25_storage_t* storage, const tsdb25_point_t* points, size_t count) {
    if (!storage || !points) return TSDB25_ERR_INVALID_PARAM;
    for (size_t i = 0; i < count; i++) {
        tsdb25_status_t status = tsdb25_storage_write_point(storage, &points[i]);
        if (status != TSDB25_OK) return status;
    }
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_storage_read_series(tsdb25_storage_t* storage, uint64_t series_id,
                                           tsdb25_time_range_t* range, tsdb25_result_set_t* result) {
    if (!storage || !result) return TSDB25_ERR_INVALID_PARAM;
    
    series_data_t* sd = NULL;
    for (size_t i = 0; i < storage->series_count; i++) {
        if (storage->series_cache[i]->series_id == series_id) {
            sd = storage->series_cache[i];
            break;
        }
    }
    
    if (!sd) return TSDB25_ERR_NOT_FOUND;
    
    size_t count = 0;
    for (size_t i = 0; i < sd->count; i++) {
        if (!range || (sd->timestamps[i] >= range->start && sd->timestamps[i] <= range->end)) {
            count++;
        }
    }
    
    if (count == 0) {
        result->points = NULL;
        result->count = 0;
        result->capacity = 0;
        return TSDB25_OK;
    }
    
    result->points = (tsdb25_point_t*)calloc(count, sizeof(tsdb25_point_t));
    if (!result->points) return TSDB25_ERR_NO_MEMORY;
    
    result->capacity = count;
    result->count = 0;
    
    for (size_t i = 0; i < sd->count && result->count < count; i++) {
        if (!range || (sd->timestamps[i] >= range->start && sd->timestamps[i] <= range->end)) {
            tsdb25_point_t* p = &result->points[result->count];
            p->timestamp = sd->timestamps[i];
            p->fields_count = 1;
            p->fields[0].type = TSDB25_FIELD_FLOAT;
            p->fields[0].value.float_val = sd->values[i];
            result->count++;
        }
    }
    
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_storage_flush(tsdb25_storage_t* storage) {
    if (!storage) return TSDB25_ERR_INVALID_PARAM;
    
    char data_path[TSDB25_MAX_PATH_LEN];
    snprintf(data_path, sizeof(data_path), "%s/data", storage->base_path);
    struct stat st = {0};
    if (stat(data_path, &st) == -1) {
        mkdir(data_path, 0755);
    }
    
    for (size_t i = 0; i < storage->series_count; i++) {
        series_data_t* sd = storage->series_cache[i];
        if (!sd->is_dirty || sd->count == 0) continue;
        
        FILE* fp = fopen(sd->file_path, "ab");
        if (!fp) continue;
        
        fwrite(sd->timestamps, sizeof(tsdb25_timestamp_t), sd->count, fp);
        fwrite(sd->values, sizeof(tsdb25_value_t), sd->count, fp);
        fclose(fp);
        
        sd->is_dirty = false;
    }
    
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_storage_compact(tsdb25_storage_t* storage) {
    if (!storage) return TSDB25_ERR_INVALID_PARAM;
    return TSDB25_OK;
}

size_t tsdb25_storage_total_points(tsdb25_storage_t* storage) {
    return storage ? storage->total_points : 0;
}

size_t tsdb25_storage_total_series(tsdb25_storage_t* storage) {
    return storage ? storage->series_count : 0;
}
