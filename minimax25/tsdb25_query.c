#include "tsdb25_query.h"
#include <stdlib.h>
#include <string.h>
#include <math.h>

extern void tsdb25_result_free(tsdb25_result_set_t* result);

struct tsdb25_query_engine {
    tsdb25_storage_t* storage;
    tsdb25_index_t*   index;
};

tsdb25_query_engine_t* tsdb25_query_engine_create(tsdb25_storage_t* storage, tsdb25_index_t* index) {
    tsdb25_query_engine_t* e = (tsdb25_query_engine_t*)calloc(1, sizeof(tsdb25_query_engine_t));
    if (!e) return NULL;
    e->storage = storage;
    e->index = index;
    return e;
}

void tsdb25_query_engine_destroy(tsdb25_query_engine_t* engine) {
    free(engine);
}

tsdb25_expr_t* tsdb25_expr_create(tsdb25_op_type_t op) {
    tsdb25_expr_t* e = (tsdb25_expr_t*)calloc(1, sizeof(tsdb25_expr_t));
    if (!e) return NULL;
    e->op = op;
    return e;
}

void tsdb25_expr_destroy(tsdb25_expr_t* expr) {
    if (!expr) return;
    free(expr->tag_key);
    free(expr->tag_value);
    tsdb25_expr_destroy(expr->left);
    tsdb25_expr_destroy(expr->right);
    free(expr);
}

tsdb25_expr_t* tsdb25_expr_tag(const char* key, tsdb25_op_type_t op, const char* value) {
    tsdb25_expr_t* e = tsdb25_expr_create(op);
    if (!e) return NULL;
    e->tag_key = strdup(key);
    e->tag_value = strdup(value);
    return e;
}

tsdb25_expr_t* tsdb25_expr_and(tsdb25_expr_t* left, tsdb25_expr_t* right) {
    tsdb25_expr_t* e = tsdb25_expr_create(TSDB25_OP_AND);
    if (!e) return NULL;
    e->left = left;
    e->right = right;
    return e;
}

tsdb25_expr_t* tsdb25_expr_or(tsdb25_expr_t* left, tsdb25_expr_t* right) {
    tsdb25_expr_t* e = tsdb25_expr_create(TSDB25_OP_OR);
    if (!e) return NULL;
    e->left = left;
    e->right = right;
    return e;
}

tsdb25_expr_t* tsdb25_expr_not(tsdb25_expr_t* expr) {
    tsdb25_expr_t* e = tsdb25_expr_create(TSDB25_OP_NOT);
    if (!e) return NULL;
    e->left = expr;
    return e;
}

bool tsdb25_expr_evaluate(const tsdb25_expr_t* expr, const tsdb25_point_t* point) {
    if (!expr || !point) return false;
    
    switch (expr->op) {
        case TSDB25_OP_EQ:
            for (size_t i = 0; i < point->tags_count; i++) {
                if (strcmp(point->tags[i].key, expr->tag_key) == 0) {
                    return strcmp(point->tags[i].value, expr->tag_value) == 0;
                }
            }
            return false;
        case TSDB25_OP_NE:
            for (size_t i = 0; i < point->tags_count; i++) {
                if (strcmp(point->tags[i].key, expr->tag_key) == 0) {
                    return strcmp(point->tags[i].value, expr->tag_value) != 0;
                }
            }
            return true;
        case TSDB25_OP_AND:
            return tsdb25_expr_evaluate(expr->left, point) && tsdb25_expr_evaluate(expr->right, point);
        case TSDB25_OP_OR:
            return tsdb25_expr_evaluate(expr->left, point) || tsdb25_expr_evaluate(expr->right, point);
        case TSDB25_OP_NOT:
            return !tsdb25_expr_evaluate(expr->left, point);
        default:
            return false;
    }
}

tsdb25_query_t* tsdb25_query_create(void) {
    tsdb25_query_t* q = (tsdb25_query_t*)calloc(1, sizeof(tsdb25_query_t));
    if (!q) return NULL;
    q->limit = 10000;
    q->ascending = true;
    return q;
}

void tsdb25_query_destroy(tsdb25_query_t* query) {
    if (!query) return;
    tsdb25_expr_destroy(query->filter);
    free(query->group_by_tag);
    free(query->field_name);
    free(query);
}

tsdb25_query_t* tsdb25_query_measurement(tsdb25_query_t* query, const char* name) {
    if (query && name) strncpy(query->measurement, name, sizeof(query->measurement) - 1);
    return query;
}

tsdb25_query_t* tsdb25_query_filter(tsdb25_query_t* query, tsdb25_expr_t* filter) {
    if (query) query->filter = filter;
    return query;
}

tsdb25_query_t* tsdb25_query_time_range(tsdb25_query_t* query, tsdb25_time_range_t range) {
    if (query) query->time_range = range;
    return query;
}

tsdb25_query_t* tsdb25_query_set_agg(tsdb25_query_t* query, tsdb25_agg_type_t agg, const char* field) {
    if (query) {
        query->aggregation = agg;
        free(query->field_name);
        query->field_name = field ? strdup(field) : NULL;
    }
    return query;
}

tsdb25_query_t* tsdb25_query_group_by(tsdb25_query_t* query, const char* tag) {
    if (query) {
        free(query->group_by_tag);
        query->group_by_tag = tag ? strdup(tag) : NULL;
    }
    return query;
}

tsdb25_query_t* tsdb25_query_limit(tsdb25_query_t* query, size_t limit, size_t offset) {
    if (query) {
        query->limit = limit;
        query->offset = offset;
    }
    return query;
}

tsdb25_query_t* tsdb25_query_order(tsdb25_query_t* query, bool ascending) {
    if (query) {
        query->ascending = ascending;
        query->descending = !ascending;
    }
    return query;
}

static int cmp_point_asc(const void* a, const void* b) {
    const tsdb25_point_t* pa = (const tsdb25_point_t*)a;
    const tsdb25_point_t* pb = (const tsdb25_point_t*)b;
    if (pa->timestamp < pb->timestamp) return -1;
    if (pa->timestamp > pb->timestamp) return 1;
    return 0;
}

static int cmp_point_desc(const void* a, const void* b) {
    return -cmp_point_asc(a, b);
}

tsdb25_status_t tsdb25_query_execute(tsdb25_query_engine_t* engine, const tsdb25_query_t* query,
                                     tsdb25_result_set_t* result) {
    if (!engine || !query || !result) return TSDB25_ERR_INVALID_PARAM;
    
    memset(result, 0, sizeof(tsdb25_result_set_t));
    
    tsdb25_series_list_t list;
    tsdb25_status_t status = tsdb25_index_find_by_measurement(engine->index, query->measurement, &list);
    if (status != TSDB25_OK) return status;
    
    result->capacity = 4096;
    result->points = (tsdb25_point_t*)calloc(result->capacity, sizeof(tsdb25_point_t));
    if (!result->points) {
        free(list.series_ids);
        return TSDB25_ERR_NO_MEMORY;
    }
    
    for (size_t i = 0; i < list.count && result->count < query->limit; i++) {
        tsdb25_result_set_t series_result;
        status = tsdb25_storage_read_series(engine->storage, list.series_ids[i],
                                            (tsdb25_time_range_t*)&query->time_range, &series_result);
        if (status == TSDB25_OK && series_result.count > 0) {
            for (size_t j = 0; j < series_result.count && result->count < query->limit; j++) {
                if (result->count >= result->capacity) {
                    result->capacity *= 2;
                    tsdb25_point_t* new_pts = (tsdb25_point_t*)realloc(result->points, result->capacity * sizeof(tsdb25_point_t));
                    if (!new_pts) {
                        free(series_result.points);
                        free(list.series_ids);
                        return TSDB25_ERR_NO_MEMORY;
                    }
                    result->points = new_pts;
                }
                result->points[result->count++] = series_result.points[j];
            }
            free(series_result.points);
        }
    }
    
    free(list.series_ids);
    
    if (query->ascending) {
        qsort(result->points, result->count, sizeof(tsdb25_point_t), cmp_point_asc);
    } else if (query->descending) {
        qsort(result->points, result->count, sizeof(tsdb25_point_t), cmp_point_desc);
    }
    
    return TSDB25_OK;
}

static int cmp_double_asc(const void* a, const void* b) {
    double da = *(const double*)a;
    double db = *(const double*)b;
    if (da < db) return -1;
    if (da > db) return 1;
    return 0;
}

tsdb25_status_t tsdb25_query_aggregate_engine(tsdb25_query_engine_t* engine, const tsdb25_query_t* query,
                                       tsdb25_agg_result_set_t* result) {
    if (!engine || !query || !result) return TSDB25_ERR_INVALID_PARAM;
    
    memset(result, 0, sizeof(tsdb25_agg_result_set_t));
    
    tsdb25_result_set_t raw;
    tsdb25_status_t status = tsdb25_query_execute(engine, query, &raw);
    if (status != TSDB25_OK) return status;
    
    if (raw.count == 0) {
        return TSDB25_OK;
    }
    
    result->capacity = 1;
    result->results = (tsdb25_agg_result_t*)calloc(result->capacity, sizeof(tsdb25_agg_result_t));
    if (!result->results) {
        tsdb25_result_free(&raw);
        return TSDB25_ERR_NO_MEMORY;
    }
    
    tsdb25_agg_result_t* agg = &result->results[0];
    agg->count = raw.count;
    
    switch (query->aggregation) {
        case TSDB25_AGG_COUNT:
            agg->value = (tsdb25_value_t)raw.count;
            break;
        case TSDB25_AGG_SUM:
            for (size_t i = 0; i < raw.count; i++) {
                if (raw.points[i].fields_count > 0) {
                    agg->value += raw.points[i].fields[0].value.float_val;
                }
            }
            break;
        case TSDB25_AGG_AVG:
            for (size_t i = 0; i < raw.count; i++) {
                if (raw.points[i].fields_count > 0) {
                    agg->value += raw.points[i].fields[0].value.float_val;
                }
            }
            agg->value /= (tsdb25_value_t)raw.count;
            break;
        case TSDB25_AGG_MIN:
            agg->value = INFINITY;
            for (size_t i = 0; i < raw.count; i++) {
                if (raw.points[i].fields_count > 0) {
                    double v = raw.points[i].fields[0].value.float_val;
                    if (v < agg->value) agg->value = v;
                }
            }
            break;
        case TSDB25_AGG_MAX:
            agg->value = -INFINITY;
            for (size_t i = 0; i < raw.count; i++) {
                if (raw.points[i].fields_count > 0) {
                    double v = raw.points[i].fields[0].value.float_val;
                    if (v > agg->value) agg->value = v;
                }
            }
            break;
        case TSDB25_AGG_FIRST:
            if (raw.points[0].fields_count > 0) {
                agg->value = raw.points[0].fields[0].value.float_val;
                agg->timestamp = raw.points[0].timestamp;
            }
            break;
        case TSDB25_AGG_LAST:
            if (raw.points[raw.count - 1].fields_count > 0) {
                agg->value = raw.points[raw.count - 1].fields[0].value.float_val;
                agg->timestamp = raw.points[raw.count - 1].timestamp;
            }
            break;
        case TSDB25_AGG_STDDEV: {
            double sum = 0, sum_sq = 0;
            for (size_t i = 0; i < raw.count; i++) {
                if (raw.points[i].fields_count > 0) {
                    double v = raw.points[i].fields[0].value.float_val;
                    sum += v;
                    sum_sq += v * v;
                }
            }
            double mean = sum / raw.count;
            agg->value = sqrt(sum_sq / raw.count - mean * mean);
            break;
        }
        case TSDB25_AGG_MEDIAN: {
            double* vals = (double*)malloc(raw.count * sizeof(double));
            if (!vals) break;
            size_t n = 0;
            for (size_t i = 0; i < raw.count; i++) {
                if (raw.points[i].fields_count > 0) {
                    vals[n++] = raw.points[i].fields[0].value.float_val;
                }
            }
            if (n > 0) {
                qsort(vals, n, sizeof(double), cmp_double_asc);
                agg->value = vals[n / 2];
            }
            free(vals);
            break;
        }
        default:
            break;
    }
    
    result->count = 1;
    tsdb25_result_free(&raw);
    return TSDB25_OK;
}
