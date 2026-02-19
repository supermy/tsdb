#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "rtsdb.h"

#define NANO 1000000000LL

static rtsdb_timestamp_t now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (rtsdb_timestamp_t)ts.tv_sec * NANO + ts.tv_nsec;
}

int main(void) {
    printf("=== RocksDB-GLM5 Time Series Database ===\n");
    printf("Version: %s\n\n", rtsdb_version());
    
    system("rm -rf rtsdb_data");
    
    printf("Opening database...\n");
    rtsdb_t* db = rtsdb_open("rtsdb_data", NULL);
    if (!db) {
        fprintf(stderr, "Failed to open database\n");
        return 1;
    }
    printf("Database opened successfully\n\n");
    
    printf("Writing 30000 data points...\n");
    
    rtsdb_timestamp_t base = now_ns() - 7200LL * NANO;
    const char* hosts[] = {"web01", "web02", "web03", "db01", "cache01"};
    const char* regions[] = {"us-east", "us-west", "eu-west", "ap-south"};
    
    for (int i = 0; i < 30000; i++) {
        rtsdb_point_t* p = rtsdb_point_create("cpu_usage", base + i * 500000000LL);
        if (!p) continue;
        
        rtsdb_point_add_tag(p, "host", hosts[i % 5]);
        rtsdb_point_add_tag(p, "region", regions[i % 4]);
        rtsdb_point_add_field_float(p, "usage", 15.0 + (rand() % 850) / 10.0);
        
        rtsdb_write(db, p);
        rtsdb_point_destroy(p);
    }
    
    printf("Wrote 30000 points\n\n");
    
    rtsdb_stats_t st = rtsdb_stats(db);
    printf("=== Database Statistics ===\n");
    printf("  Series count:      %lu\n", (unsigned long)st.total_series);
    printf("  Measurements:     %lu\n", (unsigned long)st.total_measurements);
    printf("  Total points:     %lu\n\n", (unsigned long)st.total_points);
    
    printf("=== Query Test ===\n");
    
    rtsdb_time_range_t range = {base, base + 5000LL * NANO};
    rtsdb_result_set_t result;
    
    rtsdb_status_t err = rtsdb_query(db, "cpu_usage", &range, 10, &result);
    
    if (err == RTSDB_OK) {
        printf("Query returned %zu points:\n", result.count);
        for (size_t i = 0; i < result.count && i < 10; i++) {
            printf("  [%zu] ts=%lld, usage=%.2f%%\n",
                   i, (long long)result.points[i].timestamp,
                   result.points[i].fields[0].value.float_val);
        }
        rtsdb_result_set_destroy(&result);
    }
    
    printf("\n=== Aggregation Test ===\n");
    
    rtsdb_agg_result_set_t agg_result;
    err = rtsdb_query_agg(db, "cpu_usage", &range, RTSDB_AGG_AVG, "usage", &agg_result);
    
    if (err == RTSDB_OK && agg_result.count > 0) {
        printf("Average: %.2f%% (count: %zu)\n", agg_result.results[0].value, agg_result.results[0].count);
        rtsdb_agg_result_set_destroy(&agg_result);
    }
    
    err = rtsdb_query_agg(db, "cpu_usage", &range, RTSDB_AGG_MAX, "usage", &agg_result);
    if (err == RTSDB_OK && agg_result.count > 0) {
        printf("Max: %.2f%%\n", agg_result.results[0].value);
        rtsdb_agg_result_set_destroy(&agg_result);
    }
    
    err = rtsdb_query_agg(db, "cpu_usage", &range, RTSDB_AGG_MIN, "usage", &agg_result);
    if (err == RTSDB_OK && agg_result.count > 0) {
        printf("Min: %.2f%%\n", agg_result.results[0].value);
        rtsdb_agg_result_set_destroy(&agg_result);
    }
    
    printf("\nCompacting database...\n");
    rtsdb_compact(db);
    
    printf("Closing database...\n");
    rtsdb_close(db);
    
    printf("Done!\n");
    return 0;
}
