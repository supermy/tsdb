#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "kimi_tsdb.h"

#define NANO 1000000000LL

int main(void) {
    printf("=== Kimi25 Time Series Database ===\n");
    printf("Version: %s\n\n", kimi_tsdb_version());
    
    /* Open database */
    kimi_config_t cfg = {
        .cache_size_mb = 64,
        .max_series = 100000,
        .enable_wal = true,
        .sync_write = false,
        .block_size = 8192
    };
    
    printf("Opening database...\n");
    kimi_tsdb_t* db = kimi_tsdb_open("./kimi_data", &cfg);
    if (!db) {
        fprintf(stderr, "Failed to open database\n");
        return 1;
    }
    printf("Database opened successfully\n\n");
    
    /* Write data points */
    printf("Writing 500 data points...\n");
    
    kimi_timestamp_t base = kimi_now() - 3600LL * NANO;
    const char* hosts[] = {"web01", "web02", "web03", "db01", "db02"};
    const char* regions[] = {"us-east", "us-west", "eu-central"};
    
    for (int i = 0; i < 500; i++) {
        kimi_point_t* p = kimi_point_create("cpu_usage", base + i * 7200000000LL);
        if (!p) continue;
        
        kimi_point_add_tag(p, "host", hosts[i % 5]);
        kimi_point_add_tag(p, "region", regions[i % 3]);
        kimi_point_add_field_f64(p, "usage", 20.0 + (rand() % 600) / 10.0);
        
        kimi_error_t err = kimi_write(db, p);
        if (err != KIMI_OK) {
            printf("Write error: %s\n", kimi_strerror(err));
        }
        
        kimi_point_destroy(p);
    }
    printf("Wrote 500 points\n\n");
    
    /* Query data */
    printf("=== Query Test ===\n");
    
    kimi_query_t* q = kimi_query_new("cpu_usage");
    kimi_query_range(q, (kimi_range_t){base, base + 1000LL * NANO});
    kimi_query_limit(q, 10);
    
    kimi_result_t result;
    kimi_error_t err = kimi_query(db, q, &result);
    
    if (err == KIMI_OK) {
        printf("Query returned %zu points:\n", result.count);
        for (size_t i = 0; i < result.count && i < 10; i++) {
            printf("  [%zu] ts=%lld, usage=%.2f%%\n",
                   i, (long long)result.points[i].timestamp,
                   result.points[i].fields[0].value.f64);
        }
        kimi_result_free(&result);
    } else {
        printf("Query failed: %s\n", kimi_strerror(err));
    }
    
    kimi_query_free(q);
    
    /* Aggregation test */
    printf("\n=== Aggregation Test ===\n");
    
    q = kimi_query_new("cpu_usage");
    kimi_query_range(q, (kimi_range_t){base, base + 3600LL * NANO});
    
    kimi_agg_result_t agg;
    err = kimi_query_agg(db, q, KIMI_AGG_AVG, "usage", &agg);
    
    if (err == KIMI_OK) {
        printf("Average CPU usage: %.2f%% (count: %zu)\n", agg.value, agg.count);
    }
    
    err = kimi_query_agg(db, q, KIMI_AGG_MAX, "usage", &agg);
    if (err == KIMI_OK) {
        printf("Max CPU usage: %.2f%%\n", agg.value);
    }
    
    kimi_query_free(q);
    
    /* Close database */
    printf("\nClosing database...\n");
    kimi_tsdb_close(db);
    printf("Done!\n");
    
    return 0;
}
