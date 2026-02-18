#ifndef TSDB25_CACHE_H
#define TSDB25_CACHE_H

#include "tsdb25_types.h"

#define TSDB25_CACHE_DEFAULT_SIZE (128 * 1024 * 1024)

typedef struct tsdb25_cache      tsdb25_cache_t;
typedef struct tsdb25_write_buf  tsdb25_write_buf_t;

typedef struct {
    uint64_t hits;
    uint64_t misses;
    uint64_t evictions;
    uint64_t inserts;
    size_t   current_size;
    size_t   max_size;
    size_t   entry_count;
} tsdb25_cache_stats_t;

typedef void (*tsdb25_cache_evict_cb)(uint64_t key, void* value, void* user_data);

tsdb25_cache_t* tsdb25_cache_create(size_t max_size);
void            tsdb25_cache_destroy(tsdb25_cache_t* cache);

tsdb25_status_t tsdb25_cache_put(tsdb25_cache_t* cache, uint64_t key, void* value, size_t size);
void*           tsdb25_cache_get(tsdb25_cache_t* cache, uint64_t key);
tsdb25_status_t tsdb25_cache_remove(tsdb25_cache_t* cache, uint64_t key);
bool             tsdb25_cache_contains(tsdb25_cache_t* cache, uint64_t key);

void                   tsdb25_cache_clear(tsdb25_cache_t* cache);
tsdb25_cache_stats_t  tsdb25_cache_stats(tsdb25_cache_t* cache);
double                tsdb25_cache_hit_rate(tsdb25_cache_t* cache);

void tsdb25_cache_set_evict(tsdb25_cache_t* cache, tsdb25_cache_evict_cb cb, void* user_data);

tsdb25_write_buf_t* tsdb25_write_buf_create(size_t max_size);
void                tsdb25_write_buf_destroy(tsdb25_write_buf_t* buf);

tsdb25_status_t tsdb25_write_buf_add(tsdb25_write_buf_t* buf, const tsdb25_point_t* point);
size_t          tsdb25_write_buf_count(tsdb25_write_buf_t* buf);
bool            tsdb25_write_buf_full(tsdb25_write_buf_t* buf);
tsdb25_point_t* tsdb25_write_buf_points(tsdb25_write_buf_t* buf);
void            tsdb25_write_buf_clear(tsdb25_write_buf_t* buf);

#endif
