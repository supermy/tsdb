#include "tsdb25_index.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

#define HASH_BUCKET_COUNT 2048

typedef struct hash_bucket {
    tsdb25_series_t* series;
    struct hash_bucket* next;
} hash_bucket_t;

struct tsdb25_index {
    hash_bucket_t** buckets;
    size_t           bucket_count;
    size_t           series_count;
    size_t           measurement_count;
    
    tsdb25_series_t** all_series;
    size_t           all_series_cap;
    
    char** measurements;
    size_t           measurements_cap;
};

static size_t hash_series_id(uint64_t id, size_t bucket_count) {
    return id % bucket_count;
}

tsdb25_index_t* tsdb25_index_create(void) {
    tsdb25_index_t* idx = (tsdb25_index_t*)calloc(1, sizeof(tsdb25_index_t));
    if (!idx) return NULL;
    
    idx->bucket_count = HASH_BUCKET_COUNT;
    idx->buckets = (hash_bucket_t**)calloc(idx->bucket_count, sizeof(hash_bucket_t*));
    if (!idx->buckets) {
        free(idx);
        return NULL;
    }
    
    idx->all_series_cap = 4096;
    idx->all_series = (tsdb25_series_t**)calloc(idx->all_series_cap, sizeof(tsdb25_series_t*));
    
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

void tsdb25_index_destroy(tsdb25_index_t* index) {
    if (!index) return;
    
    for (size_t i = 0; i < index->series_count; i++) {
        free(index->all_series[i]);
    }
    for (size_t i = 0; i < index->measurement_count; i++) {
        free(index->measurements[i]);
    }
    
    for (size_t i = 0; i < index->bucket_count; i++) {
        hash_bucket_t* bucket = index->buckets[i];
        while (bucket) {
            hash_bucket_t* next = bucket->next;
            free(bucket);
            bucket = next;
        }
    }
    
    free(index->buckets);
    free(index->all_series);
    free(index->measurements);
    free(index);
}

tsdb25_status_t tsdb25_index_add_series(tsdb25_index_t* index, const tsdb25_series_t* series) {
    if (!index || !series) return TSDB25_ERR_INVALID_PARAM;
    
    if (index->series_count >= index->all_series_cap) {
        size_t new_cap = index->all_series_cap * 2;
        tsdb25_series_t** new_series = (tsdb25_series_t**)realloc(
            index->all_series, new_cap * sizeof(tsdb25_series_t*));
        if (!new_series) return TSDB25_ERR_NO_MEMORY;
        index->all_series = new_series;
        index->all_series_cap = new_cap;
    }
    
    tsdb25_series_t* new_series = (tsdb25_series_t*)malloc(sizeof(tsdb25_series_t));
    if (!new_series) return TSDB25_ERR_NO_MEMORY;
    memcpy(new_series, series, sizeof(tsdb25_series_t));
    new_series->next = NULL;
    
    size_t bucket = hash_series_id(series->series_id, index->bucket_count);
    hash_bucket_t* new_bucket = (hash_bucket_t*)malloc(sizeof(hash_bucket_t));
    if (!new_bucket) {
        free(new_series);
        return TSDB25_ERR_NO_MEMORY;
    }
    new_bucket->series = new_series;
    new_bucket->next = index->buckets[bucket];
    index->buckets[bucket] = new_bucket;
    
    index->all_series[index->series_count++] = new_series;
    
    bool meas_exists = false;
    for (size_t i = 0; i < index->measurement_count; i++) {
        if (strcmp(index->measurements[i], series->measurement) == 0) {
            meas_exists = true;
            break;
        }
    }
    
    if (!meas_exists) {
        index->measurements[index->measurement_count++] = strdup(series->measurement);
    }
    
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_index_remove_series(tsdb25_index_t* index, uint64_t series_id) {
    if (!index) return TSDB25_ERR_INVALID_PARAM;
    
    size_t bucket = hash_series_id(series_id, index->bucket_count);
    hash_bucket_t* prev = NULL;
    hash_bucket_t* curr = index->buckets[bucket];
    
    while (curr) {
        if (curr->series->series_id == series_id) {
            if (prev) {
                prev->next = curr->next;
            } else {
                index->buckets[bucket] = curr->next;
            }
            
            for (size_t i = 0; i < index->series_count; i++) {
                if (index->all_series[i] && index->all_series[i]->series_id == series_id) {
                    free(index->all_series[i]);
                    index->all_series[i] = index->all_series[--index->series_count];
                    break;
                }
            }
            
            free(curr->series);
            free(curr);
            return TSDB25_OK;
        }
        prev = curr;
        curr = curr->next;
    }
    
    return TSDB25_ERR_NOT_FOUND;
}

tsdb25_series_t* tsdb25_index_get_series(tsdb25_index_t* index, uint64_t series_id) {
    if (!index) return NULL;
    
    size_t bucket = hash_series_id(series_id, index->bucket_count);
    hash_bucket_t* curr = index->buckets[bucket];
    
    while (curr) {
        if (curr->series->series_id == series_id) {
            return curr->series;
        }
        curr = curr->next;
    }
    return NULL;
}

static bool tags_match(const tsdb25_tag_t* t1, size_t c1, const tsdb25_tag_t* t2, size_t c2) {
    if (c1 != c2) return false;
    for (size_t i = 0; i < c1; i++) {
        bool found = false;
        for (size_t j = 0; j < c2; j++) {
            if (strcmp(t1[i].key, t2[j].key) == 0 && strcmp(t1[i].value, t2[j].value) == 0) {
                found = true;
                break;
            }
        }
        if (!found) return false;
    }
    return true;
}

tsdb25_status_t tsdb25_index_find_by_measurement(tsdb25_index_t* index, const char* measurement,
                                                  tsdb25_series_list_t* result) {
    if (!index || !measurement || !result) return TSDB25_ERR_INVALID_PARAM;
    
    size_t count = 0;
    for (size_t i = 0; i < index->series_count; i++) {
        if (index->all_series[i] && strcmp(index->all_series[i]->measurement, measurement) == 0) {
            count++;
        }
    }
    
    result->capacity = count > 0 ? count : 1;
    result->series_ids = (uint64_t*)calloc(result->capacity, sizeof(uint64_t));
    if (!result->series_ids) return TSDB25_ERR_NO_MEMORY;
    
    result->count = 0;
    for (size_t i = 0; i < index->series_count; i++) {
        if (index->all_series[i] && strcmp(index->all_series[i]->measurement, measurement) == 0) {
            result->series_ids[result->count++] = index->all_series[i]->series_id;
        }
    }
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_index_find_by_tags(tsdb25_index_t* index, const char* measurement,
                                           const tsdb25_tag_t* tags, size_t tags_count,
                                           tsdb25_series_list_t* result) {
    if (!index || !measurement || !result) return TSDB25_ERR_INVALID_PARAM;
    
    size_t count = 0;
    for (size_t i = 0; i < index->series_count; i++) {
        tsdb25_series_t* s = index->all_series[i];
        if (s && strcmp(s->measurement, measurement) == 0 && tags_match(s->tags, s->tags_count, tags, tags_count)) {
            count++;
        }
    }
    
    result->capacity = count > 0 ? count : 1;
    result->series_ids = (uint64_t*)calloc(result->capacity, sizeof(uint64_t));
    if (!result->series_ids) return TSDB25_ERR_NO_MEMORY;
    
    result->count = 0;
    for (size_t i = 0; i < index->series_count; i++) {
        tsdb25_series_t* s = index->all_series[i];
        if (s && strcmp(s->measurement, measurement) == 0 && tags_match(s->tags, s->tags_count, tags, tags_count)) {
            result->series_ids[result->count++] = s->series_id;
        }
    }
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_index_find_by_time_range(tsdb25_index_t* index, const char* measurement,
                                                  tsdb25_time_range_t* range, tsdb25_series_list_t* result) {
    if (!index || !measurement || !range || !result) return TSDB25_ERR_INVALID_PARAM;
    
    size_t count = 0;
    for (size_t i = 0; i < index->series_count; i++) {
        tsdb25_series_t* s = index->all_series[i];
        if (s && strcmp(s->measurement, measurement) == 0) {
            if (s->max_time >= range->start && s->min_time <= range->end) {
                count++;
            }
        }
    }
    
    result->capacity = count > 0 ? count : 1;
    result->series_ids = (uint64_t*)calloc(result->capacity, sizeof(uint64_t));
    if (!result->series_ids) return TSDB25_ERR_NO_MEMORY;
    
    result->count = 0;
    for (size_t i = 0; i < index->series_count; i++) {
        tsdb25_series_t* s = index->all_series[i];
        if (s && strcmp(s->measurement, measurement) == 0) {
            if (s->max_time >= range->start && s->min_time <= range->end) {
                result->series_ids[result->count++] = s->series_id;
            }
        }
    }
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_index_update_stats(tsdb25_index_t* index, uint64_t series_id,
                                          tsdb25_timestamp_t timestamp, uint64_t points_added) {
    if (!index) return TSDB25_ERR_INVALID_PARAM;
    
    tsdb25_series_t* s = tsdb25_index_get_series(index, series_id);
    if (!s) return TSDB25_ERR_NOT_FOUND;
    
    if (timestamp < s->min_time || s->min_time == 0) s->min_time = timestamp;
    if (timestamp > s->max_time) s->max_time = timestamp;
    s->point_count += points_added;
    
    return TSDB25_OK;
}

size_t tsdb25_index_series_count(tsdb25_index_t* index) {
    return index ? index->series_count : 0;
}

size_t tsdb25_index_measurement_count(tsdb25_index_t* index) {
    return index ? index->measurement_count : 0;
}

tsdb25_status_t tsdb25_index_persist(tsdb25_index_t* index, const char* path) {
    if (!index || !path) return TSDB25_ERR_INVALID_PARAM;
    
    FILE* fp = fopen(path, "wb");
    if (!fp) return TSDB25_ERR_IO_ERROR;
    
    fwrite(&index->series_count, sizeof(size_t), 1, fp);
    fwrite(&index->measurement_count, sizeof(size_t), 1, fp);
    
    for (size_t i = 0; i < index->series_count; i++) {
        fwrite(index->all_series[i], sizeof(tsdb25_series_t), 1, fp);
    }
    
    fclose(fp);
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_index_load(tsdb25_index_t* index, const char* path) {
    if (!index || !path) return TSDB25_ERR_INVALID_PARAM;
    
    FILE* fp = fopen(path, "rb");
    if (!fp) return TSDB25_ERR_IO_ERROR;
    
    size_t series_count, measurement_count;
    fread(&series_count, sizeof(size_t), 1, fp);
    fread(&measurement_count, sizeof(size_t), 1, fp);
    
    for (size_t i = 0; i < series_count; i++) {
        tsdb25_series_t series;
        fread(&series, sizeof(tsdb25_series_t), 1, fp);
        tsdb25_index_add_series(index, &series);
    }
    
    fclose(fp);
    return TSDB25_OK;
}

void tsdb25_series_list_destroy(tsdb25_series_list_t* list) {
    if (!list) return;
    free(list->series_ids);
    list->series_ids = NULL;
    list->count = 0;
    list->capacity = 0;
}
