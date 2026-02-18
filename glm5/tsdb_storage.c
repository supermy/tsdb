#include "tsdb_storage.h"
#include "tsdb_compress.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <time.h>

#define SERIES_BLOCK_SIZE (64 * 1024)
#define MAX_SERIES_PER_FILE 10000

typedef struct {
    uint64_t series_id;
    tsdb_timestamp_t* timestamps;
    tsdb_value_t* values;
    size_t count;
    size_t capacity;
    char file_path[TSDB_MAX_PATH_LEN];
    int fd;
    uint64_t data_offset;
} tsdb_series_data_t;

struct tsdb_storage {
    char base_path[TSDB_MAX_PATH_LEN];
    tsdb_config_t config;
    tsdb_series_data_t** series;
    size_t series_count;
    size_t series_capacity;
    tsdb_file_header_t header;
    int current_fd;
    uint64_t current_offset;
    tsdb_compress_config_t compress_config;
};

static uint64_t fnv1a_hash(const void* data, size_t len) {
    const uint8_t* bytes = (const uint8_t*)data;
    uint64_t hash = 14695981039346656037ULL;
    
    for (size_t i = 0; i < len; i++) {
        hash ^= bytes[i];
        hash *= 1099511628211ULL;
    }
    
    return hash;
}

uint64_t tsdb_hash_tags(const tsdb_tag_t* tags, size_t count) {
    uint64_t hash = 0;
    
    for (size_t i = 0; i < count; i++) {
        hash ^= fnv1a_hash(tags[i].key, strlen(tags[i].key));
        hash ^= fnv1a_hash(tags[i].value, strlen(tags[i].value));
    }
    
    return hash;
}

uint64_t tsdb_storage_get_series_id(const tsdb_point_t* point) {
    uint64_t hash = fnv1a_hash(point->measurement, strlen(point->measurement));
    hash ^= tsdb_hash_tags(point->tags, point->tags_count);
    return hash;
}

static tsdb_series_data_t* series_data_create(uint64_t series_id) {
    tsdb_series_data_t* sd = (tsdb_series_data_t*)calloc(1, sizeof(tsdb_series_data_t));
    if (!sd) return NULL;
    
    sd->series_id = series_id;
    sd->capacity = SERIES_BLOCK_SIZE / (sizeof(tsdb_timestamp_t) + sizeof(tsdb_value_t));
    sd->timestamps = (tsdb_timestamp_t*)malloc(sd->capacity * sizeof(tsdb_timestamp_t));
    sd->values = (tsdb_value_t*)malloc(sd->capacity * sizeof(tsdb_value_t));
    sd->fd = -1;
    
    if (!sd->timestamps || !sd->values) {
        free(sd->timestamps);
        free(sd->values);
        free(sd);
        return NULL;
    }
    
    return sd;
}

static void series_data_destroy(tsdb_series_data_t* sd) {
    if (!sd) return;
    
    if (sd->fd >= 0) {
        close(sd->fd);
    }
    
    free(sd->timestamps);
    free(sd->values);
    free(sd);
}

tsdb_storage_t* tsdb_storage_create(const tsdb_config_t* config) {
    tsdb_storage_t* storage = (tsdb_storage_t*)calloc(1, sizeof(tsdb_storage_t));
    if (!storage) return NULL;
    
    storage->config = *config;
    storage->series_capacity = 1024;
    storage->series = (tsdb_series_data_t**)calloc(storage->series_capacity, sizeof(tsdb_series_data_t*));
    
    if (!storage->series) {
        free(storage);
        return NULL;
    }
    
    storage->compress_config = tsdb_compress_default_config();
    storage->compress_config.enabled = config->enable_compression;
    storage->compress_config.level = config->compression_level;
    storage->current_fd = -1;
    
    return storage;
}

void tsdb_storage_destroy(tsdb_storage_t* storage) {
    if (!storage) return;
    
    tsdb_storage_close(storage);
    
    for (size_t i = 0; i < storage->series_count; i++) {
        series_data_destroy(storage->series[i]);
    }
    
    free(storage->series);
    free(storage);
}

tsdb_status_t tsdb_storage_open(tsdb_storage_t* storage, const char* path) {
    if (!storage || !path) return TSDB_ERR_INVALID_PARAM;
    
    strncpy(storage->base_path, path, TSDB_MAX_PATH_LEN - 1);
    
    struct stat st = {0};
    if (stat(path, &st) == -1) {
        if (mkdir(path, 0755) != 0) {
            return TSDB_ERR_IO_ERROR;
        }
    }
    
    char meta_path[TSDB_MAX_PATH_LEN];
    snprintf(meta_path, sizeof(meta_path), "%s/meta.db", path);
    
    int fd = open(meta_path, O_RDWR | O_CREAT, 0644);
    if (fd < 0) {
        return TSDB_ERR_IO_ERROR;
    }
    
    if (read(fd, &storage->header, sizeof(storage->header)) == sizeof(storage->header)) {
        if (storage->header.magic != TSDB_FILE_MAGIC) {
            close(fd);
            return TSDB_ERR_CORRUPTED;
        }
    } else {
        memset(&storage->header, 0, sizeof(storage->header));
        storage->header.magic = TSDB_FILE_MAGIC;
        storage->header.version = TSDB_FILE_VERSION;
        storage->header.create_time = (uint64_t)time(NULL);
        
        if (write(fd, &storage->header, sizeof(storage->header)) != sizeof(storage->header)) {
            close(fd);
            return TSDB_ERR_IO_ERROR;
        }
    }
    
    close(fd);
    
    return TSDB_OK;
}

tsdb_status_t tsdb_storage_close(tsdb_storage_t* storage) {
    if (!storage) return TSDB_ERR_INVALID_PARAM;
    
    tsdb_storage_flush(storage);
    
    if (storage->current_fd >= 0) {
        close(storage->current_fd);
        storage->current_fd = -1;
    }
    
    char meta_path[TSDB_MAX_PATH_LEN];
    snprintf(meta_path, sizeof(meta_path), "%s/meta.db", storage->base_path);
    
    int fd = open(meta_path, O_WRONLY);
    if (fd >= 0) {
        write(fd, &storage->header, sizeof(storage->header));
        close(fd);
    }
    
    return TSDB_OK;
}

static tsdb_series_data_t* find_or_create_series(tsdb_storage_t* storage, uint64_t series_id) {
    for (size_t i = 0; i < storage->series_count; i++) {
        if (storage->series[i]->series_id == series_id) {
            return storage->series[i];
        }
    }
    
    if (storage->series_count >= storage->series_capacity) {
        size_t new_capacity = storage->series_capacity * 2;
        tsdb_series_data_t** new_series = (tsdb_series_data_t**)realloc(
            storage->series, new_capacity * sizeof(tsdb_series_data_t*));
        if (!new_series) return NULL;
        
        storage->series = new_series;
        storage->series_capacity = new_capacity;
    }
    
    tsdb_series_data_t* sd = series_data_create(series_id);
    if (!sd) return NULL;
    
    storage->series[storage->series_count++] = sd;
    return sd;
}

tsdb_status_t tsdb_storage_write_point(tsdb_storage_t* storage, const tsdb_point_t* point) {
    if (!storage || !point) return TSDB_ERR_INVALID_PARAM;
    
    uint64_t series_id = tsdb_storage_get_series_id(point);
    tsdb_series_data_t* sd = find_or_create_series(storage, series_id);
    if (!sd) return TSDB_ERR_NO_MEMORY;
    
    if (sd->count >= sd->capacity) {
        size_t new_capacity = sd->capacity * 2;
        tsdb_timestamp_t* new_ts = (tsdb_timestamp_t*)realloc(
            sd->timestamps, new_capacity * sizeof(tsdb_timestamp_t));
        tsdb_value_t* new_vals = (tsdb_value_t*)realloc(
            sd->values, new_capacity * sizeof(tsdb_value_t));
        
        if (!new_ts || !new_vals) {
            free(new_ts);
            free(new_vals);
            return TSDB_ERR_NO_MEMORY;
        }
        
        sd->timestamps = new_ts;
        sd->values = new_vals;
        sd->capacity = new_capacity;
    }
    
    sd->timestamps[sd->count] = point->timestamp;
    
    if (point->fields_count > 0 && point->fields[0].type == TSDB_FIELD_FLOAT) {
        sd->values[sd->count] = point->fields[0].value.float_val;
    } else {
        sd->values[sd->count] = 0.0;
    }
    
    sd->count++;
    storage->header.total_points++;
    
    return TSDB_OK;
}

tsdb_status_t tsdb_storage_write_batch(tsdb_storage_t* storage, const tsdb_point_t* points, size_t count) {
    if (!storage || !points) return TSDB_ERR_INVALID_PARAM;
    
    for (size_t i = 0; i < count; i++) {
        tsdb_status_t status = tsdb_storage_write_point(storage, &points[i]);
        if (status != TSDB_OK) return status;
    }
    
    return TSDB_OK;
}

tsdb_status_t tsdb_storage_read_series(
    tsdb_storage_t* storage,
    uint64_t series_id,
    tsdb_time_range_t* range,
    tsdb_result_set_t* result
) {
    if (!storage || !result) return TSDB_ERR_INVALID_PARAM;
    
    tsdb_series_data_t* sd = NULL;
    for (size_t i = 0; i < storage->series_count; i++) {
        if (storage->series[i]->series_id == series_id) {
            sd = storage->series[i];
            break;
        }
    }
    
    if (!sd) return TSDB_ERR_NOT_FOUND;
    
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
        return TSDB_OK;
    }
    
    result->points = (tsdb_point_t*)calloc(count, sizeof(tsdb_point_t));
    if (!result->points) return TSDB_ERR_NO_MEMORY;
    
    result->capacity = count;
    result->count = 0;
    
    for (size_t i = 0; i < sd->count && result->count < count; i++) {
        if (!range || (sd->timestamps[i] >= range->start && sd->timestamps[i] <= range->end)) {
            tsdb_point_t* p = &result->points[result->count];
            p->timestamp = sd->timestamps[i];
            p->fields_count = 1;
            p->fields[0].type = TSDB_FIELD_FLOAT;
            p->fields[0].value.float_val = sd->values[i];
            result->count++;
        }
    }
    
    return TSDB_OK;
}

tsdb_status_t tsdb_storage_flush(tsdb_storage_t* storage) {
    if (!storage) return TSDB_ERR_INVALID_PARAM;
    
    char data_path[TSDB_MAX_PATH_LEN];
    snprintf(data_path, sizeof(data_path), "%s/data", storage->base_path);
    
    struct stat st = {0};
    if (stat(data_path, &st) == -1) {
        mkdir(data_path, 0755);
    }
    
    for (size_t i = 0; i < storage->series_count; i++) {
        tsdb_series_data_t* sd = storage->series[i];
        
        if (sd->count == 0) continue;
        
        char file_path[TSDB_MAX_PATH_LEN];
        snprintf(file_path, sizeof(file_path), "%s/series_%lu.db", data_path, (unsigned long)sd->series_id);
        
        int fd = open(file_path, O_WRONLY | O_CREAT | O_APPEND, 0644);
        if (fd < 0) continue;
        
        write(fd, sd->timestamps, sd->count * sizeof(tsdb_timestamp_t));
        write(fd, sd->values, sd->count * sizeof(tsdb_value_t));
        
        close(fd);
    }
    
    return TSDB_OK;
}

tsdb_status_t tsdb_storage_compact(tsdb_storage_t* storage) {
    if (!storage) return TSDB_ERR_INVALID_PARAM;
    
    return TSDB_OK;
}
