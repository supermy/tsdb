#include "tsdb25_cache.h"
#include <stdlib.h>
#include <string.h>

typedef struct cache_entry {
    uint64_t key;
    void*    value;
    size_t   size;
    struct cache_entry* prev;
    struct cache_entry* next;
} cache_entry_t;

struct tsdb25_cache {
    cache_entry_t** buckets;
    size_t          bucket_count;
    cache_entry_t*  head;
    cache_entry_t*  tail;
    size_t          max_size;
    size_t          current_size;
    size_t          entry_count;
    uint64_t        hits;
    uint64_t        misses;
    uint64_t        evictions;
    uint64_t        inserts;
    tsdb25_cache_evict_cb evict_cb;
    void*            evict_ud;
};

static size_t hash_key(uint64_t key, size_t bucket_count) {
    return key % bucket_count;
}

tsdb25_cache_t* tsdb25_cache_create(size_t max_size) {
    tsdb25_cache_t* c = (tsdb25_cache_t*)calloc(1, sizeof(tsdb25_cache_t));
    if (!c) return NULL;
    
    c->bucket_count = 2048;
    c->buckets = (cache_entry_t**)calloc(c->bucket_count, sizeof(cache_entry_t*));
    if (!c->buckets) {
        free(c);
        return NULL;
    }
    
    c->max_size = max_size;
    return c;
}

static void detach_entry(tsdb25_cache_t* c, cache_entry_t* e) {
    if (e->prev) e->prev->next = e->next;
    else c->head = e->next;
    if (e->next) e->next->prev = e->prev;
    else c->tail = e->prev;
}

static void attach_head(tsdb25_cache_t* c, cache_entry_t* e) {
    e->prev = NULL;
    e->next = c->head;
    if (c->head) c->head->prev = e;
    c->head = e;
    if (!c->tail) c->tail = e;
}

static void evict_lru(tsdb25_cache_t* c) {
    while (c->tail && c->current_size > c->max_size) {
        cache_entry_t* e = c->tail;
        
        detach_entry(c, e);
        
        size_t bucket = hash_key(e->key, c->bucket_count);
        cache_entry_t* p = NULL;
        cache_entry_t* cur = c->buckets[bucket];
        while (cur) {
            if (cur->key == e->key) {
                if (p) p->next = cur->next;
                else c->buckets[bucket] = cur->next;
                break;
            }
            p = cur;
            cur = cur->next;
        }
        
        c->current_size -= e->size;
        c->entry_count--;
        c->evictions++;
        
        if (c->evict_cb) c->evict_cb(e->key, e->value, c->evict_ud);
        
        free(e->value);
        free(e);
    }
}

void tsdb25_cache_destroy(tsdb25_cache_t* cache) {
    if (!cache) return;
    tsdb25_cache_clear(cache);
    free(cache->buckets);
    free(cache);
}

tsdb25_status_t tsdb25_cache_put(tsdb25_cache_t* cache, uint64_t key, void* value, size_t size) {
    if (!cache || !value) return TSDB25_ERR_INVALID_PARAM;
    
    size_t bucket = hash_key(key, cache->bucket_count);
    cache_entry_t* e = cache->buckets[bucket];
    while (e) {
        if (e->key == key) {
            cache->current_size -= e->size;
            free(e->value);
            e->value = value;
            e->size = size;
            cache->current_size += size;
            detach_entry(cache, e);
            attach_head(cache, e);
            evict_lru(cache);
            return TSDB25_OK;
        }
        e = e->next;
    }
    
    e = (cache_entry_t*)malloc(sizeof(cache_entry_t));
    if (!e) return TSDB25_ERR_NO_MEMORY;
    
    e->key = key;
    e->value = value;
    e->size = size;
    e->prev = NULL;
    e->next = cache->buckets[bucket];
    if (cache->buckets[bucket]) cache->buckets[bucket]->prev = e;
    cache->buckets[bucket] = e;
    
    attach_head(cache, e);
    cache->current_size += size;
    cache->entry_count++;
    cache->inserts++;
    
    evict_lru(cache);
    return TSDB25_OK;
}

void* tsdb25_cache_get(tsdb25_cache_t* cache, uint64_t key) {
    if (!cache) return NULL;
    
    size_t bucket = hash_key(key, cache->bucket_count);
    cache_entry_t* e = cache->buckets[bucket];
    while (e) {
        if (e->key == key) {
            cache->hits++;
            detach_entry(cache, e);
            attach_head(cache, e);
            return e->value;
        }
        e = e->next;
    }
    cache->misses++;
    return NULL;
}

tsdb25_status_t tsdb25_cache_remove(tsdb25_cache_t* cache, uint64_t key) {
    if (!cache) return TSDB25_ERR_INVALID_PARAM;
    
    size_t bucket = hash_key(key, cache->bucket_count);
    cache_entry_t* p = NULL;
    cache_entry_t* e = cache->buckets[bucket];
    while (e) {
        if (e->key == key) {
            if (p) p->next = e->next;
            else cache->buckets[bucket] = e->next;
            detach_entry(cache, e);
            cache->current_size -= e->size;
            cache->entry_count--;
            if (cache->evict_cb) cache->evict_cb(e->key, e->value, cache->evict_ud);
            free(e->value);
            free(e);
            return TSDB25_OK;
        }
        p = e;
        e = e->next;
    }
    return TSDB25_ERR_NOT_FOUND;
}

bool tsdb25_cache_contains(tsdb25_cache_t* cache, uint64_t key) {
    if (!cache) return false;
    size_t bucket = hash_key(key, cache->bucket_count);
    cache_entry_t* e = cache->buckets[bucket];
    while (e) {
        if (e->key == key) return true;
        e = e->next;
    }
    return false;
}

void tsdb25_cache_clear(tsdb25_cache_t* cache) {
    if (!cache) return;
    cache_entry_t* e = cache->head;
    while (e) {
        cache_entry_t* n = e->next;
        if (cache->evict_cb) cache->evict_cb(e->key, e->value, cache->evict_ud);
        free(e->value);
        free(e);
        e = n;
    }
    memset(cache->buckets, 0, cache->bucket_count * sizeof(cache_entry_t*));
    cache->head = NULL;
    cache->tail = NULL;
    cache->current_size = 0;
    cache->entry_count = 0;
}

tsdb25_cache_stats_t tsdb25_cache_stats(tsdb25_cache_t* cache) {
    tsdb25_cache_stats_t s = {0};
    if (cache) {
        s.hits = cache->hits;
        s.misses = cache->misses;
        s.evictions = cache->evictions;
        s.inserts = cache->inserts;
        s.current_size = cache->current_size;
        s.max_size = cache->max_size;
        s.entry_count = cache->entry_count;
    }
    return s;
}

double tsdb25_cache_hit_rate(tsdb25_cache_t* cache) {
    if (!cache) return 0.0;
    uint64_t total = cache->hits + cache->misses;
    if (total == 0) return 0.0;
    return (double)cache->hits / (double)total;
}

void tsdb25_cache_set_evict(tsdb25_cache_t* cache, tsdb25_cache_evict_cb cb, void* user_data) {
    if (cache) {
        cache->evict_cb = cb;
        cache->evict_ud = user_data;
    }
}

struct tsdb25_write_buf {
    tsdb25_point_t* points;
    size_t          count;
    size_t          capacity;
    size_t          max_size;
};

tsdb25_write_buf_t* tsdb25_write_buf_create(size_t max_size) {
    tsdb25_write_buf_t* b = (tsdb25_write_buf_t*)calloc(1, sizeof(tsdb25_write_buf_t));
    if (!b) return NULL;
    b->max_size = max_size;
    b->capacity = 256;
    b->points = (tsdb25_point_t*)calloc(b->capacity, sizeof(tsdb25_point_t));
    if (!b->points) {
        free(b);
        return NULL;
    }
    return b;
}

void tsdb25_write_buf_destroy(tsdb25_write_buf_t* buf) {
    if (!buf) return;
    free(buf->points);
    free(buf);
}

tsdb25_status_t tsdb25_write_buf_add(tsdb25_write_buf_t* buf, const tsdb25_point_t* point) {
    if (!buf || !point) return TSDB25_ERR_INVALID_PARAM;
    if (buf->count >= buf->max_size) return TSDB25_ERR_FULL;
    if (buf->count >= buf->capacity) {
        size_t new_cap = buf->capacity * 2;
        if (new_cap > buf->max_size) new_cap = buf->max_size;
        tsdb25_point_t* new_pts = (tsdb25_point_t*)realloc(buf->points, new_cap * sizeof(tsdb25_point_t));
        if (!new_pts) return TSDB25_ERR_NO_MEMORY;
        buf->points = new_pts;
        buf->capacity = new_cap;
    }
    buf->points[buf->count++] = *point;
    return TSDB25_OK;
}

size_t tsdb25_write_buf_count(tsdb25_write_buf_t* buf) {
    return buf ? buf->count : 0;
}

bool tsdb25_write_buf_full(tsdb25_write_buf_t* buf) {
    return buf ? (buf->count >= buf->max_size) : true;
}

tsdb25_point_t* tsdb25_write_buf_points(tsdb25_write_buf_t* buf) {
    return buf ? buf->points : NULL;
}

void tsdb25_write_buf_clear(tsdb25_write_buf_t* buf) {
    if (buf) buf->count = 0;
}
