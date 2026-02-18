#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "tsdb25.h"

#define NANO 1000000000LL

static tsdb25_timestamp_t now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (tsdb25_timestamp_t)ts.tv_sec * NANO + ts.tv_nsec;
}

int main(void) {
    printf("=== MiniMax25 Time Series Database ===\n");
    printf("Version: %s\n\n", tsdb25_version());
    
    tsdb25_config_t cfg = tsdb25_default_config();
    strcpy(cfg.data_dir, "./tsdb25_data");
    cfg.cache_size_mb = 64;
    cfg.enable_wal = true;
    cfg.enable_compression = true;
    
    printf("Opening database: %s\n", cfg.data_dir);
    
    tsdb25_t* db = tsdb25_open(cfg.data_dir, &cfg);
    if (!db) {
        fprintf(stderr, "Failed to open database\n");
        return 1;
    }
    printf("Database opened successfully\n\n");
    
    printf("Writing 1000 data points...\n");
    
    tsdb25_timestamp_t base = now_ns() - 3600LL * NANO;
    
    const char* hosts[] = {"server01", "server02", "server03", "server04", "server05"};
    const char* regions[] = {"us-east", "us-west", "eu-west", "ap-south"};
    
    for (int i = 0; i < 1000; i++) {
        tsdb25_point_t* p = tsdb25_point_new("cpu_usage", base + i * 6000000000LL);
        if (!p) continue;
        
        tsdb25_point_tag(p, "host", hosts[i % 5]);
        tsdb25_point_tag(p, "region", regions[i % 4]);
        tsdb25_point_field_f64(p, "value", 30.0 + (rand() % 700) / 10.0);
        
        tsdb25_write(db, p);
        tsdb25_point_free(p);
    }
    
    printf("Wrote 1000 points\n\n");
    
    tsdb25_stats_t st = tsdb25_stats(db);
    printf("=== Database Statistics ===\n");
    printf("  Series count:      %lu\n", (unsigned long)st.total_series);
    printf("  Measurements:     %lu\n", (unsigned long)st.total_measurements);
    printf("  Cache size:       %zu bytes\n", st.cache_size);
    printf("  Cache hit rate:   %.2f%%\n\n", tsdb25_cache_hit_rate(tsdb25_get_cache(db)) * 100);
    
    printf("=== Query: cpu_usage (limit 10) ===\n");
    
    tsdb25_query_t* q = tsdb25_query_create();
    tsdb25_query_measurement(q, "cpu_usage");
    
    tsdb25_time_range_t rng = {base, base + 1000LL * NANO};
    tsdb25_query_time_range(q, rng);
    tsdb25_query_limit(q, 10, 0);
    tsdb25_query_order(q, true);
    
    tsdb25_result_set_t rs;
    tsdb25_status_t status = tsdb25_query_select(db, q, &rs);
    
    if (status == TSDB25_OK) {
        printf("Returned %zu points:\n", rs.count);
        for (size_t i = 0; i < rs.count && i < 10; i++) {
            printf("  [%zu] ts=%lld, value=%.2f\n", 
                   i, (long long)rs.points[i].timestamp,
                   rs.points[i].fields_count > 0 ? rs.points[i].fields[0].value.float_val : 0.0);
        }
        tsdb25_result_free(&rs);
    } else {
        printf("Query failed: %s\n", tsdb25_strerror(status));
    }
    
    tsdb25_query_destroy(q);
    
    printf("\n=== Aggregation Query ===\n");
    
    q = tsdb25_query_create();
    tsdb25_query_measurement(q, "cpu_usage");
    tsdb25_query_time_range(q, rng);
    tsdb25_query_set_agg(q, TSDB25_AGG_AVG, "value");
    
    tsdb25_agg_result_set_t ars;
    status = tsdb25_query_aggregate(db, q, &ars);
    
    if (status == TSDB25_OK && ars.count > 0) {
        printf("Average: %.2f (count: %zu)\n", ars.results[0].value, ars.results[0].count);
        tsdb25_agg_result_free(&ars);
    }
    
    tsdb25_query_destroy(q);
    
    printf("\nFlushing and closing...\n");
    tsdb25_flush(db);
    tsdb25_close(db);
    
    printf("Done!\n");
    return 0;
}
