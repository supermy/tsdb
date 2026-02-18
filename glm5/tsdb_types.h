#ifndef TSDB_TYPES_H
#define TSDB_TYPES_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#define TSDB_VERSION_MAJOR 1
#define TSDB_VERSION_MINOR 0
#define TSDB_VERSION_PATCH 0

#define TSDB_MAX_TAG_KEY_LEN 64
#define TSDB_MAX_TAG_VALUE_LEN 256
#define TSDB_MAX_FIELD_NAME_LEN 128
#define TSDB_MAX_TAGS_COUNT 16
#define TSDB_MAX_FIELDS_COUNT 32
#define TSDB_DEFAULT_BLOCK_SIZE 4096
#define TSDB_MAX_PATH_LEN 512

typedef int64_t tsdb_timestamp_t;
typedef double tsdb_value_t;

typedef enum {
    TSDB_OK = 0,
    TSDB_ERR_INVALID_PARAM = -1,
    TSDB_ERR_NO_MEMORY = -2,
    TSDB_ERR_NOT_FOUND = -3,
    TSDB_ERR_IO_ERROR = -4,
    TSDB_ERR_CORRUPTED = -5,
    TSDB_ERR_FULL = -6,
    TSDB_ERR_ALREADY_EXISTS = -7,
    TSDB_ERR_INVALID_STATE = -8,
    TSDB_ERR_COMPRESSION = -9,
    TSDB_ERR_TIMEOUT = -10,
} tsdb_status_t;

typedef enum {
    TSDB_FIELD_FLOAT = 0,
    TSDB_FIELD_INTEGER = 1,
    TSDB_FIELD_STRING = 2,
    TSDB_FIELD_BOOLEAN = 3,
} tsdb_field_type_t;

typedef struct {
    char key[TSDB_MAX_TAG_KEY_LEN];
    char value[TSDB_MAX_TAG_VALUE_LEN];
} tsdb_tag_t;

typedef struct {
    char name[TSDB_MAX_FIELD_NAME_LEN];
    tsdb_field_type_t type;
    union {
        double float_val;
        int64_t int_val;
        char str_val[256];
        bool bool_val;
    } value;
} tsdb_field_t;

typedef struct {
    char measurement[128];
    tsdb_tag_t tags[TSDB_MAX_TAGS_COUNT];
    size_t tags_count;
    tsdb_field_t fields[TSDB_MAX_FIELDS_COUNT];
    size_t fields_count;
    tsdb_timestamp_t timestamp;
} tsdb_point_t;

typedef struct {
    tsdb_timestamp_t start;
    tsdb_timestamp_t end;
} tsdb_time_range_t;

typedef struct {
    char measurement[128];
    tsdb_tag_t tags[TSDB_MAX_TAGS_COUNT];
    size_t tags_count;
    tsdb_time_range_t time_range;
    size_t limit;
    bool ascending;
} tsdb_query_t;

typedef struct {
    tsdb_point_t* points;
    size_t count;
    size_t capacity;
} tsdb_result_set_t;

void tsdb_result_set_destroy(tsdb_result_set_t* result);

const char* tsdb_strerror(tsdb_status_t status);

#endif
