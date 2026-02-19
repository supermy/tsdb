#include "rkimi_tsdb.h"
#include <rocksdb/c.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <time.h>
#include <math.h>
#include <sys/stat.h>

struct rkimi_db {
    rocksdb_t* db;
    rocksdb_options_t* opts;
    rocksdb_writeoptions_t* wopts;
    rocksdb_readoptions_t* ropts;
    uint64_t total;
};

static uint64_t hash_series(const rkimi_point_t* p) {
    uint64_t h = 14695981039346656037ULL;
    const uint8_t* d = (const uint8_t*)p->measurement;
    for (size_t i = 0; i < strlen(p->measurement); i++) {
        h ^= d[i]; h *= 1099511628211ULL;
    }
    for (size_t i = 0; i < p->tag_count; i++) {
        d = (const uint8_t*)p->tags[i].key;
        for (size_t j = 0; j < strlen(p->tags[i].key); j++) { h ^= d[j]; h *= 1099511628211ULL; }
        d = (const uint8_t*)p->tags[i].value;
        for (size_t j = 0; j < strlen(p->tags[i].value); j++) { h ^= d[j]; h *= 1099511628211ULL; }
    }
    return h;
}

static void make_key(char* b, size_t n, const char* m, uint64_t sid, rkimi_ts_t ts) {
    snprintf(b, n, "%s_%lu_%lld", m, (unsigned long)sid, (long long)ts);
}

const char* rkimi_version(void) { return RKIMI_VERSION; }

rkimi_db_t* rkimi_open(const char* path) {
    if (!path) return NULL;
    struct stat st; if (stat(path, &st) != 0) mkdir(path, 0755);
    rkimi_db_t* db = calloc(1, sizeof(rkimi_db_t));
    if (!db) return NULL;
    db->opts = rocksdb_options_create();
    rocksdb_options_set_create_if_missing(db->opts, 1);
    char* err = NULL;
    db->db = rocksdb_open(db->opts, path, &err);
    if (err) { free(err); free(db); return NULL; }
    db->wopts = rocksdb_writeoptions_create();
    db->ropts = rocksdb_readoptions_create();
    return db;
}

void rkimi_close(rkimi_db_t* db) {
    if (!db) return;
    if (db->db) rocksdb_close(db->db);
    if (db->opts) rocksdb_options_destroy(db->opts);
    if (db->wopts) rocksdb_writeoptions_destroy(db->wopts);
    if (db->ropts) rocksdb_readoptions_destroy(db->ropts);
    free(db);
}

rkimi_error_t rkimi_write(rkimi_db_t* db, const rkimi_point_t* p) {
    if (!db || !p) return RKIMI_ERR_INVALID;
    char k[512]; make_key(k, sizeof(k), p->measurement, hash_series(p), p->timestamp);
    char v[64]; snprintf(v, sizeof(v), "%.10f", p->value);
    char* err = NULL;
    rocksdb_put(db->db, db->wopts, k, strlen(k), v, strlen(v), &err);
    if (err) { free(err); return RKIMI_ERR_IO; }
    db->total++;
    return RKIMI_OK;
}

rkimi_error_t rkimi_write_batch(rkimi_db_t* db, const rkimi_point_t* pts, size_t n) {
    if (!db || !pts) return RKIMI_ERR_INVALID;
    rocksdb_writebatch_t* b = rocksdb_writebatch_create();
    for (size_t i = 0; i < n; i++) {
        const rkimi_point_t* p = &pts[i];
        char k[512]; make_key(k, sizeof(k), p->measurement, hash_series(p), p->timestamp);
        char v[64]; snprintf(v, sizeof(v), "%.10f", p->value);
        rocksdb_writebatch_put(b, k, strlen(k), v, strlen(v));
    }
    char* err = NULL;
    rocksdb_write(db->db, db->wopts, b, &err);
    rocksdb_writebatch_destroy(b);
    if (err) { free(err); return RKIMI_ERR_IO; }
    db->total += n;
    return RKIMI_OK;
}

rkimi_error_t rkimi_query(rkimi_db_t* db, const char* m, const rkimi_range_t* r, size_t lim, rkimi_result_t* res) {
    if (!db || !m || !res) return RKIMI_ERR_INVALID;
    memset(res, 0, sizeof(rkimi_result_t));
    res->capacity = lim > 0 ? lim : 1000;
    res->points = calloc(res->capacity, sizeof(rkimi_point_t));
    if (!res->points) return RKIMI_ERR_MEMORY;
    rocksdb_iterator_t* it = rocksdb_create_iterator(db->db, db->ropts);
    char prefix[256]; snprintf(prefix, sizeof(prefix), "%s_", m);
    size_t plen = strlen(prefix);
    rocksdb_iter_seek(it, prefix, plen);
    while (rocksdb_iter_valid(it) && res->count < lim) {
        size_t kl; const char* k = rocksdb_iter_key(it, &kl);
        if (kl < plen || memcmp(k, prefix, plen) != 0) break;
        size_t vl; const char* v = rocksdb_iter_value(it, &vl);
        if (res->count >= res->capacity) {
            res->capacity *= 2;
            rkimi_point_t* np = realloc(res->points, res->capacity * sizeof(rkimi_point_t));
            if (!np) { rocksdb_iter_destroy(it); return RKIMI_ERR_MEMORY; }
            res->points = np;
        }
        rkimi_point_t* p = &res->points[res->count];
        strncpy(p->measurement, m, sizeof(p->measurement) - 1);
        p->timestamp = 0;
        const char* ks = (const char*)k;
        char* lu = strrchr(ks, '_');
        if (lu) p->timestamp = strtoll(lu + 1, NULL, 10);
        if (!r || (p->timestamp >= r->start && p->timestamp <= r->end)) {
            p->value = atof(v);
            res->count++;
        }
        rocksdb_iter_next(it);
    }
    rocksdb_iter_destroy(it);
    return RKIMI_OK;
}

rkimi_error_t rkimi_agg(rkimi_db_t* db, const char* m, const rkimi_range_t* r, rkimi_agg_t agg, rkimi_agg_result_t* res) {
    if (!db || !m || !res) return RKIMI_ERR_INVALID;
    memset(res, 0, sizeof(rkimi_agg_result_t));
    rkimi_result_t data;
    rkimi_error_t err = rkimi_query(db, m, r, 1000000, &data);
    if (err != RKIMI_OK) return err;
    if (data.count == 0) { rkimi_result_free(&data); return RKIMI_OK; }
    res->count = data.count;
    switch (agg) {
        case RKIMI_AGG_COUNT: res->value = data.count; break;
        case RKIMI_AGG_SUM:
            for (size_t i = 0; i < data.count; i++) res->value += data.points[i].value;
            break;
        case RKIMI_AGG_AVG:
            for (size_t i = 0; i < data.count; i++) res->value += data.points[i].value;
            res->value /= data.count;
            break;
        case RKIMI_AGG_MIN:
            res->value = INFINITY;
            for (size_t i = 0; i < data.count; i++) if (data.points[i].value < res->value) res->value = data.points[i].value;
            break;
        case RKIMI_AGG_MAX:
            res->value = -INFINITY;
            for (size_t i = 0; i < data.count; i++) if (data.points[i].value > res->value) res->value = data.points[i].value;
            break;
    }
    rkimi_result_free(&data);
    return RKIMI_OK;
}

rkimi_point_t* rkimi_point_new(const char* m, rkimi_ts_t ts) {
    if (!m) return NULL;
    rkimi_point_t* p = calloc(1, sizeof(rkimi_point_t));
    if (!p) return NULL;
    strncpy(p->measurement, m, sizeof(p->measurement) - 1);
    p->timestamp = ts > 0 ? ts : rkimi_now();
    return p;
}

void rkimi_point_free(rkimi_point_t* p) { free(p); }

rkimi_point_t* rkimi_tag(rkimi_point_t* p, const char* k, const char* v) {
    if (!p || !k || !v || p->tag_count >= 32) return p;
    strncpy(p->tags[p->tag_count].key, k, 63);
    strncpy(p->tags[p->tag_count].value, v, 255);
    p->tag_count++;
    return p;
}

rkimi_point_t* rkimi_val(rkimi_point_t* p, double v) {
    if (p) p->value = v;
    return p;
}

void rkimi_result_free(rkimi_result_t* r) {
    if (!r) return;
    free(r->points);
    r->points = NULL; r->count = 0; r->capacity = 0;
}

rkimi_ts_t rkimi_now(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (rkimi_ts_t)ts.tv_sec * 1000000000LL + ts.tv_nsec;
}
