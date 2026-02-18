#ifndef TSDB_QUERY_H
#define TSDB_QUERY_H

#include "tsdb_types.h"
#include "tsdb_index.h"
#include "tsdb_storage.h"

typedef enum {
    TSDB_AGG_NONE = 0,
    TSDB_AGG_SUM = 1,
    TSDB_AGG_AVG = 2,
    TSDB_AGG_MIN = 3,
    TSDB_AGG_MAX = 4,
    TSDB_AGG_COUNT = 5,
    TSDB_AGG_FIRST = 6,
    TSDB_AGG_LAST = 7,
} tsdb_agg_type_t;

typedef enum {
    TSDB_OP_EQ = 0,
    TSDB_OP_NE = 1,
    TSDB_OP_LT = 2,
    TSDB_OP_LE = 3,
    TSDB_OP_GT = 4,
    TSDB_OP_GE = 5,
    TSDB_OP_AND = 6,
    TSDB_OP_OR = 7,
    TSDB_OP_NOT = 8,
} tsdb_op_type_t;

typedef struct tsdb_expr tsdb_expr_t;

struct tsdb_expr {
    tsdb_op_type_t op;
    char* tag_key;
    char* tag_value;
    tsdb_expr_t* left;
    tsdb_expr_t* right;
};

typedef struct {
    char measurement[128];
    tsdb_expr_t* filter;
    tsdb_time_range_t time_range;
    tsdb_agg_type_t aggregation;
    char* group_by_tag;
    char* field_name;
    size_t limit;
    size_t offset;
    bool ascending;
} tsdb_query_builder_t;

typedef struct {
    char group_key[256];
    tsdb_value_t value;
    tsdb_timestamp_t timestamp;
    size_t count;
} tsdb_agg_result_t;

typedef struct {
    tsdb_agg_result_t* results;
    size_t count;
    size_t capacity;
} tsdb_agg_result_set_t;

typedef struct tsdb_query_engine tsdb_query_engine_t;

tsdb_query_engine_t* tsdb_query_engine_create(tsdb_storage_t* storage, tsdb_index_t* index);
void tsdb_query_engine_destroy(tsdb_query_engine_t* engine);

tsdb_status_t tsdb_query_execute(
    tsdb_query_engine_t* engine,
    const tsdb_query_builder_t* query,
    tsdb_result_set_t* result
);

tsdb_status_t tsdb_query_aggregate(
    tsdb_query_engine_t* engine,
    const tsdb_query_builder_t* query,
    tsdb_agg_result_set_t* result
);

tsdb_expr_t* tsdb_expr_create(tsdb_op_type_t op);
void tsdb_expr_destroy(tsdb_expr_t* expr);

tsdb_expr_t* tsdb_expr_tag_filter(const char* key, tsdb_op_type_t op, const char* value);
tsdb_expr_t* tsdb_expr_and(tsdb_expr_t* left, tsdb_expr_t* right);
tsdb_expr_t* tsdb_expr_or(tsdb_expr_t* left, tsdb_expr_t* right);
tsdb_expr_t* tsdb_expr_not(tsdb_expr_t* expr);

bool tsdb_expr_evaluate(const tsdb_expr_t* expr, const tsdb_point_t* point);

tsdb_query_builder_t* tsdb_query_builder_create(void);
void tsdb_query_builder_destroy(tsdb_query_builder_t* builder);

tsdb_query_builder_t* tsdb_query_builder_set_measurement(tsdb_query_builder_t* builder, const char* name);
tsdb_query_builder_t* tsdb_query_builder_set_filter(tsdb_query_builder_t* builder, tsdb_expr_t* filter);
tsdb_query_builder_t* tsdb_query_builder_set_time_range(tsdb_query_builder_t* builder, tsdb_time_range_t range);
tsdb_query_builder_t* tsdb_query_builder_set_aggregation(tsdb_query_builder_t* builder, tsdb_agg_type_t agg, const char* field);
tsdb_query_builder_t* tsdb_query_builder_set_group_by(tsdb_query_builder_t* builder, const char* tag);
tsdb_query_builder_t* tsdb_query_builder_set_limit(tsdb_query_builder_t* builder, size_t limit, size_t offset);
tsdb_query_builder_t* tsdb_query_builder_set_order(tsdb_query_builder_t* builder, bool ascending);

#endif
