#ifndef TSDB25_WAL_H
#define TSDB25_WAL_H

#include "tsdb25_types.h"

#define TSDB25_WAL_MAGIC    0x57414C25
#define TSDB25_WAL_VERSION  1

typedef struct tsdb25_wal tsdb25_wal_t;

typedef struct {
    uint32_t magic;
    uint32_t version;
    uint64_t entry_count;
    uint64_t file_size;
    char     reserved[480];
} tsdb25_wal_header_t;

typedef struct {
    uint32_t    type;
    uint32_t    size;
    uint64_t    sequence;
    tsdb25_timestamp_t timestamp;
    char        data[];
} tsdb25_wal_entry_t;

typedef enum {
    TSDB25_WAL_ENTRY_POINT = 1,
    TSDB25_WAL_ENTRY_DELETE = 2,
    TSDB25_WAL_ENTRY_CHECKPOINT = 3,
} tsdb25_wal_entry_type_t;

tsdb25_wal_t* tsdb25_wal_create(const char* path, size_t max_size);
void           tsdb25_wal_destroy(tsdb25_wal_t* wal);

tsdb25_status_t tsdb25_wal_open(tsdb25_wal_t* wal);
tsdb25_status_t tsdb25_wal_close(tsdb25_wal_t* wal);

tsdb25_status_t tsdb25_wal_append(tsdb25_wal_t* wal, const tsdb25_point_t* point);
tsdb25_status_t tsdb25_wal_flush(tsdb25_wal_t* wal);
tsdb25_status_t tsdb25_wal_recover(tsdb25_wal_t* wal, tsdb25_point_t** points, size_t* count);

uint64_t tsdb25_wal_entry_count(tsdb25_wal_t* wal);
tsdb25_status_t tsdb25_wal_truncate(tsdb25_wal_t* wal, uint64_t keep_count);

#endif
