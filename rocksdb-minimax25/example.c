#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "rocksdb_tsdb.h"

#define NANO 1000000000LL

int main(void) {
    printf("=== RocksDB Time Series Database ===\n");
    printf("Version: %s\n\n", rocksdb_tsdb_version());
    
    system("rm -rf rocksdb_data");
    
    printf("Opening database...\n");
    rocksdb_tsdb_t* db = rocksdb_tsdb_open("rocksdb_data", NULL);
    if (!db) {
        fprintf(stderr, "Failed to open database\n");
        return 1;
    }
    printf("Database opened successfully\n\n");
    
    printf("Writing 20000 data points...\n");
    
    ts_timestamp_t base = ts_now() - 3600LL * NANO;
    const char* hosts[] = {"web01", "web02", "web03", "db01", "api01"};
    const char* regions[] = {"us-east", "us-west", "eu-central", "ap-northeast"};
    
    for (int i = 0; i < 20000; i++) {
        point_t* p = point_create("cpu_usage", base + i * 1000000000LL);
        if (!p) continue;
        
        point_add_tag(p, "host", hosts[i % 5]);
        point_add_tag(p, "region", regions[i % 4]);
        point_add_field_f64(p, "usage", 20.0 + (rand() % 800) / 10.0);
        
        rocksdb_tsdb_write(db, p);
        point_destroy(p);
    }
    
    printf("Wrote 20000 points\n\n");
    
    printf("=== Query Test ===\n");
    
    range_t range = {base, base + 10000LL * NANO};
    result_t result;
    
    rocksdb_error_t err = rocksdb_tsdb_query(db, "cpu_usage", &range, 10, &result);
    
    if (err == ROCKSDB_OK) {
        printf("Query returned %zu points:\n", result.count);
        for (size_t i = 0; i < result.count && i < 10; i++) {
            printf("  [%zu] ts=%lld, usage=%.2f%%\n",
                   i, (long long)result.points[i].timestamp,
                   result.points[i].fields[0].value.f64);
        }
        result_destroy(&result);
    }
    
    printf("\n=== Aggregation Test ===\n");
    
    agg_result_t agg;
    err = rocksdb_tsdb_query_agg(db, "cpu_usage", &range, AGG_AVG, "usage", &agg);
    
    if (err == ROCKSDB_OK) {
        printf("Average: %.2f%% (count: %zu)\n", agg.value, agg.count);
    }
    
    err = rocksdb_tsdb_query_agg(db, "cpu_usage", &range, AGG_MAX, "usage", &agg);
    if (err == ROCKSDB_OK) {
        printf("Max: %.2f%%\n", agg.value);
    }
    
    err = rocksdb_tsdb_query_agg(db, "cpu_usage", &range, AGG_MIN, "usage", &agg);
    if (err == ROCKSDB_OK) {
        printf("Min: %.2f%%\n", agg.value);
    }
    
    printf("\n=== Statistics ===\n");
    printf("Total points: %lu\n", (unsigned long)rocksdb_get_total_points(db));
    
    printf("\nClosing database...\n");
    rocksdb_tsdb_close(db);
    
    printf("Done!\n");
    return 0;
}
