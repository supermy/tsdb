#ifndef TSDB_CACHE_H
#define TSDB_CACHE_H

#include "tsdb_types.h"

#define TSDB_CACHE_DEFAULT_SIZE (64 * 1024 * 1024)

typedef struct tsdb_cache tsdb_cache_t;

typedef struct {
    uint64_t hits;
    uint64_t misses;
    uint64_t evictions;
    uint64_t inserts;
    size_t current_size;
    size_t max_size;
    size_t entry_count;
} tsdb_cache_stats_t;

typedef void (*tsdb_cache_evict_cb)(uint64_t key, void* value, void* user_data);

tsdb_cache_t* tsdb_cache_create(size_t max_size);
void tsdb_cache_destroy(tsdb_cache_t* cache);

tsdb_status_t tsdb_cache_put(tsdb_cache_t* cache, uint64_t key, void* value, size_t size);
void* tsdb_cache_get(tsdb_cache_t* cache, uint64_t key);
tsdb_status_t tsdb_cache_remove(tsdb_cache_t* cache, uint64_t key);
bool tsdb_cache_contains(tsdb_cache_t* cache, uint64_t key);

void tsdb_cache_clear(tsdb_cache_t* cache);
tsdb_cache_stats_t tsdb_cache_get_stats(tsdb_cache_t* cache);

void tsdb_cache_set_evict_callback(tsdb_cache_t* cache, tsdb_cache_evict_cb callback, void* user_data);

double tsdb_cache_hit_rate(tsdb_cache_t* cache);

typedef struct tsdb_write_buffer tsdb_write_buffer_t;

tsdb_write_buffer_t* tsdb_write_buffer_create(size_t max_size);
void tsdb_write_buffer_destroy(tsdb_write_buffer_t* buf);

tsdb_status_t tsdb_write_buffer_add(tsdb_write_buffer_t* buf, const tsdb_point_t* point);
size_t tsdb_write_buffer_count(tsdb_write_buffer_t* buf);
bool tsdb_write_buffer_is_full(tsdb_write_buffer_t* buf);
tsdb_point_t* tsdb_write_buffer_get_points(tsdb_write_buffer_t* buf);
void tsdb_write_buffer_clear(tsdb_write_buffer_t* buf);

#endif
