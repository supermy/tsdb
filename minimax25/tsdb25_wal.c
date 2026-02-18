#include "tsdb25_wal.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <time.h>

struct tsdb25_wal {
    char           path[TSDB25_MAX_PATH_LEN];
    int            fd;
    size_t         max_size;
    size_t         current_size;
    uint64_t       entry_count;
    uint64_t       sequence;
};

tsdb25_wal_t* tsdb25_wal_create(const char* path, size_t max_size) {
    tsdb25_wal_t* w = (tsdb25_wal_t*)calloc(1, sizeof(tsdb25_wal_t));
    if (!w) return NULL;
    
    strncpy(w->path, path, TSDB25_MAX_PATH_LEN - 1);
    w->max_size = max_size;
    w->fd = -1;
    w->sequence = 1;
    
    return w;
}

void tsdb25_wal_destroy(tsdb25_wal_t* wal) {
    if (!wal) return;
    tsdb25_wal_close(wal);
    free(wal);
}

tsdb25_status_t tsdb25_wal_open(tsdb25_wal_t* wal) {
    if (!wal) return TSDB25_ERR_INVALID_PARAM;
    
    wal->fd = open(wal->path, O_RDWR | O_CREAT | O_APPEND, 0644);
    if (wal->fd < 0) return TSDB25_ERR_IO_ERROR;
    
    struct stat st;
    if (fstat(wal->fd, &st) == 0) {
        wal->current_size = st.st_size;
    }
    
    if (wal->current_size == 0) {
        tsdb25_wal_header_t header;
        memset(&header, 0, sizeof(header));
        header.magic = TSDB25_WAL_MAGIC;
        header.version = TSDB25_WAL_VERSION;
        header.entry_count = 0;
        header.file_size = sizeof(header);
        write(wal->fd, &header, sizeof(header));
        wal->current_size = sizeof(header);
    }
    
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_wal_close(tsdb25_wal_t* wal) {
    if (!wal) return TSDB25_ERR_INVALID_PARAM;
    if (wal->fd >= 0) {
        close(wal->fd);
        wal->fd = -1;
    }
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_wal_append(tsdb25_wal_t* wal, const tsdb25_point_t* point) {
    if (!wal || !point) return TSDB25_ERR_INVALID_PARAM;
    if (wal->fd < 0) return TSDB25_ERR_INVALID_STATE;
    
    size_t point_size = sizeof(tsdb25_point_t);
    size_t entry_size = sizeof(tsdb25_wal_entry_t) + point_size;
    
    if (wal->current_size + entry_size > wal->max_size) {
        return TSDB25_ERR_FULL;
    }
    
    tsdb25_wal_entry_t* entry = (tsdb25_wal_entry_t*)malloc(entry_size);
    if (!entry) return TSDB25_ERR_NO_MEMORY;
    
    entry->type = TSDB25_WAL_ENTRY_POINT;
    entry->size = point_size;
    entry->sequence = wal->sequence++;
    entry->timestamp = point->timestamp;
    memcpy(entry->data, point, point_size);
    
    ssize_t written = write(wal->fd, entry, entry_size);
    free(entry);
    
    if (written != (ssize_t)entry_size) {
        return TSDB25_ERR_IO_ERROR;
    }
    
    wal->current_size += entry_size;
    wal->entry_count++;
    
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_wal_flush(tsdb25_wal_t* wal) {
    if (!wal) return TSDB25_ERR_INVALID_PARAM;
    if (wal->fd >= 0) {
        fsync(wal->fd);
    }
    return TSDB25_OK;
}

tsdb25_status_t tsdb25_wal_recover(tsdb25_wal_t* wal, tsdb25_point_t** points, size_t* count) {
    if (!wal || !points || !count) return TSDB25_ERR_INVALID_PARAM;
    
    *points = NULL;
    *count = 0;
    
    if (wal->fd < 0) return TSDB25_ERR_INVALID_STATE;
    
    lseek(wal->fd, sizeof(tsdb25_wal_header_t), SEEK_SET);
    
    size_t capacity = 256;
    *points = (tsdb25_point_t*)malloc(capacity * sizeof(tsdb25_point_t));
    if (!*points) return TSDB25_ERR_NO_MEMORY;
    
    tsdb25_wal_entry_t entry;
    while (read(wal->fd, &entry, sizeof(entry)) == sizeof(entry)) {
        if (entry.type == TSDB25_WAL_ENTRY_POINT) {
            if (*count >= capacity) {
                capacity *= 2;
                tsdb25_point_t* new_pts = (tsdb25_point_t*)realloc(*points, capacity * sizeof(tsdb25_point_t));
                if (!new_pts) {
                    free(*points);
                    return TSDB25_ERR_NO_MEMORY;
                }
                *points = new_pts;
            }
            read(wal->fd, &(*points)[*count], entry.size);
            (*count)++;
        } else {
            lseek(wal->fd, entry.size, SEEK_CUR);
        }
    }
    
    return TSDB25_OK;
}

uint64_t tsdb25_wal_entry_count(tsdb25_wal_t* wal) {
    return wal ? wal->entry_count : 0;
}

tsdb25_status_t tsdb25_wal_truncate(tsdb25_wal_t* wal, uint64_t keep_count) {
    if (!wal) return TSDB25_ERR_INVALID_PARAM;
    return TSDB25_OK;
}
