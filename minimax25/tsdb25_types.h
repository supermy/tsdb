#ifndef TSDB25_TYPES_H
#define TSDB25_TYPES_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#define TSDB25_VERSION "2.5.0"

#define TSDB25_MAX_TAG_KEY_LEN   64
#define TSDB25_MAX_TAG_VALUE_LEN 256
#define TSDB25_MAX_FIELD_NAME_LEN 128
#define TSDB25_MAX_TAGS_COUNT     32
#define TSDB25_MAX_FIELDS_COUNT    64
#define TSDB25_MAX_PATH_LEN        512
#define TSDB25_DEFAULT_BLOCK_SIZE  8192

typedef int64_t  tsdb25_timestamp_t;
typedef double   tsdb25_value_t;

typedef enum {
    TSDB25_OK = 0,
    TSDB25_ERR_INVALID_PARAM   = -1,
    TSDB25_ERR_NO_MEMORY       = -2,
    TSDB25_ERR_NOT_FOUND       = -3,
    TSDB25_ERR_IO_ERROR        = -4,
    TSDB25_ERR_CORRUPTED       = -5,
    TSDB25_ERR_FULL            = -6,
    TSDB25_ERR_ALREADY_EXISTS  = -7,
    TSDB25_ERR_INVALID_STATE   = -8,
    TSDB25_ERR_COMPRESSION     = -9,
    TSDB25_ERR_TIMEOUT         = -10,
    TSDB25_ERR_LOCKED          = -11,
} tsdb25_status_t;

typedef enum {
    TSDB25_FIELD_FLOAT   = 0,
    TSDB25_FIELD_INTEGER = 1,
    TSDB25_FIELD_STRING  = 2,
    TSDB25_FIELD_BOOLEAN = 3,
    TSDB25_FIELD_UINT64  = 4,
} tsdb25_field_type_t;

typedef struct {
    char key[TSDB25_MAX_TAG_KEY_LEN];
    char value[TSDB25_MAX_TAG_VALUE_LEN];
} tsdb25_tag_t;

typedef struct {
    char name[TSDB25_MAX_FIELD_NAME_LEN];
    tsdb25_field_type_t type;
    union {
        double   float_val;
        int64_t  int_val;
        uint64_t uint_val;
        char     str_val[512];
        bool     bool_val;
    } value;
} tsdb25_field_t;

typedef struct {
    char measurement[256];
    tsdb25_tag_t     tags[TSDB25_MAX_TAGS_COUNT];
    size_t           tags_count;
    tsdb25_field_t   fields[TSDB25_MAX_FIELDS_COUNT];
    size_t           fields_count;
    tsdb25_timestamp_t timestamp;
} tsdb25_point_t;

typedef struct {
    tsdb25_timestamp_t start;
    tsdb25_timestamp_t end;
} tsdb25_time_range_t;

typedef struct {
    tsdb25_point_t* points;
    size_t count;
    size_t capacity;
} tsdb25_result_set_t;

typedef struct {
    char group_key[512];
    tsdb25_value_t value;
    tsdb25_timestamp_t timestamp;
    size_t count;
} tsdb25_agg_result_t;

typedef struct {
    tsdb25_agg_result_t* results;
    size_t count;
    size_t capacity;
} tsdb25_agg_result_set_t;

typedef enum {
    TSDB25_AGG_NONE   = 0,
    TSDB25_AGG_SUM    = 1,
    TSDB25_AGG_AVG    = 2,
    TSDB25_AGG_MIN    = 3,
    TSDB25_AGG_MAX    = 4,
    TSDB25_AGG_COUNT  = 5,
    TSDB25_AGG_FIRST  = 6,
    TSDB25_AGG_LAST   = 7,
    TSDB25_AGG_STDDEV = 8,
    TSDB25_AGG_MEDIAN = 9,
    TSDB25_AGG_PERCENTILE_95 = 10,
    TSDB25_AGG_PERCENTILE_99 = 11,
} tsdb25_agg_type_t;

typedef enum {
    TSDB25_COMPRESS_NONE   = 0,
    TSDB25_COMPRESS_SNAPPY = 1,
    TSDB25_COMPRESS_ZSTD   = 2,
    TSDB25_COMPRESS_GORILLA = 3,
} tsdb25_compress_type_t;

const char* tsdb25_strerror(tsdb25_status_t status);

#endif
