/*
 * Qwen35 TSDB 示例程序
 */

#include "qtsdb.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char* argv[]) {
    const char* db_path = "./qwen35_data";
    
    printf("=== Qwen35 TSDB 示例程序 ===\n\n");
    printf("版本：%s\n\n", qtsdb_version());
    
    /* 1. 打开数据库 */
    printf("1. 打开数据库...\n");
    qtsdb_config_t config = qtsdb_default_config();
    config.enable_wal = false;  // 高性能模式
    config.batch_size = 1000;
    config.flush_interval_ms = 100;
    
    qtsdb_db_t* db = qtsdb_open(db_path, &config);
    if (!db) {
        fprintf(stderr, "无法打开数据库\n");
        return 1;
    }
    printf("   数据库路径：%s\n", db_path);
    printf("   写入缓冲：%zu MB\n", config.write_buffer_mb);
    printf("   批量大小：%zu\n", config.batch_size);
    printf("   刷新间隔：%zu ms\n\n", config.flush_interval_ms);
    
    /* 2. 写入数据 */
    printf("2. 写入测试数据...\n");
    
    /* 单点写入 */
    qtsdb_point_t* point = qtsdb_point_create("cpu_usage", qtsdb_now());
    qtsdb_point_add_tag(point, "host", "server01");
    qtsdb_point_add_tag(point, "region", "us-east");
    qtsdb_point_add_field_float(point, "value", 50.5);
    
    qtsdb_status_t status = qtsdb_write(db, point);
    if (status != QTSDB_OK) {
        fprintf(stderr, "写入失败：%s\n", qtsdb_strerror(status));
    } else {
        printf("   单点写入：成功\n");
    }
    qtsdb_point_destroy(point);
    
    /* 批量写入 */
    const size_t batch_count = 1000;
    qtsdb_point_t* batch[batch_count];
    
    for (size_t i = 0; i < batch_count; i++) {
        batch[i] = qtsdb_point_create("memory_usage", qtsdb_now() + i * 1000000000LL);
        qtsdb_point_add_tag(batch[i], "host", "server01");
        qtsdb_point_add_field_float(batch[i], "value", 60.0 + (i % 100) / 10.0);
    }
    
    status = qtsdb_write_batch(db, batch, batch_count);
    if (status != QTSDB_OK) {
        fprintf(stderr, "批量写入失败：%s\n", qtsdb_strerror(status));
    } else {
        printf("   批量写入：%zu 点\n", batch_count);
    }
    
    for (size_t i = 0; i < batch_count; i++) {
        qtsdb_point_destroy(batch[i]);
    }
    
    /* 刷新数据 */
    qtsdb_flush(db);
    printf("   数据已刷新到磁盘\n\n");
    
    /* 3. 查询数据 */
    printf("3. 查询数据...\n");
    
    qtsdb_time_range_t range = {0, qtsdb_now() + 1000000000LL};
    qtsdb_result_set_t result;
    
    status = qtsdb_query(db, "cpu_usage", &range, 100, &result);
    if (status == QTSDB_OK) {
        printf("   CPU 使用率查询：返回 %zu 条记录\n", result.count);
        for (size_t i = 0; i < result.count && i < 3; i++) {
            printf("     [%zu] 时间：%lld, 值：%.2f\n", 
                   i, (long long)result.points[i].timestamp,
                   result.points[i].fields[0].value.float_val);
        }
    }
    
    status = qtsdb_query(db, "memory_usage", &range, 100, &result);
    if (status == QTSDB_OK) {
        printf("   内存使用率查询：返回 %zu 条记录\n", result.count);
    }
    qtsdb_result_set_destroy(&result);
    printf("\n");
    
    /* 4. 聚合查询 */
    printf("4. 聚合查询...\n");
    
    qtsdb_agg_result_set_t agg_result;
    
    status = qtsdb_query_agg(db, "memory_usage", &range, QTSDB_AGG_COUNT, NULL, &agg_result);
    if (status == QTSDB_OK) {
        printf("   COUNT: %zu\n", (size_t)agg_result.results[0].value);
    }
    
    status = qtsdb_query_agg(db, "memory_usage", &range, QTSDB_AGG_SUM, NULL, &agg_result);
    if (status == QTSDB_OK) {
        printf("   SUM: %.2f\n", agg_result.results[0].value);
    }
    
    status = qtsdb_query_agg(db, "memory_usage", &range, QTSDB_AGG_AVG, NULL, &agg_result);
    if (status == QTSDB_OK) {
        printf("   AVG: %.2f\n", agg_result.results[0].value);
    }
    
    status = qtsdb_query_agg(db, "memory_usage", &range, QTSDB_AGG_MIN, NULL, &agg_result);
    if (status == QTSDB_OK) {
        printf("   MIN: %.2f\n", agg_result.results[0].value);
    }
    
    status = qtsdb_query_agg(db, "memory_usage", &range, QTSDB_AGG_MAX, NULL, &agg_result);
    if (status == QTSDB_OK) {
        printf("   MAX: %.2f\n", agg_result.results[0].value);
    }
    
    qtsdb_agg_result_set_destroy(&agg_result);
    printf("\n");
    
    /* 5. 统计信息 */
    printf("5. 数据库统计信息...\n");
    qtsdb_stats_t stats = qtsdb_stats(db);
    printf("   总数据点：%lu\n", (unsigned long)stats.total_points);
    printf("   总序列数：%lu\n", (unsigned long)stats.total_series);
    printf("   存储大小：%zu 字节\n\n", stats.storage_size);
    
    /* 6. 关闭数据库 */
    printf("6. 关闭数据库...\n");
    qtsdb_close(db);
    printf("   数据库已关闭\n\n");
    
    printf("=== 示例程序结束 ===\n");
    
    return 0;
}
