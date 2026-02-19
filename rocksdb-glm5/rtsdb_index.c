#include "rtsdb_index.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

#define HASH_BUCKET_COUNT 2048

typedef struct index_bucket {
    rtsdb_series_info_t* series;
    struct index_bucket* next;
} index_bucket_t;

struct rtsdb_index {
    index_bucket_t** buckets;
    size_t           bucket_count;
    size_t           series_count;
    size_t           measurement_count;
    
    rtsdb_series_info_t** all_series;
    size_t           all_series_cap;
    
    char** measurements;
    size_t           measurements_cap;
};

static size_t hash_series_id(uint64_t id, size_t bucket_count) {
    return id % bucket_count;
}

rtsdb_index_t* rtsdb_index_create(void) {
    rtsdb_index_t* idx = (rtsdb_index_t*)calloc(1, sizeof(rtsdb_index_t));
    if (!idx) return NULL;
    
    idx->bucket_count = HASH_BUCKET_COUNT;
    idx->buckets = (index_bucket_t**)calloc(idx->bucket_count, sizeof(index_bucket_t*));
    if (!idx->buckets) {
        free(idx);
        return NULL;
    }
    
    idx->all_series_cap = 4096;
    idx->all_series = (rtsdb_series_info_t**)calloc(idx->all_series_cap, sizeof(rtsdb_series_info_t*));
    
    idx->measurements_cap = 256;
    idx->measurements = (char**)calloc(idx->measurements_cap, sizeof(char*));
    
    if (!idx->all_series || !idx->measurements) {
        free(idx->buckets);
        free(idx->all_series);
        free(idx->measurements);
        free(idx);
        return NULL;
    }
    
    return idx;
}

void rtsdb_index_destroy(rtsdb_index_t* index) {
    if (!index) return;
    
    for (size_t i = 0; i < index->series_count; i++) {
        free(index->all_series[i]);
    }
    for (size_t i = 0; i < index->measurement_count; i++) {
        free(index->measurements[i]);
    }
    
    for (size_t i = 0; i < index->bucket_count; i++) {
        index_bucket_t* bucket = index->buckets[i];
        while (bucket) {
            index_bucket_t* next = bucket->next;
            free(bucket);
            bucket = next;
        }
    }
    
    free(index->buckets);
    free(index->all_series);
    free(index->measurements);
    free(index);
}

rtsdb_status_t rtsdb_index_add_series(rtsdb_index_t* index, const rtsdb_series_info_t* info) {
    if (!index || !info) return RTSDB_ERR_INVALID_PARAM;
    
    if (index->series_count >= index->all_series_cap) {
        size_t new_cap = index->all_series_cap * 2;
        rtsdb_series_info_t** new_series = (rtsdb_series_info_t**)realloc(
            index->all_series, new_cap * sizeof(rtsdb_series_info_t*));
        if (!new_series) return RTSDB_ERR_NO_MEMORY;
        index->all_series = new_series;
        index->all_series_cap = new_cap;
    }
    
    rtsdb_series_info_t* new_info = (rtsdb_series_info_t*)malloc(sizeof(rtsdb_series_info_t));
    if (!new_info) return RTSDB_ERR_NO_MEMORY;
    memcpy(new_info, info, sizeof(rtsdb_series_info_t));
    
    size_t bucket = hash_series_id(info->series_id, index->bucket_count);
    index_bucket_t* new_bucket = (index_bucket_t*)malloc(sizeof(index_bucket_t));
    if (!new_bucket) {
        free(new_info);
        return RTSDB_ERR_NO_MEMORY;
    }
    new_bucket->series = new_info;
    new_bucket->next = index->buckets[bucket];
    index->buckets[bucket] = new_bucket;
    
    index->all_series[index->series_count++] = new_info;
    
    bool meas_exists = false;
    for (size_t i = 0; i < index->measurement_count; i++) {
        if (strcmp(index->measurements[i], info->measurement) == 0) {
            meas_exists = true;
            break;
        }
    }
    
    if (!meas_exists) {
        if (index->measurement_count >= index->measurements_cap) {
            size_t new_cap = index->measurements_cap * 2;
            char** new_measurements = (char**)realloc(index->measurements, new_cap * sizeof(char*));
            if (!new_measurements) return RTSDB_ERR_NO_MEMORY;
            index->measurements = new_measurements;
            index->measurements_cap = new_cap;
        }
        index->measurements[index->measurement_count++] = strdup(info->measurement);
    }
    
    return RTSDB_OK;
}

rtsdb_status_t rtsdb_index_remove_series(rtsdb_index_t* index, uint64_t series_id) {
    if (!index) return RTSDB_ERR_INVALID_PARAM;
    
    size_t bucket = hash_series_id(series_id, index->bucket_count);
    index_bucket_t* prev = NULL;
    index_bucket_t* curr = index->buckets[bucket];
    
    while (curr) {
        if (curr->series->series_id == series_id) {
            if (prev) prev->next = curr->next;
            else index->buckets[bucket] = curr->next;
            
            for (size_t i = 0; i < index->series_count; i++) {
                if (index->all_series[i] && index->all_series[i]->series_id == series_id) {
                    free(index->all_series[i]);
                    index->all_series[i] = index->all_series[--index->series_count];
                    break;
                }
            }
            
            free(curr->series);
            free(curr);
            return RTSDB_OK;
        }
        prev = curr;
        curr = curr->next;
    }
    
    return RTSDB_ERR_NOT_FOUND;
}

rtsdb_series_info_t* rtsdb_index_get_series(rtsdb_index_t* index, uint64_t series_id) {
    if (!index) return NULL;
    
    size_t bucket = hash_series_id(series_id, index->bucket_count);
    index_bucket_t* curr = index->buckets[bucket];
    
    while (curr) {
        if (curr->series->series_id == series_id) return curr->series;
        curr = curr->next;
    }
    return NULL;
}

rtsdb_status_t rtsdb_index_find_by_measurement(rtsdb_index_t* index, const char* measurement,
                                                rtsdb_series_list_t* result) {
    if (!index || !measurement || !result) return RTSDB_ERR_INVALID_PARAM;
    
    size_t count = 0;
    for (size_t i = 0; i < index->series_count; i++) {
        if (index->all_series[i] && strcmp(index->all_series[i]->measurement, measurement) == 0) {
            count++;
        }
    }
    
    result->capacity = count > 0 ? count : 1;
    result->series_ids = (uint64_t*)calloc(result->capacity, sizeof(uint64_t));
    if (!result->series_ids) return RTSDB_ERR_NO_MEMORY;
    
    result->count = 0;
    for (size_t i = 0; i < index->series_count; i++) {
        if (index->all_series[i] && strcmp(index->all_series[i]->measurement, measurement) == 0) {
            result->series_ids[result->count++] = index->all_series[i]->series_id;
        }
    }
    return RTSDB_OK;
}

rtsdb_status_t rtsdb_index_update_stats(rtsdb_index_t* index, uint64_t series_id,
                                         rtsdb_timestamp_t timestamp, uint64_t points_added) {
    if (!index) return RTSDB_ERR_INVALID_PARAM;
    
    rtsdb_series_info_t* s = rtsdb_index_get_series(index, series_id);
    if (!s) return RTSDB_ERR_NOT_FOUND;
    
    if (timestamp < s->min_time || s->min_time == 0) s->min_time = timestamp;
    if (timestamp > s->max_time) s->max_time = timestamp;
    s->point_count += points_added;
    
    return RTSDB_OK;
}

size_t rtsdb_index_series_count(rtsdb_index_t* index) {
    return index ? index->series_count : 0;
}

size_t rtsdb_index_measurement_count(rtsdb_index_t* index) {
    return index ? index->measurement_count : 0;
}

rtsdb_status_t rtsdb_index_persist(rtsdb_index_t* index, const char* path) {
    if (!index || !path) return RTSDB_ERR_INVALID_PARAM;
    
    FILE* fp = fopen(path, "wb");
    if (!fp) return RTSDB_ERR_IO;
    
    fwrite(&index->series_count, sizeof(size_t), 1, fp);
    fwrite(&index->measurement_count, sizeof(size_t), 1, fp);
    
    for (size_t i = 0; i < index->series_count; i++) {
        fwrite(index->all_series[i], sizeof(rtsdb_series_info_t), 1, fp);
    }
    
    fclose(fp);
    return RTSDB_OK;
}

rtsdb_status_t rtsdb_index_load(rtsdb_index_t* index, const char* path) {
    if (!index || !path) return RTSDB_ERR_INVALID_PARAM;
    
    FILE* fp = fopen(path, "rb");
    if (!fp) return RTSDB_ERR_IO;
    
    size_t series_count, measurement_count;
    fread(&series_count, sizeof(size_t), 1, fp);
    fread(&measurement_count, sizeof(size_t), 1, fp);
    
    for (size_t i = 0; i < series_count; i++) {
        rtsdb_series_info_t info;
        fread(&info, sizeof(rtsdb_series_info_t), 1, fp);
        rtsdb_index_add_series(index, &info);
    }
    
    fclose(fp);
    return RTSDB_OK;
}

void rtsdb_series_list_destroy(rtsdb_series_list_t* list) {
    if (!list) return;
    free(list->series_ids);
    list->series_ids = NULL;
    list->count = 0;
    list->capacity = 0;
}
