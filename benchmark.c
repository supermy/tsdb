/*
 * TSDB Performance Benchmark
 * 对比 glm5, minimax25, kimi25 三个时序数据库的性能
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <sys/time.h>
#include <unistd.h>

/* 获取当前时间（微秒） */
static inline long long get_usec(void) {
    struct timeval tv;
    gettimeofday(&tv, NULL);
    return (long long)tv.tv_sec * 1000000LL + tv.tv_usec;
}

/* 获取当前时间（纳秒） */
static inline long long get_nsec(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (long long)ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

/* ==================== glm5 ==================== */
#ifdef BENCHMARK_GLM5
#include "glm5/tsdb.h"

static void benchmark_glm5(int num_points) {
    printf("\n========== glm5 Benchmark ==========\n");
    
    /* 清理旧数据 */
    system("rm -rf glm5_bench_data");
    
    long long start, end;
    
    /* 打开数据库 */
    start = get_usec();
    tsdb_t* db = tsdb_open("glm5_bench_data", NULL);
    end = get_usec();
    if (!db) {
        printf("Failed to open glm5 database\n");
        return;
    }
    printf("Open database: %.3f ms\n", (end - start) / 1000.0);
    
    /* 写入测试 */
    printf("\n--- Write Test (%d points) ---\n", num_points);
    
    tsdb_point_t** points = malloc(num_points * sizeof(tsdb_point_t*));
    for (int i = 0; i < num_points; i++) {
        points[i] = tsdb_point_create("cpu_usage", i * 1000000000LL);
        tsdb_point_add_tag(points[i], "host", "server01");
        tsdb_point_add_tag(points[i], "region", "us-east");
        tsdb_point_add_field_float(points[i], "value", 50.0 + (i % 500) / 10.0);
    }
    
    /* 单点写入 */
    start = get_usec();
    for (int i = 0; i < num_points; i++) {
        tsdb_write(db, points[i]);
    }
    end = get_usec();
    double write_time = (end - start) / 1000.0;
    printf("Single write: %.3f ms (%.1f points/sec)\n", 
           write_time, num_points / (write_time / 1000.0));
    
    /* 批量写入测试（重新创建数据库） */
    tsdb_close(db);
    system("rm -rf glm5_bench_data");
    db = tsdb_open("glm5_bench_data", NULL);
    
    start = get_usec();
    tsdb_write_batch(db, (const tsdb_point_t*)points, num_points);
    end = get_usec();
    double batch_time = (end - start) / 1000.0;
    printf("Batch write:  %.3f ms (%.1f points/sec)\n", 
           batch_time, num_points / (batch_time / 1000.0));
    
    /* 查询测试 */
    printf("\n--- Query Test ---\n");
    
    tsdb_query_builder_t* query = tsdb_query_builder_create();
    tsdb_query_builder_set_measurement(query, "cpu_usage");
    tsdb_time_range_t range = {0, num_points * 1000000000LL};
    tsdb_query_builder_set_time_range(query, range);
    tsdb_query_builder_set_limit(query, num_points, 0);
    
    tsdb_result_set_t result;
    start = get_usec();
    tsdb_query_data(db, query, &result);
    end = get_usec();
    double query_time = (end - start) / 1000.0;
    printf("Range query:  %.3f ms (returned %zu points)\n", query_time, result.count);
    tsdb_result_set_destroy(&result);
    tsdb_query_builder_destroy(query);
    
    /* 聚合查询测试 */
    printf("\n--- Aggregation Test ---\n");
    
    query = tsdb_query_builder_create();
    tsdb_query_builder_set_measurement(query, "cpu_usage");
    tsdb_query_builder_set_time_range(query, range);
    tsdb_query_builder_set_aggregation(query, TSDB_AGG_AVG, "value");
    
    tsdb_agg_result_set_t agg_result;
    start = get_usec();
    tsdb_query_agg(db, query, &agg_result);
    end = get_usec();
    printf("Avg aggregation: %.3f ms (value: %.2f)\n", 
           (end - start) / 1000.0, agg_result.results[0].value);
    tsdb_agg_result_set_destroy(&agg_result);
    tsdb_query_builder_destroy(query);
    
    /* 清理 */
    for (int i = 0; i < num_points; i++) {
        tsdb_point_destroy(points[i]);
    }
    free(points);
    
    tsdb_close(db);
    
    /* 获取数据目录大小 */
    FILE* fp = popen("du -sh glm5_bench_data 2>/dev/null | cut -f1", "r");
    if (fp) {
        char size[32];
        if (fgets(size, sizeof(size), fp)) {
            size[strcspn(size, "\n")] = 0;
            printf("\nData size: %s\n", size);
        }
        pclose(fp);
    }
}
#endif

/* ==================== minimax25 ==================== */
#ifdef BENCHMARK_MINIMAX25
#include "minimax25/tsdb25.h"

static void benchmark_minimax25(int num_points) {
    printf("\n========== minimax25 Benchmark ==========\n");
    
    /* 清理旧数据 */
    system("rm -rf minimax25_bench_data");
    
    long long start, end;
    
    /* 打开数据库 */
    start = get_usec();
    tsdb25_t* db = tsdb25_open("minimax25_bench_data", NULL);
    end = get_usec();
    if (!db) {
        printf("Failed to open minimax25 database\n");
        return;
    }
    printf("Open database: %.3f ms\n", (end - start) / 1000.0);
    
    /* 写入测试 */
    printf("\n--- Write Test (%d points) ---\n", num_points);
    
    tsdb25_point_t** points = malloc(num_points * sizeof(tsdb25_point_t*));
    for (int i = 0; i < num_points; i++) {
        points[i] = tsdb25_point_new("cpu_usage", i * 1000000000LL);
        tsdb25_point_tag(points[i], "host", "server01");
        tsdb25_point_tag(points[i], "region", "us-east");
        tsdb25_point_field_f64(points[i], "value", 50.0 + (i % 500) / 10.0);
    }
    
    /* 单点写入 */
    start = get_usec();
    for (int i = 0; i < num_points; i++) {
        tsdb25_write(db, points[i]);
    }
    end = get_usec();
    double write_time = (end - start) / 1000.0;
    printf("Single write: %.3f ms (%.1f points/sec)\n", 
           write_time, num_points / (write_time / 1000.0));
    
    /* 批量写入测试 */
    tsdb25_close(db);
    system("rm -rf minimax25_bench_data");
    db = tsdb25_open("minimax25_bench_data", NULL);
    
    start = get_usec();
    tsdb25_write_batch(db, (const tsdb25_point_t*)points, num_points);
    end = get_usec();
    double batch_time = (end - start) / 1000.0;
    printf("Batch write:  %.3f ms (%.1f points/sec)\n", 
           batch_time, num_points / (batch_time / 1000.0));
    
    /* 查询测试 */
    printf("\n--- Query Test ---\n");
    
    tsdb25_query_t* query = tsdb25_query_create();
    tsdb25_query_measurement(query, "cpu_usage");
    tsdb25_time_range_t range = {0, num_points * 1000000000LL};
    tsdb25_query_time_range(query, range);
    tsdb25_query_limit(query, num_points, 0);
    
    tsdb25_result_set_t result;
    start = get_usec();
    tsdb25_query_select(db, query, &result);
    end = get_usec();
    double query_time = (end - start) / 1000.0;
    printf("Range query:  %.3f ms (returned %zu points)\n", query_time, result.count);
    tsdb25_result_free(&result);
    tsdb25_query_destroy(query);
    
    /* 聚合查询测试 */
    printf("\n--- Aggregation Test ---\n");
    
    query = tsdb25_query_create();
    tsdb25_query_measurement(query, "cpu_usage");
    tsdb25_query_time_range(query, range);
    tsdb25_query_set_agg(query, TSDB25_AGG_AVG, "value");
    
    tsdb25_agg_result_set_t agg_result;
    start = get_usec();
    tsdb25_query_aggregate(db, query, &agg_result);
    end = get_usec();
    printf("Avg aggregation: %.3f ms (value: %.2f)\n", 
           (end - start) / 1000.0, agg_result.results[0].value);
    tsdb25_agg_result_free(&agg_result);
    tsdb25_query_destroy(query);
    
    /* 清理 */
    for (int i = 0; i < num_points; i++) {
        tsdb25_point_free(points[i]);
    }
    free(points);
    
    tsdb25_close(db);
    
    /* 获取数据目录大小 */
    FILE* fp = popen("du -sh minimax25_bench_data 2>/dev/null | cut -f1", "r");
    if (fp) {
        char size[32];
        if (fgets(size, sizeof(size), fp)) {
            size[strcspn(size, "\n")] = 0;
            printf("\nData size: %s\n", size);
        }
        pclose(fp);
    }
}
#endif

/* ==================== kimi25 ==================== */
#ifdef BENCHMARK_KIMI25
#include "kimi25/kimi_tsdb.h"

static void benchmark_kimi25(int num_points) {
    printf("\n========== kimi25 Benchmark ==========\n");
    
    /* 清理旧数据 */
    system("rm -rf kimi25_bench_data");
    
    long long start, end;
    
    /* 打开数据库 */
    start = get_usec();
    kimi_tsdb_t* db = kimi_tsdb_open("kimi25_bench_data", NULL);
    end = get_usec();
    if (!db) {
        printf("Failed to open kimi25 database\n");
        return;
    }
    printf("Open database: %.3f ms\n", (end - start) / 1000.0);
    
    /* 写入测试 */
    printf("\n--- Write Test (%d points) ---\n", num_points);
    
    kimi_point_t** points = malloc(num_points * sizeof(kimi_point_t*));
    for (int i = 0; i < num_points; i++) {
        points[i] = kimi_point_create("cpu_usage", i * 1000000000LL);
        kimi_point_add_tag(points[i], "host", "server01");
        kimi_point_add_tag(points[i], "region", "us-east");
        kimi_point_add_field_f64(points[i], "usage", 50.0 + (i % 500) / 10.0);
    }
    
    /* 单点写入 */
    start = get_usec();
    for (int i = 0; i < num_points; i++) {
        kimi_write(db, points[i]);
    }
    end = get_usec();
    double write_time = (end - start) / 1000.0;
    printf("Single write: %.3f ms (%.1f points/sec)\n", 
           write_time, num_points / (write_time / 1000.0));
    
    /* 批量写入测试 */
    kimi_tsdb_close(db);
    system("rm -rf kimi25_bench_data");
    db = kimi_tsdb_open("kimi25_bench_data", NULL);
    
    start = get_usec();
    kimi_write_batch(db, (const kimi_point_t*)points, num_points);
    end = get_usec();
    double batch_time = (end - start) / 1000.0;
    printf("Batch write:  %.3f ms (%.1f points/sec)\n", 
           batch_time, num_points / (batch_time / 1000.0));
    
    /* 查询测试 */
    printf("\n--- Query Test ---\n");
    
    kimi_query_t* query = kimi_query_new("cpu_usage");
    kimi_query_range(query, (kimi_range_t){0, num_points * 1000000000LL});
    kimi_query_limit(query, num_points);
    
    kimi_result_t result;
    start = get_usec();
    kimi_query(db, query, &result);
    end = get_usec();
    double query_time = (end - start) / 1000.0;
    printf("Range query:  %.3f ms (returned %zu points)\n", query_time, result.count);
    kimi_result_free(&result);
    kimi_query_free(query);
    
    /* 聚合查询测试 */
    printf("\n--- Aggregation Test ---\n");
    
    query = kimi_query_new("cpu_usage");
    kimi_query_range(query, (kimi_range_t){0, num_points * 1000000000LL});
    
    kimi_agg_result_t agg_result;
    start = get_usec();
    kimi_query_agg(db, query, KIMI_AGG_AVG, "usage", &agg_result);
    end = get_usec();
    printf("Avg aggregation: %.3f ms (value: %.2f)\n", 
           (end - start) / 1000.0, agg_result.value);
    kimi_query_free(query);
    
    /* 清理 */
    for (int i = 0; i < num_points; i++) {
        kimi_point_destroy(points[i]);
    }
    free(points);
    
    kimi_tsdb_close(db);
    
    /* 获取数据目录大小 */
    FILE* fp = popen("du -sh kimi25_bench_data 2>/dev/null | cut -f1", "r");
    if (fp) {
        char size[32];
        if (fgets(size, sizeof(size), fp)) {
            size[strcspn(size, "\n")] = 0;
            printf("\nData size: %s\n", size);
        }
        pclose(fp);
    }
}
#endif

/* ==================== Main ==================== */
int main(int argc, char* argv[]) {
    int num_points = 10000;  /* 默认测试数据点数 */
    
    if (argc > 1) {
        num_points = atoi(argv[1]);
        if (num_points <= 0) num_points = 10000;
    }
    
    printf("========================================\n");
    printf("    TSDB Performance Benchmark\n");
    printf("    Data points: %d\n", num_points);
    printf("========================================\n");
    
#ifdef BENCHMARK_GLM5
    benchmark_glm5(num_points);
#endif

#ifdef BENCHMARK_MINIMAX25
    benchmark_minimax25(num_points);
#endif

#ifdef BENCHMARK_KIMI25
    benchmark_kimi25(num_points);
#endif

    /* 清理所有测试数据 */
    printf("\n========================================\n");
    printf("Cleaning up benchmark data...\n");
    system("rm -rf glm5_bench_data minimax25_bench_data kimi25_bench_data");
    printf("Done!\n");
    
    return 0;
}
