#include "tsdb_query.h"
#include <stdlib.h>
#include <string.h>
#include <math.h>

extern void tsdb_result_set_destroy(tsdb_result_set_t* result);

struct tsdb_query_engine {
    tsdb_storage_t* storage;
    tsdb_index_t* index;
};

tsdb_query_engine_t* tsdb_query_engine_create(tsdb_storage_t* storage, tsdb_index_t* index) {
    tsdb_query_engine_t* engine = (tsdb_query_engine_t*)calloc(1, sizeof(tsdb_query_engine_t));
    if (!engine) return NULL;
    
    engine->storage = storage;
    engine->index = index;
    
    return engine;
}

void tsdb_query_engine_destroy(tsdb_query_engine_t* engine) {
    free(engine);
}

tsdb_expr_t* tsdb_expr_create(tsdb_op_type_t op) {
    tsdb_expr_t* expr = (tsdb_expr_t*)calloc(1, sizeof(tsdb_expr_t));
    if (!expr) return NULL;
    
    expr->op = op;
    return expr;
}

void tsdb_expr_destroy(tsdb_expr_t* expr) {
    if (!expr) return;
    
    free(expr->tag_key);
    free(expr->tag_value);
    tsdb_expr_destroy(expr->left);
    tsdb_expr_destroy(expr->right);
    free(expr);
}

tsdb_expr_t* tsdb_expr_tag_filter(const char* key, tsdb_op_type_t op, const char* value) {
    tsdb_expr_t* expr = tsdb_expr_create(op);
    if (!expr) return NULL;
    
    expr->tag_key = strdup(key);
    expr->tag_value = strdup(value);
    
    return expr;
}

tsdb_expr_t* tsdb_expr_and(tsdb_expr_t* left, tsdb_expr_t* right) {
    tsdb_expr_t* expr = tsdb_expr_create(TSDB_OP_AND);
    if (!expr) return NULL;
    
    expr->left = left;
    expr->right = right;
    
    return expr;
}

tsdb_expr_t* tsdb_expr_or(tsdb_expr_t* left, tsdb_expr_t* right) {
    tsdb_expr_t* expr = tsdb_expr_create(TSDB_OP_OR);
    if (!expr) return NULL;
    
    expr->left = left;
    expr->right = right;
    
    return expr;
}

tsdb_expr_t* tsdb_expr_not(tsdb_expr_t* expr) {
    tsdb_expr_t* not_expr = tsdb_expr_create(TSDB_OP_NOT);
    if (!not_expr) return NULL;
    
    not_expr->left = expr;
    
    return not_expr;
}

bool tsdb_expr_evaluate(const tsdb_expr_t* expr, const tsdb_point_t* point) {
    if (!expr || !point) return false;
    
    switch (expr->op) {
        case TSDB_OP_EQ:
            for (size_t i = 0; i < point->tags_count; i++) {
                if (strcmp(point->tags[i].key, expr->tag_key) == 0) {
                    return strcmp(point->tags[i].value, expr->tag_value) == 0;
                }
            }
            return false;
            
        case TSDB_OP_NE:
            for (size_t i = 0; i < point->tags_count; i++) {
                if (strcmp(point->tags[i].key, expr->tag_key) == 0) {
                    return strcmp(point->tags[i].value, expr->tag_value) != 0;
                }
            }
            return true;
            
        case TSDB_OP_AND:
            return tsdb_expr_evaluate(expr->left, point) && 
                   tsdb_expr_evaluate(expr->right, point);
            
        case TSDB_OP_OR:
            return tsdb_expr_evaluate(expr->left, point) || 
                   tsdb_expr_evaluate(expr->right, point);
            
        case TSDB_OP_NOT:
            return !tsdb_expr_evaluate(expr->left, point);
            
        default:
            return false;
    }
}

tsdb_query_builder_t* tsdb_query_builder_create(void) {
    tsdb_query_builder_t* builder = (tsdb_query_builder_t*)calloc(1, sizeof(tsdb_query_builder_t));
    if (!builder) return NULL;
    
    builder->limit = 1000;
    builder->ascending = true;
    
    return builder;
}

void tsdb_query_builder_destroy(tsdb_query_builder_t* builder) {
    if (!builder) return;
    
    tsdb_expr_destroy(builder->filter);
    free(builder->group_by_tag);
    free(builder->field_name);
    free(builder);
}

tsdb_query_builder_t* tsdb_query_builder_set_measurement(tsdb_query_builder_t* builder, const char* name) {
    if (builder && name) {
        strncpy(builder->measurement, name, sizeof(builder->measurement) - 1);
    }
    return builder;
}

tsdb_query_builder_t* tsdb_query_builder_set_filter(tsdb_query_builder_t* builder, tsdb_expr_t* filter) {
    if (builder) {
        builder->filter = filter;
    }
    return builder;
}

tsdb_query_builder_t* tsdb_query_builder_set_time_range(tsdb_query_builder_t* builder, tsdb_time_range_t range) {
    if (builder) {
        builder->time_range = range;
    }
    return builder;
}

tsdb_query_builder_t* tsdb_query_builder_set_aggregation(tsdb_query_builder_t* builder, tsdb_agg_type_t agg, const char* field) {
    if (builder) {
        builder->aggregation = agg;
        builder->field_name = field ? strdup(field) : NULL;
    }
    return builder;
}

tsdb_query_builder_t* tsdb_query_builder_set_group_by(tsdb_query_builder_t* builder, const char* tag) {
    if (builder) {
        free(builder->group_by_tag);
        builder->group_by_tag = tag ? strdup(tag) : NULL;
    }
    return builder;
}

tsdb_query_builder_t* tsdb_query_builder_set_limit(tsdb_query_builder_t* builder, size_t limit, size_t offset) {
    if (builder) {
        builder->limit = limit;
        builder->offset = offset;
    }
    return builder;
}

tsdb_query_builder_t* tsdb_query_builder_set_order(tsdb_query_builder_t* builder, bool ascending) {
    if (builder) {
        builder->ascending = ascending;
    }
    return builder;
}

static int compare_points_asc(const void* a, const void* b) {
    const tsdb_point_t* pa = (const tsdb_point_t*)a;
    const tsdb_point_t* pb = (const tsdb_point_t*)b;
    
    if (pa->timestamp < pb->timestamp) return -1;
    if (pa->timestamp > pb->timestamp) return 1;
    return 0;
}

static int compare_points_desc(const void* a, const void* b) {
    return -compare_points_asc(a, b);
}

tsdb_status_t tsdb_query_execute(
    tsdb_query_engine_t* engine,
    const tsdb_query_builder_t* query,
    tsdb_result_set_t* result
) {
    if (!engine || !query || !result) return TSDB_ERR_INVALID_PARAM;
    
    memset(result, 0, sizeof(tsdb_result_set_t));
    
    tsdb_series_list_t series_list;
    tsdb_status_t status = tsdb_index_find_by_measurement(
        engine->index, query->measurement, &series_list);
    
    if (status != TSDB_OK) return status;
    
    result->capacity = 1024;
    result->points = (tsdb_point_t*)calloc(result->capacity, sizeof(tsdb_point_t));
    if (!result->points) {
        free(series_list.series_ids);
        return TSDB_ERR_NO_MEMORY;
    }
    
    for (size_t i = 0; i < series_list.count && result->count < query->limit; i++) {
        tsdb_result_set_t series_result;
        status = tsdb_storage_read_series(
            engine->storage, series_list.series_ids[i], 
            (tsdb_time_range_t*)&query->time_range, &series_result);
        
        if (status == TSDB_OK && series_result.count > 0) {
            for (size_t j = 0; j < series_result.count && result->count < query->limit; j++) {
                if (result->count >= result->capacity) {
                    result->capacity *= 2;
                    tsdb_point_t* new_points = (tsdb_point_t*)realloc(
                        result->points, result->capacity * sizeof(tsdb_point_t));
                    if (!new_points) {
                        free(series_result.points);
                        free(series_list.series_ids);
                        return TSDB_ERR_NO_MEMORY;
                    }
                    result->points = new_points;
                }
                
                result->points[result->count++] = series_result.points[j];
            }
            
            free(series_result.points);
        }
    }
    
    free(series_list.series_ids);
    
    if (query->ascending) {
        qsort(result->points, result->count, sizeof(tsdb_point_t), compare_points_asc);
    } else {
        qsort(result->points, result->count, sizeof(tsdb_point_t), compare_points_desc);
    }
    
    return TSDB_OK;
}

tsdb_status_t tsdb_query_aggregate(
    tsdb_query_engine_t* engine,
    const tsdb_query_builder_t* query,
    tsdb_agg_result_set_t* result
) {
    if (!engine || !query || !result) return TSDB_ERR_INVALID_PARAM;
    
    memset(result, 0, sizeof(tsdb_agg_result_set_t));
    
    tsdb_result_set_t raw_result;
    tsdb_status_t status = tsdb_query_execute(engine, query, &raw_result);
    if (status != TSDB_OK) return status;
    
    if (raw_result.count == 0) {
        tsdb_result_set_destroy(&raw_result);
        return TSDB_OK;
    }
    
    result->capacity = 1;
    result->results = (tsdb_agg_result_t*)calloc(result->capacity, sizeof(tsdb_agg_result_t));
    if (!result->results) {
        tsdb_result_set_destroy(&raw_result);
        return TSDB_ERR_NO_MEMORY;
    }
    
    tsdb_agg_result_t* agg = &result->results[0];
    agg->count = raw_result.count;
    
    switch (query->aggregation) {
        case TSDB_AGG_COUNT:
            agg->value = (tsdb_value_t)raw_result.count;
            break;
            
        case TSDB_AGG_SUM:
            for (size_t i = 0; i < raw_result.count; i++) {
                if (raw_result.points[i].fields_count > 0) {
                    agg->value += raw_result.points[i].fields[0].value.float_val;
                }
            }
            break;
            
        case TSDB_AGG_AVG:
            for (size_t i = 0; i < raw_result.count; i++) {
                if (raw_result.points[i].fields_count > 0) {
                    agg->value += raw_result.points[i].fields[0].value.float_val;
                }
            }
            agg->value /= (tsdb_value_t)raw_result.count;
            break;
            
        case TSDB_AGG_MIN:
            agg->value = INFINITY;
            for (size_t i = 0; i < raw_result.count; i++) {
                if (raw_result.points[i].fields_count > 0) {
                    tsdb_value_t v = raw_result.points[i].fields[0].value.float_val;
                    if (v < agg->value) agg->value = v;
                }
            }
            break;
            
        case TSDB_AGG_MAX:
            agg->value = -INFINITY;
            for (size_t i = 0; i < raw_result.count; i++) {
                if (raw_result.points[i].fields_count > 0) {
                    tsdb_value_t v = raw_result.points[i].fields[0].value.float_val;
                    if (v > agg->value) agg->value = v;
                }
            }
            break;
            
        case TSDB_AGG_FIRST:
            if (raw_result.points[0].fields_count > 0) {
                agg->value = raw_result.points[0].fields[0].value.float_val;
                agg->timestamp = raw_result.points[0].timestamp;
            }
            break;
            
        case TSDB_AGG_LAST:
            if (raw_result.points[raw_result.count - 1].fields_count > 0) {
                agg->value = raw_result.points[raw_result.count - 1].fields[0].value.float_val;
                agg->timestamp = raw_result.points[raw_result.count - 1].timestamp;
            }
            break;
            
        default:
            break;
    }
    
    result->count = 1;
    tsdb_result_set_destroy(&raw_result);
    
    return TSDB_OK;
}
