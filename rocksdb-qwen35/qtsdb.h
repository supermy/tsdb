/*
 * RocksDB 高性能时序数据库 - Qwen35 版本
 * 
 * 整合优化策略：
 * 1. 二进制紧凑 Key 设计 (kimi25)
 * 2. 时间块聚合存储 (minimax25/glm5)
 * 3. ColumnFamily 冷热分离 (glm5)
 * 4. 自动批量写入 (kimi25/minimax25)
 * 5. Gorilla 压缩算法 (glm5)
 * 
 * @author Qwen35
 * @version 1.0.0
 */

#ifndef QTSDB_H
#define QTSDB_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include <pthread.h>

#ifdef __cplusplus
extern "C" {
#endif

#define QTSDB_VERSION "1.0.0"
#define QTSDB_MAX_MEASUREMENT_LEN 128
#define QTSDB_MAX_TAG_KEY_LEN 64
#define QTSDB_MAX_TAG_VALUE_LEN 256
#define QTSDB_MAX_FIELD_NAME_LEN 64
#define QTSDB_MAX_TAGS_COUNT 32
#define QTSDB_MAX_FIELDS_COUNT 64
#define QTSDB_MAX_PATH_LEN 512

/* 时间戳类型（纳秒精度） */
typedef int64_t qtsdb_timestamp_t;

/* 数值类型 */
typedef double qtsdb_value_t;

/* 错误码 */
typedef enum {
    QTSDB_OK = 0,
    QTSDB_ERR_INVALID_PARAM,
    QTSDB_ERR_NO_MEMORY,
    QTSDB_ERR_NOT_FOUND,
    QTSDB_ERR_IO,
    QTSDB_ERR_CORRUPTED,
    QTSDB_ERR_EXISTS,
    QTSDB_ERR_TIMEOUT,
    QTSDB_ERR_FULL,
    QTSDB_ERR_INVALID_STATE
} qtsdb_status_t;

/* 字段类型 */
typedef enum {
    QTSDB_FIELD_FLOAT,
    QTSDB_FIELD_INTEGER,
    QTSDB_FIELD_BOOLEAN,
    QTSDB_FIELD_STRING
} qtsdb_field_type_t;

/* 聚合类型 */
typedef enum {
    QTSDB_AGG_COUNT,
    QTSDB_AGG_SUM,
    QTSDB_AGG_AVG,
    QTSDB_AGG_MIN,
    QTSDB_AGG_MAX,
    QTSDB_AGG_FIRST,
    QTSDB_AGG_LAST,
    QTSDB_AGG_STDDEV
} qtsdb_agg_type_t;

/* Tag 结构 */
typedef struct {
    char key[QTSDB_MAX_TAG_KEY_LEN];
    char value[QTSDB_MAX_TAG_VALUE_LEN];
} qtsdb_tag_t;

/* Field 结构 */
typedef struct {
    char name[QTSDB_MAX_FIELD_NAME_LEN];
    qtsdb_field_type_t type;
    union {
        double float_val;
        int64_t int_val;
        bool bool_val;
        char str_val[256];
    } value;
} qtsdb_field_t;

/* 数据点 */
typedef struct {
    char measurement[QTSDB_MAX_MEASUREMENT_LEN];
    qtsdb_timestamp_t timestamp;
    qtsdb_tag_t tags[QTSDB_MAX_TAGS_COUNT];
    size_t tags_count;
    qtsdb_field_t fields[QTSDB_MAX_FIELDS_COUNT];
    size_t fields_count;
} qtsdb_point_t;

/* 时间范围 */
typedef struct {
    qtsdb_timestamp_t start;
    qtsdb_timestamp_t end;
} qtsdb_time_range_t;

/* 查询结果 */
typedef struct {
    qtsdb_point_t* points;
    size_t count;
    size_t capacity;
} qtsdb_result_set_t;

/* 聚合结果 */
typedef struct {
    qtsdb_value_t value;
    qtsdb_timestamp_t timestamp;
    size_t count;
} qtsdb_agg_result_t;

typedef struct {
    qtsdb_agg_result_t* results;
    size_t count;
    size_t capacity;
} qtsdb_agg_result_set_t;

/* 数据库配置 */
typedef struct {
    size_t write_buffer_mb;
    size_t cache_size_mb;
    size_t max_open_files;
    bool enable_compression;
    int compression_level;
    bool enable_wal;
    size_t batch_size;
    uint64_t flush_interval_ms;
    bool enable_cf_split;
    int hot_data_days;
} qtsdb_config_t;

/* 数据库句柄 */
typedef struct qtsdb_db qtsdb_db_t;

/* API 函数声明 */

/* 版本信息 */
const char* qtsdb_version(void);

/* 错误信息 */
const char* qtsdb_strerror(qtsdb_status_t status);

/* 默认配置 */
qtsdb_config_t qtsdb_default_config(void);

/* 数据库操作 */
qtsdb_db_t* qtsdb_open(const char* path, const qtsdb_config_t* config);
qtsdb_status_t qtsdb_close(qtsdb_db_t* db);

/* 写入操作 */
qtsdb_status_t qtsdb_write(qtsdb_db_t* db, const qtsdb_point_t* point);
qtsdb_status_t qtsdb_write_batch(qtsdb_db_t* db, const qtsdb_point_t* points, size_t count);
qtsdb_status_t qtsdb_flush(qtsdb_db_t* db);

/* 查询操作 */
qtsdb_status_t qtsdb_query(qtsdb_db_t* db, const char* measurement,
                           qtsdb_time_range_t* range, size_t limit,
                           qtsdb_result_set_t* result);

qtsdb_status_t qtsdb_query_agg(qtsdb_db_t* db, const char* measurement,
                               qtsdb_time_range_t* range, qtsdb_agg_type_t agg,
                               const char* field, qtsdb_agg_result_set_t* result);

/* 管理操作 */
qtsdb_status_t qtsdb_delete_measurement(qtsdb_db_t* db, const char* measurement);
qtsdb_status_t qtsdb_delete_range(qtsdb_db_t* db, const char* measurement,
                                  qtsdb_time_range_t* range);
qtsdb_status_t qtsdb_compact(qtsdb_db_t* db);

/* 统计信息 */
typedef struct {
    uint64_t total_points;
    uint64_t total_series;
    uint64_t total_measurements;
    size_t storage_size;
} qtsdb_stats_t;

qtsdb_stats_t qtsdb_stats(qtsdb_db_t* db);

/* 辅助函数 */

/* 创建数据点 */
qtsdb_point_t* qtsdb_point_create(const char* measurement, qtsdb_timestamp_t timestamp);
void qtsdb_point_destroy(qtsdb_point_t* point);

/* 添加 Tag */
qtsdb_point_t* qtsdb_point_add_tag(qtsdb_point_t* point, const char* key, const char* value);

/* 添加 Field */
qtsdb_point_t* qtsdb_point_add_field_float(qtsdb_point_t* point, const char* name, double value);
qtsdb_point_t* qtsdb_point_add_field_int(qtsdb_point_t* point, const char* name, int64_t value);
qtsdb_point_t* qtsdb_point_add_field_bool(qtsdb_point_t* point, const char* name, bool value);
qtsdb_point_t* qtsdb_point_add_field_string(qtsdb_point_t* point, const char* name, const char* value);

/* 结果管理 */
void qtsdb_result_set_destroy(qtsdb_result_set_t* result);
void qtsdb_agg_result_set_destroy(qtsdb_agg_result_set_t* result);

/* 时间工具 */
qtsdb_timestamp_t qtsdb_now(void);
qtsdb_timestamp_t qtsdb_now_ms(void);

#ifdef __cplusplus
}
#endif

#endif /* QTSDB_H */
