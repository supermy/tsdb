#include "tsdb_index.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

typedef struct index_node {
    struct index_node* children[TSDB_INDEX_ORDER];
    struct index_node* parent;
    tsdb_series_info_t** series;
    size_t series_count;
    size_t series_capacity;
    bool is_leaf;
} index_node_t;

struct tsdb_index {
    index_node_t* root;
    size_t series_count;
    size_t measurement_count;
    
    tsdb_series_info_t** all_series;
    size_t all_series_capacity;
    
    char** measurements;
    size_t measurements_capacity;
};

static index_node_t* index_node_create(bool is_leaf) {
    index_node_t* node = (index_node_t*)calloc(1, sizeof(index_node_t));
    if (!node) return NULL;
    
    node->is_leaf = is_leaf;
    node->series_capacity = 16;
    node->series = (tsdb_series_info_t**)calloc(node->series_capacity, sizeof(tsdb_series_info_t*));
    
    if (!node->series) {
        free(node);
        return NULL;
    }
    
    return node;
}

static void index_node_destroy(index_node_t* node) {
    if (!node) return;
    
    for (int i = 0; i < TSDB_INDEX_ORDER; i++) {
        if (node->children[i]) {
            index_node_destroy(node->children[i]);
        }
    }
    
    free(node->series);
    free(node);
}

tsdb_index_t* tsdb_index_create(void) {
    tsdb_index_t* index = (tsdb_index_t*)calloc(1, sizeof(tsdb_index_t));
    if (!index) return NULL;
    
    index->root = index_node_create(true);
    if (!index->root) {
        free(index);
        return NULL;
    }
    
    index->all_series_capacity = 1024;
    index->all_series = (tsdb_series_info_t**)calloc(index->all_series_capacity, sizeof(tsdb_series_info_t*));
    
    index->measurements_capacity = 64;
    index->measurements = (char**)calloc(index->measurements_capacity, sizeof(char*));
    
    if (!index->all_series || !index->measurements) {
        free(index->all_series);
        free(index->measurements);
        index_node_destroy(index->root);
        free(index);
        return NULL;
    }
    
    return index;
}

void tsdb_index_destroy(tsdb_index_t* index) {
    if (!index) return;
    
    index_node_destroy(index->root);
    
    for (size_t i = 0; i < index->series_count; i++) {
        if (index->all_series[i]) {
            free(index->all_series[i]);
        }
    }
    
    for (size_t i = 0; i < index->measurement_count; i++) {
        free(index->measurements[i]);
    }
    
    free(index->all_series);
    free(index->measurements);
    free(index);
}

static size_t get_series_slot(uint64_t series_id) {
    return series_id % TSDB_INDEX_ORDER;
}

tsdb_status_t tsdb_index_add_series(tsdb_index_t* index, const tsdb_series_info_t* info) {
    if (!index || !info) return TSDB_ERR_INVALID_PARAM;
    
    if (index->series_count >= index->all_series_capacity) {
        size_t new_capacity = index->all_series_capacity * 2;
        tsdb_series_info_t** new_series = (tsdb_series_info_t**)realloc(
            index->all_series, new_capacity * sizeof(tsdb_series_info_t*));
        if (!new_series) return TSDB_ERR_NO_MEMORY;
        
        index->all_series = new_series;
        index->all_series_capacity = new_capacity;
    }
    
    tsdb_series_info_t* new_info = (tsdb_series_info_t*)malloc(sizeof(tsdb_series_info_t));
    if (!new_info) return TSDB_ERR_NO_MEMORY;
    
    memcpy(new_info, info, sizeof(tsdb_series_info_t));
    index->all_series[index->series_count++] = new_info;
    
    bool measurement_exists = false;
    for (size_t i = 0; i < index->measurement_count; i++) {
        if (strcmp(index->measurements[i], info->measurement) == 0) {
            measurement_exists = true;
            break;
        }
    }
    
    if (!measurement_exists) {
        if (index->measurement_count >= index->measurements_capacity) {
            size_t new_capacity = index->measurements_capacity * 2;
            char** new_measurements = (char**)realloc(
                index->measurements, new_capacity * sizeof(char*));
            if (!new_measurements) {
                free(new_info);
                return TSDB_ERR_NO_MEMORY;
            }
            index->measurements = new_measurements;
            index->measurements_capacity = new_capacity;
        }
        
        index->measurements[index->measurement_count] = strdup(info->measurement);
        index->measurement_count++;
    }
    
    return TSDB_OK;
}

tsdb_status_t tsdb_index_remove_series(tsdb_index_t* index, uint64_t series_id) {
    if (!index) return TSDB_ERR_INVALID_PARAM;
    
    for (size_t i = 0; i < index->series_count; i++) {
        if (index->all_series[i] && index->all_series[i]->series_id == series_id) {
            free(index->all_series[i]);
            index->all_series[i] = index->all_series[--index->series_count];
            index->all_series[index->series_count] = NULL;
            return TSDB_OK;
        }
    }
    
    return TSDB_ERR_NOT_FOUND;
}

tsdb_series_info_t* tsdb_index_get_series(tsdb_index_t* index, uint64_t series_id) {
    if (!index) return NULL;
    
    for (size_t i = 0; i < index->series_count; i++) {
        if (index->all_series[i] && index->all_series[i]->series_id == series_id) {
            return index->all_series[i];
        }
    }
    
    return NULL;
}

tsdb_status_t tsdb_index_find_by_measurement(
    tsdb_index_t* index,
    const char* measurement,
    tsdb_series_list_t* result
) {
    if (!index || !measurement || !result) return TSDB_ERR_INVALID_PARAM;
    
    size_t count = 0;
    for (size_t i = 0; i < index->series_count; i++) {
        if (index->all_series[i] && 
            strcmp(index->all_series[i]->measurement, measurement) == 0) {
            count++;
        }
    }
    
    result->capacity = count > 0 ? count : 1;
    result->series_ids = (uint64_t*)calloc(result->capacity, sizeof(uint64_t));
    if (!result->series_ids) return TSDB_ERR_NO_MEMORY;
    
    result->count = 0;
    for (size_t i = 0; i < index->series_count; i++) {
        if (index->all_series[i] && 
            strcmp(index->all_series[i]->measurement, measurement) == 0) {
            result->series_ids[result->count++] = index->all_series[i]->series_id;
        }
    }
    
    return TSDB_OK;
}

static bool tags_match(const tsdb_tag_t* tags1, size_t count1,
                       const tsdb_tag_t* tags2, size_t count2) {
    if (count1 != count2) return false;
    
    for (size_t i = 0; i < count1; i++) {
        bool found = false;
        for (size_t j = 0; j < count2; j++) {
            if (strcmp(tags1[i].key, tags2[j].key) == 0 &&
                strcmp(tags1[i].value, tags2[j].value) == 0) {
                found = true;
                break;
            }
        }
        if (!found) return false;
    }
    
    return true;
}

tsdb_status_t tsdb_index_find_by_tags(
    tsdb_index_t* index,
    const char* measurement,
    const tsdb_tag_t* tags,
    size_t tags_count,
    tsdb_series_list_t* result
) {
    if (!index || !measurement || !result) return TSDB_ERR_INVALID_PARAM;
    
    size_t count = 0;
    for (size_t i = 0; i < index->series_count; i++) {
        tsdb_series_info_t* info = index->all_series[i];
        if (info && strcmp(info->measurement, measurement) == 0 &&
            tags_match(info->tags, info->tags_count, tags, tags_count)) {
            count++;
        }
    }
    
    result->capacity = count > 0 ? count : 1;
    result->series_ids = (uint64_t*)calloc(result->capacity, sizeof(uint64_t));
    if (!result->series_ids) return TSDB_ERR_NO_MEMORY;
    
    result->count = 0;
    for (size_t i = 0; i < index->series_count; i++) {
        tsdb_series_info_t* info = index->all_series[i];
        if (info && strcmp(info->measurement, measurement) == 0 &&
            tags_match(info->tags, info->tags_count, tags, tags_count)) {
            result->series_ids[result->count++] = info->series_id;
        }
    }
    
    return TSDB_OK;
}

tsdb_status_t tsdb_index_find_by_time_range(
    tsdb_index_t* index,
    const char* measurement,
    tsdb_time_range_t* range,
    tsdb_series_list_t* result
) {
    if (!index || !measurement || !range || !result) return TSDB_ERR_INVALID_PARAM;
    
    size_t count = 0;
    for (size_t i = 0; i < index->series_count; i++) {
        tsdb_series_info_t* info = index->all_series[i];
        if (info && strcmp(info->measurement, measurement) == 0) {
            if (info->max_time >= range->start && info->min_time <= range->end) {
                count++;
            }
        }
    }
    
    result->capacity = count > 0 ? count : 1;
    result->series_ids = (uint64_t*)calloc(result->capacity, sizeof(uint64_t));
    if (!result->series_ids) return TSDB_ERR_NO_MEMORY;
    
    result->count = 0;
    for (size_t i = 0; i < index->series_count; i++) {
        tsdb_series_info_t* info = index->all_series[i];
        if (info && strcmp(info->measurement, measurement) == 0) {
            if (info->max_time >= range->start && info->min_time <= range->end) {
                result->series_ids[result->count++] = info->series_id;
            }
        }
    }
    
    return TSDB_OK;
}

tsdb_status_t tsdb_index_update_stats(
    tsdb_index_t* index,
    uint64_t series_id,
    tsdb_timestamp_t timestamp,
    uint64_t points_added
) {
    if (!index) return TSDB_ERR_INVALID_PARAM;
    
    tsdb_series_info_t* info = tsdb_index_get_series(index, series_id);
    if (!info) return TSDB_ERR_NOT_FOUND;
    
    if (timestamp < info->min_time || info->min_time == 0) {
        info->min_time = timestamp;
    }
    if (timestamp > info->max_time) {
        info->max_time = timestamp;
    }
    info->point_count += points_added;
    
    return TSDB_OK;
}

size_t tsdb_index_series_count(tsdb_index_t* index) {
    return index ? index->series_count : 0;
}

size_t tsdb_index_measurement_count(tsdb_index_t* index) {
    return index ? index->measurement_count : 0;
}

tsdb_status_t tsdb_index_persist(tsdb_index_t* index, const char* path) {
    if (!index || !path) return TSDB_ERR_INVALID_PARAM;
    
    FILE* fp = fopen(path, "wb");
    if (!fp) return TSDB_ERR_IO_ERROR;
    
    fwrite(&index->series_count, sizeof(size_t), 1, fp);
    fwrite(&index->measurement_count, sizeof(size_t), 1, fp);
    
    for (size_t i = 0; i < index->series_count; i++) {
        fwrite(index->all_series[i], sizeof(tsdb_series_info_t), 1, fp);
    }
    
    fclose(fp);
    return TSDB_OK;
}

tsdb_status_t tsdb_index_load(tsdb_index_t* index, const char* path) {
    if (!index || !path) return TSDB_ERR_INVALID_PARAM;
    
    FILE* fp = fopen(path, "rb");
    if (!fp) return TSDB_ERR_IO_ERROR;
    
    size_t series_count, measurement_count;
    fread(&series_count, sizeof(size_t), 1, fp);
    fread(&measurement_count, sizeof(size_t), 1, fp);
    
    for (size_t i = 0; i < series_count; i++) {
        tsdb_series_info_t info;
        fread(&info, sizeof(tsdb_series_info_t), 1, fp);
        tsdb_index_add_series(index, &info);
    }
    
    fclose(fp);
    return TSDB_OK;
}
