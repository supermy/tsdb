#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "rkimi_tsdb.h"

#define NANO 1000000000LL

int main(void) {
    printf("=== RocksDB-Kimi25 Time Series Database ===\n");
    printf("Version: %s\n\n", rkimi_version());
    
    system("rm -rf rkimi_data");
    
    printf("Opening database...\n");
    rkimi_db_t* db = rkimi_open("rkimi_data");
    if (!db) {
        fprintf(stderr, "Failed to open database\n");
        return 1;
    }
    printf("Database opened successfully\n\n");
    
    printf("Writing 50000 data points...\n");
    
    rkimi_ts_t base = rkimi_now() - 3600LL * NANO;
    const char* hosts[] = {"web01", "web02", "web03", "db01", "cache01"};
    const char* regions[] = {"us-east", "us-west", "eu-west", "ap-south", "cn-north"};
    
    for (int i = 0; i < 50000; i++) {
        rkimi_point_t* p = rkimi_point_new("cpu_usage", base + i * 200000000LL);
        if (!p) continue;
        
        rkimi_tag(p, "host", hosts[i % 5]);
        rkimi_tag(p, "region", regions[i % 5]);
        rkimi_val(p, 10.0 + (rand() % 900) / 10.0);
        
        rkimi_write(db, p);
        rkimi_point_free(p);
    }
    
    printf("Wrote 50000 points\n\n");
    
    printf("=== Query Test ===\n");
    
    rkimi_range_t range = {base, base + 10000LL * NANO};
    rkimi_result_t result;
    
    rkimi_error_t err = rkimi_query(db, "cpu_usage", &range, 10, &result);
    
    if (err == RKIMI_OK) {
        printf("Query returned %zu points:\n", result.count);
        for (size_t i = 0; i < result.count && i < 10; i++) {
            printf("  [%zu] ts=%lld, usage=%.2f%%\n",
                   i, (long long)result.points[i].timestamp,
                   result.points[i].value);
        }
        rkimi_result_free(&result);
    }
    
    printf("\n=== Aggregation Test ===\n");
    
    rkimi_agg_result_t agg;
    err = rkimi_agg(db, "cpu_usage", &range, RKIMI_AGG_AVG, &agg);
    
    if (err == RKIMI_OK) {
        printf("Average: %.2f%% (count: %zu)\n", agg.value, agg.count);
    }
    
    err = rkimi_agg(db, "cpu_usage", &range, RKIMI_AGG_MAX, &agg);
    if (err == RKIMI_OK) {
        printf("Max: %.2f%%\n", agg.value);
    }
    
    err = rkimi_agg(db, "cpu_usage", &range, RKIMI_AGG_MIN, &agg);
    if (err == RKIMI_OK) {
        printf("Min: %.2f%%\n", agg.value);
    }
    
    printf("\nClosing database...\n");
    rkimi_close(db);
    
    printf("Done!\n");
    return 0;
}
