#ifndef TSDB25_QUERY_H
#define TSDB25_QUERY_H

#include "tsdb25_types.h"
#include "tsdb25_index.h"
#include "tsdb25_storage.h"

typedef struct tsdb25_expr      tsdb25_expr_t;
typedef struct tsdb25_query     tsdb25_query_t;
typedef struct tsdb25_query_engine tsdb25_query_engine_t;

typedef enum {
    TSDB25_OP_EQ  = 0,
    TSDB25_OP_NE  = 1,
    TSDB25_OP_LT  = 2,
    TSDB25_OP_LE  = 3,
    TSDB25_OP_GT  = 4,
    TSDB25_OP_GE  = 5,
    TSDB25_OP_AND = 6,
    TSDB25_OP_OR  = 7,
    TSDB25_OP_NOT = 8,
    TSDB25_OP_REGEX = 9,
} tsdb25_op_type_t;

struct tsdb25_expr {
    tsdb25_op_type_t op;
    char*            tag_key;
    char*            tag_value;
    tsdb25_expr_t*   left;
    tsdb25_expr_t*   right;
};

struct tsdb25_query {
    char             measurement[256];
    tsdb25_expr_t*   filter;
    tsdb25_time_range_t time_range;
    tsdb25_agg_type_t aggregation;
    char*            group_by_tag;
    char*            field_name;
    size_t           limit;
    size_t           offset;
    bool             ascending;
    bool             descending;
};

tsdb25_query_engine_t* tsdb25_query_engine_create(tsdb25_storage_t* storage, tsdb25_index_t* index);
void                   tsdb25_query_engine_destroy(tsdb25_query_engine_t* engine);

tsdb25_status_t tsdb25_query_execute(
    tsdb25_query_engine_t* engine,
    const tsdb25_query_t*   query,
    tsdb25_result_set_t*    result
);

tsdb25_status_t tsdb25_query_aggregate(
    tsdb25_query_engine_t*   engine,
    const tsdb25_query_t*    query,
    tsdb25_agg_result_set_t* result
);

tsdb25_expr_t* tsdb25_expr_create(tsdb25_op_type_t op);
void           tsdb25_expr_destroy(tsdb25_expr_t* expr);

tsdb25_expr_t* tsdb25_expr_tag(const char* key, tsdb25_op_type_t op, const char* value);
tsdb25_expr_t* tsdb25_expr_and(tsdb25_expr_t* left, tsdb25_expr_t* right);
tsdb25_expr_t* tsdb25_expr_or(tsdb25_expr_t* left, tsdb25_expr_t* right);
tsdb25_expr_t* tsdb25_expr_not(tsdb25_expr_t* expr);

bool tsdb25_expr_evaluate(const tsdb25_expr_t* expr, const tsdb25_point_t* point);

tsdb25_query_t* tsdb25_query_create(void);
void            tsdb25_query_destroy(tsdb25_query_t* query);

tsdb25_query_t* tsdb25_query_measurement(tsdb25_query_t* query, const char* name);
tsdb25_query_t* tsdb25_query_filter(tsdb25_query_t* query, tsdb25_expr_t* filter);
tsdb25_query_t* tsdb25_query_time_range(tsdb25_query_t* query, tsdb25_time_range_t range);
tsdb25_query_t* tsdb25_query_agg(tsdb25_query_t* query, tsdb25_agg_type_t agg, const char* field);
tsdb25_query_t* tsdb25_query_group_by(tsdb25_query_t* query, const char* tag);
tsdb25_query_t* tsdb25_query_limit(tsdb25_query_t* query, size_t limit, size_t offset);
tsdb25_query_t* tsdb25_query_order(tsdb25_query_t* query, bool ascending);

#endif
