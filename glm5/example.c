#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "tsdb.h"

#define NANOSECONDS_PER_SECOND 1000000000LL

static tsdb_timestamp_t current_time_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (tsdb_timestamp_t)ts.tv_sec * NANOSECONDS_PER_SECOND + ts.tv_nsec;
}

int main(void) {
    printf("TSDB Time Series Database Example\n");
    printf("Version: %s\n\n", tsdb_version());
    
    tsdb_config_t config = tsdb_default_config();
    strcpy(config.data_dir, "./tsdb_data");
    config.cache_size_mb = 32;
    config.enable_compression = true;
    
    printf("Opening database at: %s\n", config.data_dir);
    
    tsdb_t* db = tsdb_open(config.data_dir, &config);
    if (!db) {
        fprintf(stderr, "Failed to open database\n");
        return 1;
    }
    
    printf("Database opened successfully\n\n");
    
    printf("Writing data points...\n");
    
    tsdb_timestamp_t base_time = current_time_ns() - 3600LL * NANOSECONDS_PER_SECOND;
    
    for (int i = 0; i < 100; i++) {
        tsdb_point_t* point = tsdb_point_create("cpu_usage", base_time + i * 60000000000LL);
        if (!point) {
            fprintf(stderr, "Failed to create point\n");
            continue;
        }
        
        tsdb_point_add_tag(point, "host", "server01");
        tsdb_point_add_tag(point, "region", "us-east-1");
        tsdb_point_add_field_float(point, "value", 50.0 + (rand() % 500) / 10.0);
        
        tsdb_status_t status = tsdb_write(db, point);
        if (status != TSDB_OK) {
            fprintf(stderr, "Failed to write point: %s\n", tsdb_strerror(status));
        }
        
        tsdb_point_destroy(point);
    }
    
    printf("Wrote 100 data points\n\n");
    
    tsdb_stats_t stats = tsdb_get_stats(db);
    printf("Database Statistics:\n");
    printf("  Total series: %lu\n", (unsigned long)stats.total_series);
    printf("  Total measurements: %lu\n", (unsigned long)stats.total_measurements);
    printf("  Cache size: %zu bytes\n", stats.cache_size);
    printf("  Cache hit rate: %.2f%%\n\n", tsdb_get_cache_hit_rate(db) * 100);
    
    printf("Querying data...\n");
    
    tsdb_query_builder_t* query = tsdb_query_builder_create();
    tsdb_query_builder_set_measurement(query, "cpu_usage");
    
    tsdb_time_range_t range = {
        .start = base_time,
        .end = base_time + 100 * 60000000000LL
    };
    tsdb_query_builder_set_time_range(query, range);
    tsdb_query_builder_set_limit(query, 10, 0);
    tsdb_query_builder_set_order(query, true);
    
    tsdb_result_set_t result;
    tsdb_status_t status = tsdb_query_data(db, query, &result);
    
    if (status == TSDB_OK) {
        printf("Query returned %zu points:\n", result.count);
        for (size_t i = 0; i < result.count && i < 10; i++) {
            printf("  [%zu] timestamp=%lld, value=%.2f\n", 
                   i, 
                   (long long)result.points[i].timestamp,
                   result.points[i].fields_count > 0 ? result.points[i].fields[0].value.float_val : 0.0);
        }
        tsdb_result_set_destroy(&result);
    } else {
        fprintf(stderr, "Query failed: %s\n", tsdb_strerror(status));
    }
    
    tsdb_query_builder_destroy(query);
    
    printf("\nAggregation query (average)...\n");
    
    query = tsdb_query_builder_create();
    tsdb_query_builder_set_measurement(query, "cpu_usage");
    tsdb_query_builder_set_time_range(query, range);
    tsdb_query_builder_set_aggregation(query, TSDB_AGG_AVG, "value");
    
    tsdb_agg_result_set_t agg_result;
    status = tsdb_query_agg(db, query, &agg_result);
    
    if (status == TSDB_OK && agg_result.count > 0) {
        printf("Average CPU usage: %.2f (from %zu points)\n", 
               agg_result.results[0].value,
               agg_result.results[0].count);
        tsdb_agg_result_set_destroy(&agg_result);
    } else {
        fprintf(stderr, "Aggregation query failed: %s\n", tsdb_strerror(status));
    }
    
    tsdb_query_builder_destroy(query);
    
    printf("\nFlushing data to disk...\n");
    tsdb_flush(db);
    
    printf("Closing database...\n");
    tsdb_close(db);
    
    printf("\nDone!\n");
    return 0;
}
