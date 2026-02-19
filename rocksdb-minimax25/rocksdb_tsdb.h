#ifndef ROCKSDB_TSDB25_H
#define ROCKSDB_TSDB25_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

#define ROCKSDB_TSDB_VERSION "2.5.0-rc1"

typedef enum {
    ROCKSDB_OK = 0,
    ROCKSDB_ERR_INVALID_PARAM = -1,
    ROCKSDB_ERR_NO_MEMORY = -2,
    ROCKSDB_ERR_NOT_FOUND = -3,
    ROCKSDB_ERR_IO = -4,
    ROCKSDB_ERR_CORRUPTED = -5,
    ROCKSDB_ERR_EXISTS = -6,
    ROCKSDB_ERR_TIMEOUT = -7,
    ROCKSDB_ERR_FULL = -8,
} rocksdb_error_t;

typedef int64_t ts_timestamp_t;
typedef double ts_value_t;

typedef enum {
    FIELD_FLOAT = 0,
    FIELD_INT = 1,
    FIELD_BOOL = 2,
    FIELD_STRING = 3
} field_type_t;

typedef enum {
    AGG_NONE = 0,
    AGG_SUM = 1,
    AGG_AVG = 2,
    AGG_MIN = 3,
    AGG_MAX = 4,
    AGG_COUNT = 5,
    AGG_FIRST = 6,
    AGG_LAST = 7,
    AGG_STDDEV = 8,
    AGG_RATE = 9,
} agg_type_t;

typedef struct {
    char key[64];
    char value[256];
} tag_t;

typedef struct {
    char name[128];
    field_type_t type;
    union {
        double f64;
        int64_t i64;
        bool boolean;
        char str[512];
    } value;
} field_t;

typedef struct {
    char measurement[128];
    tag_t tags[32];
    size_t tag_count;
    field_t fields[64];
    size_t field_count;
    ts_timestamp_t timestamp;
} point_t;

typedef struct {
    ts_timestamp_t start;
    ts_timestamp_t end;
} range_t;

typedef struct {
    point_t* points;
    size_t count;
    size_t capacity;
} result_t;

typedef struct {
    ts_value_t value;
    size_t count;
    ts_timestamp_t timestamp;
} agg_result_t;

typedef struct tsdb rocksdb_tsdb_t;

typedef struct {
    size_t cache_size_mb;
    size_t max_open_files;
    size_t write_buffer_mb;
    size_t max_bytes_for_level_base;
    int num_levels;
    bool compression;
    int compression_level;
    bool direct_io;
    size_t block_size;
    bool bloom_filter;
    size_t bloom_bits_per_key;
} rocksdb_config_t;

rocksdb_tsdb_t* rocksdb_tsdb_open(const char* path, const rocksdb_config_t* config);
void rocksdb_tsdb_close(rocksdb_tsdb_t* db);

const char* rocksdb_tsdb_version(void);
const char* rocksdb_strerror(rocksdb_error_t err);

rocksdb_error_t rocksdb_tsdb_write(rocksdb_tsdb_t* db, const point_t* point);
rocksdb_error_t rocksdb_tsdb_write_batch(rocksdb_tsdb_t* db, const point_t* points, size_t count);

rocksdb_error_t rocksdb_tsdb_query(rocksdb_tsdb_t* db, const char* measurement, 
                              const range_t* range, size_t limit, result_t* result);

rocksdb_error_t rocksdb_tsdb_query_agg(rocksdb_tsdb_t* db, const char* measurement,
                                   const range_t* range, agg_type_t agg, 
                                   const char* field, agg_result_t* result);

rocksdb_error_t rocksdb_tsdb_delete_measurement(rocksdb_tsdb_t* db, const char* measurement);
rocksdb_error_t rocksdb_tsdb_delete_range(rocksdb_tsdb_t* db, const char* measurement, 
                                     const range_t* range);

rocksdb_error_t rocksdb_tsdb_flush(rocksdb_tsdb_t* db);
rocksdb_error_t rocksdb_tsdb_compact(rocksdb_tsdb_t* db);

uint64_t rocksdb_get_total_points(rocksdb_tsdb_t* db);
uint64_t rocksdb_get_total_series(rocksdb_tsdb_t* db);
size_t rocksdb_get_storage_size(rocksdb_tsdb_t* db);

point_t* point_create(const char* measurement, ts_timestamp_t ts);
void point_destroy(point_t* point);
point_t* point_add_tag(point_t* p, const char* key, const char* value);
point_t* point_add_field_f64(point_t* p, const char* name, double value);
point_t* point_add_field_i64(point_t* p, const char* name, int64_t value);
point_t* point_add_field_bool(point_t* p, const char* name, bool value);
point_t* point_add_field_str(point_t* p, const char* name, const char* value);

void result_destroy(result_t* result);

rocksdb_config_t rocksdb_default_config(void);

ts_timestamp_t ts_now(void);

#ifdef __cplusplus
}
#endif

#endif
