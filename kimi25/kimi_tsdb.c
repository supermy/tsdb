#include "kimi_tsdb.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <time.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <math.h>

/* Internal structures */

typedef struct series_data {
    uint64_t series_id;
    kimi_timestamp_t* timestamps;
    kimi_value_t* values;
    size_t count;
    size_t capacity;
    kimi_timestamp_t min_ts;
    kimi_timestamp_t max_ts;
    bool dirty;
} series_data_t;

typedef struct index_entry {
    uint64_t series_id;
    char measurement[128];
    kimi_tag_t tags[32];
    size_t tag_count;
    kimi_timestamp_t min_ts;
    kimi_timestamp_t max_ts;
    uint64_t point_count;
    struct index_entry* next;
} index_entry_t;

typedef struct {
    index_entry_t** buckets;
    size_t bucket_count;
    size_t entry_count;
} index_t;

struct kimi_tsdb {
    char path[512];
    kimi_config_t config;
    int meta_fd;
    
    series_data_t** series;
    size_t series_count;
    size_t series_capacity;
    
    index_t* index;
    uint64_t total_points;
    bool is_open;
};

struct kimi_query {
    char measurement[128];
    kimi_range_t range;
    size_t limit;
    kimi_tag_t tags[32];
    size_t tag_count;
};

/* Hash functions */
static uint64_t fnv1a_64(const void* data, size_t len) {
    const uint8_t* bytes = data;
    uint64_t hash = 14695981039346656037ULL;
    for (size_t i = 0; i < len; i++) {
        hash ^= bytes[i];
        hash *= 1099511628211ULL;
    }
    return hash;
}

static uint64_t hash_point(const kimi_point_t* point) {
    uint64_t h = fnv1a_64(point->measurement, strlen(point->measurement));
    for (size_t i = 0; i < point->tag_count; i++) {
        h ^= fnv1a_64(point->tags[i].key, strlen(point->tags[i].key));
        h ^= fnv1a_64(point->tags[i].value, strlen(point->tags[i].value));
    }
    return h;
}

static size_t hash_to_bucket(uint64_t hash, size_t bucket_count) {
    return hash % bucket_count;
}

/* Index operations */
static index_t* index_create(void) {
    index_t* idx = calloc(1, sizeof(index_t));
    if (!idx) return NULL;
    
    idx->bucket_count = 1024;
    idx->buckets = calloc(idx->bucket_count, sizeof(index_entry_t*));
    if (!idx->buckets) {
        free(idx);
        return NULL;
    }
    return idx;
}

static void index_destroy(index_t* idx) {
    if (!idx) return;
    for (size_t i = 0; i < idx->bucket_count; i++) {
        index_entry_t* entry = idx->buckets[i];
        while (entry) {
            index_entry_t* next = entry->next;
            free(entry);
            entry = next;
        }
    }
    free(idx->buckets);
    free(idx);
}

static kimi_error_t index_insert(index_t* idx, uint64_t series_id, const kimi_point_t* point) {
    size_t bucket = hash_to_bucket(series_id, idx->bucket_count);
    
    index_entry_t* entry = idx->buckets[bucket];
    while (entry) {
        if (entry->series_id == series_id) {
            if (point->timestamp < entry->min_ts) entry->min_ts = point->timestamp;
            if (point->timestamp > entry->max_ts) entry->max_ts = point->timestamp;
            entry->point_count++;
            return KIMI_OK;
        }
        entry = entry->next;
    }
    
    entry = malloc(sizeof(index_entry_t));
    if (!entry) return KIMI_ERR_NO_MEMORY;
    
    entry->series_id = series_id;
    strncpy(entry->measurement, point->measurement, sizeof(entry->measurement) - 1);
    memcpy(entry->tags, point->tags, sizeof(kimi_tag_t) * point->tag_count);
    entry->tag_count = point->tag_count;
    entry->min_ts = point->timestamp;
    entry->max_ts = point->timestamp;
    entry->point_count = 1;
    entry->next = idx->buckets[bucket];
    idx->buckets[bucket] = entry;
    idx->entry_count++;
    
    return KIMI_OK;
}

static index_entry_t* index_find(index_t* idx, uint64_t series_id) {
    size_t bucket = hash_to_bucket(series_id, idx->bucket_count);
    index_entry_t* entry = idx->buckets[bucket];
    while (entry) {
        if (entry->series_id == series_id) return entry;
        entry = entry->next;
    }
    return NULL;
}

/* Series data operations */
static series_data_t* series_create(uint64_t series_id) {
    series_data_t* s = calloc(1, sizeof(series_data_t));
    if (!s) return NULL;
    
    s->series_id = series_id;
    s->capacity = 4096;
    s->timestamps = malloc(s->capacity * sizeof(kimi_timestamp_t));
    s->values = malloc(s->capacity * sizeof(kimi_value_t));
    
    if (!s->timestamps || !s->values) {
        free(s->timestamps);
        free(s->values);
        free(s);
        return NULL;
    }
    return s;
}

static void series_destroy(series_data_t* s) {
    if (!s) return;
    free(s->timestamps);
    free(s->values);
    free(s);
}

static kimi_error_t series_append(series_data_t* s, kimi_timestamp_t ts, kimi_value_t value) {
    if (s->count >= s->capacity) {
        size_t new_cap = s->capacity * 2;
        kimi_timestamp_t* new_ts = realloc(s->timestamps, new_cap * sizeof(kimi_timestamp_t));
        kimi_value_t* new_vals = realloc(s->values, new_cap * sizeof(kimi_value_t));
        if (!new_ts || !new_vals) return KIMI_ERR_NO_MEMORY;
        s->timestamps = new_ts;
        s->values = new_vals;
        s->capacity = new_cap;
    }
    
    s->timestamps[s->count] = ts;
    s->values[s->count] = value;
    s->count++;
    s->dirty = true;
    
    if (s->count == 1 || ts < s->min_ts) s->min_ts = ts;
    if (s->count == 1 || ts > s->max_ts) s->max_ts = ts;
    
    return KIMI_OK;
}

/* API Implementation */

const char* kimi_tsdb_version(void) {
    return KIMI_TSDB_VERSION;
}

const char* kimi_strerror(kimi_error_t err) {
    switch (err) {
        case KIMI_OK: return "Success";
        case KIMI_ERR_INVALID_PARAM: return "Invalid parameter";
        case KIMI_ERR_NO_MEMORY: return "Out of memory";
        case KIMI_ERR_NOT_FOUND: return "Not found";
        case KIMI_ERR_IO: return "I/O error";
        case KIMI_ERR_CORRUPTED: return "Data corrupted";
        case KIMI_ERR_EXISTS: return "Already exists";
        case KIMI_ERR_TIMEOUT: return "Timeout";
        case KIMI_ERR_FULL: return "Storage full";
        default: return "Unknown error";
    }
}

kimi_tsdb_t* kimi_tsdb_open(const char* path, const kimi_config_t* config) {
    if (!path) return NULL;
    
    kimi_tsdb_t* db = calloc(1, sizeof(kimi_tsdb_t));
    if (!db) return NULL;
    
    strncpy(db->path, path, sizeof(db->path) - 1);
    
    if (config) {
        db->config = *config;
    } else {
        db->config.cache_size_mb = 64;
        db->config.max_series = 100000;
        db->config.enable_wal = true;
        db->config.sync_write = false;
        db->config.block_size = 8192;
    }
    
    struct stat st;
    if (stat(path, &st) != 0) {
        if (mkdir(path, 0755) != 0) {
            free(db);
            return NULL;
        }
    }
    
    db->index = index_create();
    if (!db->index) {
        free(db);
        return NULL;
    }
    
    db->series_capacity = 256;
    db->series = calloc(db->series_capacity, sizeof(series_data_t*));
    if (!db->series) {
        index_destroy(db->index);
        free(db);
        return NULL;
    }
    
    db->is_open = true;
    return db;
}

void kimi_tsdb_close(kimi_tsdb_t* db) {
    if (!db) return;
    
    for (size_t i = 0; i < db->series_count; i++) {
        series_destroy(db->series[i]);
    }
    free(db->series);
    
    index_destroy(db->index);
    free(db);
}

static series_data_t* get_or_create_series(kimi_tsdb_t* db, uint64_t series_id) {
    for (size_t i = 0; i < db->series_count; i++) {
        if (db->series[i]->series_id == series_id) {
            return db->series[i];
        }
    }
    
    if (db->series_count >= db->series_capacity) {
        size_t new_cap = db->series_capacity * 2;
        series_data_t** new_series = realloc(db->series, new_cap * sizeof(series_data_t*));
        if (!new_series) return NULL;
        db->series = new_series;
        db->series_capacity = new_cap;
    }
    
    series_data_t* s = series_create(series_id);
    if (!s) return NULL;
    
    db->series[db->series_count++] = s;
    return s;
}

kimi_error_t kimi_write(kimi_tsdb_t* db, const kimi_point_t* point) {
    if (!db || !point || !db->is_open) return KIMI_ERR_INVALID_PARAM;
    
    uint64_t series_id = hash_point(point);
    series_data_t* s = get_or_create_series(db, series_id);
    if (!s) return KIMI_ERR_NO_MEMORY;
    
    kimi_value_t value = 0.0;
    if (point->field_count > 0) {
        if (point->fields[0].type == KIMI_FIELD_FLOAT) {
            value = point->fields[0].value.f64;
        } else if (point->fields[0].type == KIMI_FIELD_INT) {
            value = (kimi_value_t)point->fields[0].value.i64;
        }
    }
    
    kimi_error_t err = series_append(s, point->timestamp, value);
    if (err != KIMI_OK) return err;
    
    err = index_insert(db->index, series_id, point);
    if (err != KIMI_OK) return err;
    
    db->total_points++;
    return KIMI_OK;
}

kimi_error_t kimi_write_batch(kimi_tsdb_t* db, const kimi_point_t* points, size_t count) {
    if (!db || !points) return KIMI_ERR_INVALID_PARAM;
    
    for (size_t i = 0; i < count; i++) {
        kimi_error_t err = kimi_write(db, &points[i]);
        if (err != KIMI_OK) return err;
    }
    return KIMI_OK;
}

kimi_error_t kimi_query(kimi_tsdb_t* db, const kimi_query_t* query, kimi_result_t* result) {
    if (!db || !query || !result || !db->is_open) return KIMI_ERR_INVALID_PARAM;
    
    memset(result, 0, sizeof(kimi_result_t));
    
    result->capacity = 4096;
    result->points = calloc(result->capacity, sizeof(kimi_point_t));
    if (!result->points) return KIMI_ERR_NO_MEMORY;
    
    for (size_t i = 0; i < db->series_count && result->count < query->limit; i++) {
        series_data_t* s = db->series[i];
        
        if (s->min_ts > query->range.end || s->max_ts < query->range.start) {
            continue;
        }
        
        for (size_t j = 0; j < s->count && result->count < query->limit; j++) {
            if (s->timestamps[j] >= query->range.start && s->timestamps[j] <= query->range.end) {
                kimi_point_t* p = &result->points[result->count];
                strncpy(p->measurement, query->measurement, sizeof(p->measurement) - 1);
                p->timestamp = s->timestamps[j];
                p->field_count = 1;
                p->fields[0].type = KIMI_FIELD_FLOAT;
                p->fields[0].value.f64 = s->values[j];
                result->count++;
            }
        }
    }
    
    return KIMI_OK;
}

kimi_error_t kimi_query_agg(kimi_tsdb_t* db, const kimi_query_t* query, kimi_agg_type_t agg,
                            const char* field, kimi_agg_result_t* result) {
    if (!db || !query || !result) return KIMI_ERR_INVALID_PARAM;
    
    memset(result, 0, sizeof(kimi_agg_result_t));
    
    kimi_result_t raw;
    kimi_error_t err = kimi_query(db, query, &raw);
    if (err != KIMI_OK) return err;
    
    if (raw.count == 0) {
        kimi_result_free(&raw);
        return KIMI_OK;
    }
    
    result->count = raw.count;
    
    switch (agg) {
        case KIMI_AGG_COUNT:
            result->value = (kimi_value_t)raw.count;
            break;
        case KIMI_AGG_SUM:
            for (size_t i = 0; i < raw.count; i++) {
                if (raw.points[i].field_count > 0)
                    result->value += raw.points[i].fields[0].value.f64;
            }
            break;
        case KIMI_AGG_AVG:
            for (size_t i = 0; i < raw.count; i++) {
                if (raw.points[i].field_count > 0)
                    result->value += raw.points[i].fields[0].value.f64;
            }
            result->value /= raw.count;
            break;
        case KIMI_AGG_MIN:
            result->value = INFINITY;
            for (size_t i = 0; i < raw.count; i++) {
                if (raw.points[i].field_count > 0) {
                    double v = raw.points[i].fields[0].value.f64;
                    if (v < result->value) result->value = v;
                }
            }
            break;
        case KIMI_AGG_MAX:
            result->value = -INFINITY;
            for (size_t i = 0; i < raw.count; i++) {
                if (raw.points[i].field_count > 0) {
                    double v = raw.points[i].fields[0].value.f64;
                    if (v > result->value) result->value = v;
                }
            }
            break;
        case KIMI_AGG_FIRST:
            if (raw.points[0].field_count > 0) {
                result->value = raw.points[0].fields[0].value.f64;
                result->timestamp = raw.points[0].timestamp;
            }
            break;
        case KIMI_AGG_LAST:
            if (raw.points[raw.count - 1].field_count > 0) {
                result->value = raw.points[raw.count - 1].fields[0].value.f64;
                result->timestamp = raw.points[raw.count - 1].timestamp;
            }
            break;
        default:
            break;
    }
    
    kimi_result_free(&raw);
    return KIMI_OK;
}

/* Query builder */
kimi_query_t* kimi_query_new(const char* measurement) {
    if (!measurement) return NULL;
    
    kimi_query_t* q = calloc(1, sizeof(kimi_query_t));
    if (!q) return NULL;
    
    strncpy(q->measurement, measurement, sizeof(q->measurement) - 1);
    q->limit = 10000;
    q->range.start = 0;
    q->range.end = INT64_MAX;
    return q;
}

void kimi_query_free(kimi_query_t* query) {
    free(query);
}

kimi_query_t* kimi_query_range(kimi_query_t* query, kimi_range_t range) {
    if (query) query->range = range;
    return query;
}

kimi_query_t* kimi_query_limit(kimi_query_t* query, size_t limit) {
    if (query) query->limit = limit;
    return query;
}

kimi_query_t* kimi_query_tag(kimi_query_t* query, const char* key, const char* value) {
    if (!query || !key || !value) return query;
    if (query->tag_count >= 32) return query;
    
    strncpy(query->tags[query->tag_count].key, key, sizeof(query->tags[0].key) - 1);
    strncpy(query->tags[query->tag_count].value, value, sizeof(query->tags[0].value) - 1);
    query->tag_count++;
    return query;
}

/* Point builder */
kimi_point_t* kimi_point_create(const char* measurement, kimi_timestamp_t ts) {
    if (!measurement) return NULL;
    
    kimi_point_t* p = calloc(1, sizeof(kimi_point_t));
    if (!p) return NULL;
    
    strncpy(p->measurement, measurement, sizeof(p->measurement) - 1);
    p->timestamp = ts > 0 ? ts : kimi_now();
    return p;
}

void kimi_point_destroy(kimi_point_t* point) {
    free(point);
}

kimi_point_t* kimi_point_add_tag(kimi_point_t* point, const char* key, const char* value) {
    if (!point || !key || !value) return point;
    if (point->tag_count >= 32) return point;
    
    strncpy(point->tags[point->tag_count].key, key, sizeof(point->tags[0].key) - 1);
    strncpy(point->tags[point->tag_count].value, value, sizeof(point->tags[0].value) - 1);
    point->tag_count++;
    return point;
}

kimi_point_t* kimi_point_add_field_f64(kimi_point_t* point, const char* name, double value) {
    if (!point || !name) return point;
    if (point->field_count >= 64) return point;
    
    strncpy(point->fields[point->field_count].name, name, sizeof(point->fields[0].name) - 1);
    point->fields[point->field_count].type = KIMI_FIELD_FLOAT;
    point->fields[point->field_count].value.f64 = value;
    point->field_count++;
    return point;
}

kimi_point_t* kimi_point_add_field_i64(kimi_point_t* point, const char* name, int64_t value) {
    if (!point || !name) return point;
    if (point->field_count >= 64) return point;
    
    strncpy(point->fields[point->field_count].name, name, sizeof(point->fields[0].name) - 1);
    point->fields[point->field_count].type = KIMI_FIELD_INT;
    point->fields[point->field_count].value.i64 = value;
    point->field_count++;
    return point;
}

kimi_point_t* kimi_point_add_field_bool(kimi_point_t* point, const char* name, bool value) {
    if (!point || !name) return point;
    if (point->field_count >= 64) return point;
    
    strncpy(point->fields[point->field_count].name, name, sizeof(point->fields[0].name) - 1);
    point->fields[point->field_count].type = KIMI_FIELD_BOOL;
    point->fields[point->field_count].value.boolean = value;
    point->field_count++;
    return point;
}

kimi_point_t* kimi_point_add_field_str(kimi_point_t* point, const char* name, const char* value) {
    if (!point || !name || !value) return point;
    if (point->field_count >= 64) return point;
    
    strncpy(point->fields[point->field_count].name, name, sizeof(point->fields[0].name) - 1);
    point->fields[point->field_count].type = KIMI_FIELD_STRING;
    strncpy(point->fields[point->field_count].value.str, value, sizeof(point->fields[0].value.str) - 1);
    point->field_count++;
    return point;
}

/* Result management */
void kimi_result_free(kimi_result_t* result) {
    if (!result) return;
    free(result->points);
    result->points = NULL;
    result->count = 0;
    result->capacity = 0;
}

/* Utility */
kimi_timestamp_t kimi_now(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (kimi_timestamp_t)ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

kimi_timestamp_t kimi_now_ms(void) {
    return kimi_now() / 1000000LL;
}
