#ifndef RTSDB_TYPES_H
#define RTSDB_TYPES_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#define RTSDB_VERSION "1.0.0-rocksdb"

#define RTSDB_MAX_TAG_KEY_LEN     64
#define RTSDB_MAX_TAG_VALUE_LEN   256
#define RTSDB_MAX_FIELD_NAME_LEN  128
#define RTSDB_MAX_TAGS_COUNT      32
#define RTSDB_MAX_FIELDS_COUNT    64
#define RTSDB_MAX_PATH_LEN        512
#define RTSDB_MAX_MEASUREMENT_LEN 128

typedef enum {
    RTSDB_OK = 0,
    RTSDB_ERR_INVALID_PARAM   = -1,
    RTSDB_ERR_NO_MEMORY       = -2,
    RTSDB_ERR_NOT_FOUND       = -3,
    RTSDB_ERR_IO              = -4,
    RTSDB_ERR_CORRUPTED       = -5,
    RTSDB_ERR_FULL            = -6,
    RTSDB_ERR_EXISTS          = -7,
    RTSDB_ERR_INVALID_STATE   = -8,
    RTSDB_ERR_TIMEOUT         = -9,
} rtsdb_status_t;

typedef int64_t  rtsdb_timestamp_t;
typedef double   rtsdb_value_t;

typedef enum {
    RTSDB_FIELD_FLOAT   = 0,
    RTSDB_FIELD_INTEGER = 1,
    RTSDB_FIELD_STRING  = 2,
    RTSDB_FIELD_BOOLEAN = 3,
    RTSDB_FIELD_UINT64  = 4,
} rtsdb_field_type_t;

typedef enum {
    RTSDB_AGG_NONE   = 0,
    RTSDB_AGG_SUM    = 1,
    RTSDB_AGG_AVG    = 2,
    RTSDB_AGG_MIN    = 3,
    RTSDB_AGG_MAX    = 4,
    RTSDB_AGG_COUNT  = 5,
    RTSDB_AGG_FIRST  = 6,
    RTSDB_AGG_LAST   = 7,
    RTSDB_AGG_STDDEV = 8,
    RTSDB_AGG_MEDIAN = 9,
} rtsdb_agg_type_t;

typedef struct {
    char key[RTSDB_MAX_TAG_KEY_LEN];
    char value[RTSDB_MAX_TAG_VALUE_LEN];
} rtsdb_tag_t;

typedef struct {
    char name[RTSDB_MAX_FIELD_NAME_LEN];
    rtsdb_field_type_t type;
    union {
        double   float_val;
        int64_t  int_val;
        uint64_t uint_val;
        char     str_val[512];
        bool     bool_val;
    } value;
} rtsdb_field_t;

typedef struct {
    char measurement[RTSDB_MAX_MEASUREMENT_LEN];
    rtsdb_tag_t   tags[RTSDB_MAX_TAGS_COUNT];
    size_t        tags_count;
    rtsdb_field_t fields[RTSDB_MAX_FIELDS_COUNT];
    size_t        fields_count;
    rtsdb_timestamp_t timestamp;
} rtsdb_point_t;

typedef struct {
    rtsdb_timestamp_t start;
    rtsdb_timestamp_t end;
} rtsdb_time_range_t;

typedef struct {
    rtsdb_point_t* points;
    size_t count;
    size_t capacity;
} rtsdb_result_set_t;

typedef struct {
    char group_key[512];
    rtsdb_value_t value;
    rtsdb_timestamp_t timestamp;
    size_t count;
} rtsdb_agg_result_t;

typedef struct {
    rtsdb_agg_result_t* results;
    size_t count;
    size_t capacity;
} rtsdb_agg_result_set_t;

const char* rtsdb_strerror(rtsdb_status_t status);

#endif
