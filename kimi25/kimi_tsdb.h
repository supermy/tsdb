#ifndef KIMI_TSDB_H
#define KIMI_TSDB_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

#define KIMI_TSDB_VERSION "2.5.0"

/* Error codes */
typedef enum {
    KIMI_OK = 0,
    KIMI_ERR_INVALID_PARAM = -1,
    KIMI_ERR_NO_MEMORY = -2,
    KIMI_ERR_NOT_FOUND = -3,
    KIMI_ERR_IO = -4,
    KIMI_ERR_CORRUPTED = -5,
    KIMI_ERR_EXISTS = -6,
    KIMI_ERR_TIMEOUT = -7,
    KIMI_ERR_FULL = -8
} kimi_error_t;

/* Data types */
typedef int64_t kimi_timestamp_t;
typedef double kimi_value_t;

/* Field types */
typedef enum {
    KIMI_FIELD_FLOAT = 0,
    KIMI_FIELD_INT,
    KIMI_FIELD_BOOL,
    KIMI_FIELD_STRING
} kimi_field_type_t;

/* Aggregation types */
typedef enum {
    KIMI_AGG_NONE = 0,
    KIMI_AGG_SUM,
    KIMI_AGG_AVG,
    KIMI_AGG_MIN,
    KIMI_AGG_MAX,
    KIMI_AGG_COUNT,
    KIMI_AGG_FIRST,
    KIMI_AGG_LAST
} kimi_agg_type_t;

/* Tag structure */
typedef struct {
    char key[64];
    char value[256];
} kimi_tag_t;

/* Field structure */
typedef struct {
    char name[128];
    kimi_field_type_t type;
    union {
        double f64;
        int64_t i64;
        bool boolean;
        char str[512];
    } value;
} kimi_field_t;

/* Data point */
typedef struct {
    char measurement[128];
    kimi_tag_t tags[32];
    size_t tag_count;
    kimi_field_t fields[64];
    size_t field_count;
    kimi_timestamp_t timestamp;
} kimi_point_t;

/* Time range */
typedef struct {
    kimi_timestamp_t start;
    kimi_timestamp_t end;
} kimi_range_t;

/* Query result */
typedef struct {
    kimi_point_t* points;
    size_t count;
    size_t capacity;
} kimi_result_t;

/* Aggregation result */
typedef struct {
    kimi_value_t value;
    size_t count;
    kimi_timestamp_t timestamp;
} kimi_agg_result_t;

/* Opaque handles */
typedef struct kimi_tsdb kimi_tsdb_t;
typedef struct kimi_query kimi_query_t;

/* Configuration */
typedef struct {
    size_t cache_size_mb;
    size_t max_series;
    bool enable_wal;
    bool sync_write;
    size_t block_size;
} kimi_config_t;

/* API Functions */

/* Database lifecycle */
kimi_tsdb_t* kimi_tsdb_open(const char* path, const kimi_config_t* config);
void kimi_tsdb_close(kimi_tsdb_t* db);
const char* kimi_tsdb_version(void);
const char* kimi_strerror(kimi_error_t err);

/* Write operations */
kimi_error_t kimi_write(kimi_tsdb_t* db, const kimi_point_t* point);
kimi_error_t kimi_write_batch(kimi_tsdb_t* db, const kimi_point_t* points, size_t count);

/* Read operations */
kimi_error_t kimi_query(kimi_tsdb_t* db, const kimi_query_t* query, kimi_result_t* result);
kimi_error_t kimi_query_agg(kimi_tsdb_t* db, const kimi_query_t* query, kimi_agg_type_t agg, 
                            const char* field, kimi_agg_result_t* result);

/* Query builder */
kimi_query_t* kimi_query_new(const char* measurement);
void kimi_query_free(kimi_query_t* query);
kimi_query_t* kimi_query_range(kimi_query_t* query, kimi_range_t range);
kimi_query_t* kimi_query_limit(kimi_query_t* query, size_t limit);
kimi_query_t* kimi_query_tag(kimi_query_t* query, const char* key, const char* value);

/* Point builder */
kimi_point_t* kimi_point_create(const char* measurement, kimi_timestamp_t ts);
void kimi_point_destroy(kimi_point_t* point);
kimi_point_t* kimi_point_add_tag(kimi_point_t* point, const char* key, const char* value);
kimi_point_t* kimi_point_add_field_f64(kimi_point_t* point, const char* name, double value);
kimi_point_t* kimi_point_add_field_i64(kimi_point_t* point, const char* name, int64_t value);
kimi_point_t* kimi_point_add_field_bool(kimi_point_t* point, const char* name, bool value);
kimi_point_t* kimi_point_add_field_str(kimi_point_t* point, const char* name, const char* value);

/* Result management */
void kimi_result_free(kimi_result_t* result);

/* Utility */
kimi_timestamp_t kimi_now(void);
kimi_timestamp_t kimi_now_ms(void);

#ifdef __cplusplus
}
#endif

#endif
