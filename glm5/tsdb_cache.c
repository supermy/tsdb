#include "tsdb_cache.h"
#include <stdlib.h>
#include <string.h>

typedef struct cache_entry {
    uint64_t key;
    void* value;
    size_t size;
    struct cache_entry* prev;
    struct cache_entry* next;
} cache_entry_t;

struct tsdb_cache {
    cache_entry_t** buckets;
    size_t bucket_count;
    cache_entry_t* head;
    cache_entry_t* tail;
    size_t max_size;
    size_t current_size;
    size_t entry_count;
    uint64_t hits;
    uint64_t misses;
    uint64_t evictions;
    uint64_t inserts;
    tsdb_cache_evict_cb evict_callback;
    void* evict_user_data;
};

static size_t hash_key(uint64_t key, size_t bucket_count) {
    return key % bucket_count;
}

tsdb_cache_t* tsdb_cache_create(size_t max_size) {
    tsdb_cache_t* cache = (tsdb_cache_t*)calloc(1, sizeof(tsdb_cache_t));
    if (!cache) return NULL;
    
    cache->bucket_count = 1024;
    cache->buckets = (cache_entry_t**)calloc(cache->bucket_count, sizeof(cache_entry_t*));
    if (!cache->buckets) {
        free(cache);
        return NULL;
    }
    
    cache->max_size = max_size;
    cache->head = NULL;
    cache->tail = NULL;
    
    return cache;
}

static void remove_from_list(tsdb_cache_t* cache, cache_entry_t* entry) {
    if (entry->prev) {
        entry->prev->next = entry->next;
    } else {
        cache->head = entry->next;
    }
    
    if (entry->next) {
        entry->next->prev = entry->prev;
    } else {
        cache->tail = entry->prev;
    }
}

static void add_to_head(tsdb_cache_t* cache, cache_entry_t* entry) {
    entry->prev = NULL;
    entry->next = cache->head;
    
    if (cache->head) {
        cache->head->prev = entry;
    }
    cache->head = entry;
    
    if (!cache->tail) {
        cache->tail = entry;
    }
}

static void evict_lru(tsdb_cache_t* cache) {
    while (cache->tail && cache->current_size > cache->max_size) {
        cache_entry_t* lru = cache->tail;
        
        remove_from_list(cache, lru);
        
        size_t bucket = hash_key(lru->key, cache->bucket_count);
        cache_entry_t* prev = NULL;
        cache_entry_t* curr = cache->buckets[bucket];
        
        while (curr) {
            if (curr->key == lru->key) {
                if (prev) {
                    prev->next = curr->next;
                } else {
                    cache->buckets[bucket] = curr->next;
                }
                break;
            }
            prev = curr;
            curr = curr->next;
        }
        
        cache->current_size -= lru->size;
        cache->entry_count--;
        cache->evictions++;
        
        if (cache->evict_callback) {
            cache->evict_callback(lru->key, lru->value, cache->evict_user_data);
        }
        
        free(lru->value);
        free(lru);
    }
}

void tsdb_cache_destroy(tsdb_cache_t* cache) {
    if (!cache) return;
    
    tsdb_cache_clear(cache);
    free(cache->buckets);
    free(cache);
}

tsdb_status_t tsdb_cache_put(tsdb_cache_t* cache, uint64_t key, void* value, size_t size) {
    if (!cache || !value) return TSDB_ERR_INVALID_PARAM;
    
    size_t bucket = hash_key(key, cache->bucket_count);
    
    cache_entry_t* entry = cache->buckets[bucket];
    while (entry) {
        if (entry->key == key) {
            cache->current_size -= entry->size;
            free(entry->value);
            entry->value = value;
            entry->size = size;
            cache->current_size += size;
            
            remove_from_list(cache, entry);
            add_to_head(cache, entry);
            
            evict_lru(cache);
            return TSDB_OK;
        }
        entry = entry->next;
    }
    
    entry = (cache_entry_t*)malloc(sizeof(cache_entry_t));
    if (!entry) return TSDB_ERR_NO_MEMORY;
    
    entry->key = key;
    entry->value = value;
    entry->size = size;
    entry->prev = NULL;
    entry->next = cache->buckets[bucket];
    
    if (cache->buckets[bucket]) {
        cache->buckets[bucket]->prev = entry;
    }
    cache->buckets[bucket] = entry;
    
    add_to_head(cache, entry);
    
    cache->current_size += size;
    cache->entry_count++;
    cache->inserts++;
    
    evict_lru(cache);
    
    return TSDB_OK;
}

void* tsdb_cache_get(tsdb_cache_t* cache, uint64_t key) {
    if (!cache) return NULL;
    
    size_t bucket = hash_key(key, cache->bucket_count);
    cache_entry_t* entry = cache->buckets[bucket];
    
    while (entry) {
        if (entry->key == key) {
            cache->hits++;
            
            remove_from_list(cache, entry);
            add_to_head(cache, entry);
            
            return entry->value;
        }
        entry = entry->next;
    }
    
    cache->misses++;
    return NULL;
}

tsdb_status_t tsdb_cache_remove(tsdb_cache_t* cache, uint64_t key) {
    if (!cache) return TSDB_ERR_INVALID_PARAM;
    
    size_t bucket = hash_key(key, cache->bucket_count);
    cache_entry_t* prev = NULL;
    cache_entry_t* entry = cache->buckets[bucket];
    
    while (entry) {
        if (entry->key == key) {
            if (prev) {
                prev->next = entry->next;
            } else {
                cache->buckets[bucket] = entry->next;
            }
            
            remove_from_list(cache, entry);
            
            cache->current_size -= entry->size;
            cache->entry_count--;
            
            if (cache->evict_callback) {
                cache->evict_callback(entry->key, entry->value, cache->evict_user_data);
            }
            
            free(entry->value);
            free(entry);
            
            return TSDB_OK;
        }
        prev = entry;
        entry = entry->next;
    }
    
    return TSDB_ERR_NOT_FOUND;
}

bool tsdb_cache_contains(tsdb_cache_t* cache, uint64_t key) {
    if (!cache) return false;
    
    size_t bucket = hash_key(key, cache->bucket_count);
    cache_entry_t* entry = cache->buckets[bucket];
    
    while (entry) {
        if (entry->key == key) {
            return true;
        }
        entry = entry->next;
    }
    
    return false;
}

void tsdb_cache_clear(tsdb_cache_t* cache) {
    if (!cache) return;
    
    cache_entry_t* entry = cache->head;
    while (entry) {
        cache_entry_t* next = entry->next;
        
        if (cache->evict_callback) {
            cache->evict_callback(entry->key, entry->value, cache->evict_user_data);
        }
        
        free(entry->value);
        free(entry);
        entry = next;
    }
    
    memset(cache->buckets, 0, cache->bucket_count * sizeof(cache_entry_t*));
    cache->head = NULL;
    cache->tail = NULL;
    cache->current_size = 0;
    cache->entry_count = 0;
}

tsdb_cache_stats_t tsdb_cache_get_stats(tsdb_cache_t* cache) {
    tsdb_cache_stats_t stats = {0};
    
    if (cache) {
        stats.hits = cache->hits;
        stats.misses = cache->misses;
        stats.evictions = cache->evictions;
        stats.inserts = cache->inserts;
        stats.current_size = cache->current_size;
        stats.max_size = cache->max_size;
        stats.entry_count = cache->entry_count;
    }
    
    return stats;
}

void tsdb_cache_set_evict_callback(tsdb_cache_t* cache, tsdb_cache_evict_cb callback, void* user_data) {
    if (cache) {
        cache->evict_callback = callback;
        cache->evict_user_data = user_data;
    }
}

double tsdb_cache_hit_rate(tsdb_cache_t* cache) {
    if (!cache) return 0.0;
    
    uint64_t total = cache->hits + cache->misses;
    if (total == 0) return 0.0;
    
    return (double)cache->hits / (double)total;
}

struct tsdb_write_buffer {
    tsdb_point_t* points;
    size_t count;
    size_t capacity;
    size_t max_size;
};

tsdb_write_buffer_t* tsdb_write_buffer_create(size_t max_size) {
    tsdb_write_buffer_t* buf = (tsdb_write_buffer_t*)calloc(1, sizeof(tsdb_write_buffer_t));
    if (!buf) return NULL;
    
    buf->max_size = max_size;
    buf->capacity = 64;
    buf->points = (tsdb_point_t*)calloc(buf->capacity, sizeof(tsdb_point_t));
    
    if (!buf->points) {
        free(buf);
        return NULL;
    }
    
    return buf;
}

void tsdb_write_buffer_destroy(tsdb_write_buffer_t* buf) {
    if (!buf) return;
    
    free(buf->points);
    free(buf);
}

tsdb_status_t tsdb_write_buffer_add(tsdb_write_buffer_t* buf, const tsdb_point_t* point) {
    if (!buf || !point) return TSDB_ERR_INVALID_PARAM;
    
    if (buf->count >= buf->capacity) {
        size_t new_capacity = buf->capacity * 2;
        if (new_capacity > buf->max_size) {
            new_capacity = buf->max_size;
        }
        
        if (buf->count >= new_capacity) {
            return TSDB_ERR_FULL;
        }
        
        tsdb_point_t* new_points = (tsdb_point_t*)realloc(
            buf->points, new_capacity * sizeof(tsdb_point_t));
        if (!new_points) return TSDB_ERR_NO_MEMORY;
        
        buf->points = new_points;
        buf->capacity = new_capacity;
    }
    
    buf->points[buf->count++] = *point;
    return TSDB_OK;
}

size_t tsdb_write_buffer_count(tsdb_write_buffer_t* buf) {
    return buf ? buf->count : 0;
}

bool tsdb_write_buffer_is_full(tsdb_write_buffer_t* buf) {
    return buf ? (buf->count >= buf->max_size) : true;
}

tsdb_point_t* tsdb_write_buffer_get_points(tsdb_write_buffer_t* buf) {
    return buf ? buf->points : NULL;
}

void tsdb_write_buffer_clear(tsdb_write_buffer_t* buf) {
    if (buf) {
        buf->count = 0;
    }
}
