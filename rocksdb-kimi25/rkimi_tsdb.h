#ifndef RKIMI_TSDB_H
#define RKIMI_TSDB_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

#define RKIMI_VERSION "1.0.0"

typedef enum {
    RKIMI_OK = 0,
    RKIMI_ERR_INVALID = -1,
    RKIMI_ERR_MEMORY = -2,
    RKIMI_ERR_IO = -3,
} rkimi_error_t;

typedef int64_t rkimi_ts_t;
typedef double rkimi_val_t;

typedef enum {
    RKIMI_AGG_SUM = 0,
    RKIMI_AGG_AVG = 1,
    RKIMI_AGG_MIN = 2,
    RKIMI_AGG_MAX = 3,
    RKIMI_AGG_COUNT = 4,
} rkimi_agg_t;

typedef struct {
    char key[64];
    char value[256];
} rkimi_tag_t;

typedef struct {
    char measurement[128];
    rkimi_tag_t tags[32];
    size_t tag_count;
    double value;
    rkimi_ts_t timestamp;
} rkimi_point_t;

typedef struct {
    rkimi_ts_t start;
    rkimi_ts_t end;
} rkimi_range_t;

typedef struct {
    rkimi_point_t* points;
    size_t count;
    size_t capacity;
} rkimi_result_t;

typedef struct {
    rkimi_val_t value;
    size_t count;
} rkimi_agg_result_t;

typedef struct rkimi_db rkimi_db_t;

rkimi_db_t* rkimi_open(const char* path);
void rkimi_close(rkimi_db_t* db);

rkimi_error_t rkimi_write(rkimi_db_t* db, const rkimi_point_t* point);
rkimi_error_t rkimi_write_batch(rkimi_db_t* db, const rkimi_point_t* points, size_t count);

rkimi_error_t rkimi_query(rkimi_db_t* db, const char* measurement, 
                          const rkimi_range_t* range, size_t limit, rkimi_result_t* result);

rkimi_error_t rkimi_agg(rkimi_db_t* db, const char* measurement,
                        const rkimi_range_t* range, rkimi_agg_t agg, rkimi_agg_result_t* result);

rkimi_point_t* rkimi_point_new(const char* measurement, rkimi_ts_t ts);
void rkimi_point_free(rkimi_point_t* p);
rkimi_point_t* rkimi_tag(rkimi_point_t* p, const char* k, const char* v);
rkimi_point_t* rkimi_val(rkimi_point_t* p, double v);

void rkimi_result_free(rkimi_result_t* r);

rkimi_ts_t rkimi_now(void);
const char* rkimi_version(void);

#ifdef __cplusplus
}
#endif

#endif
