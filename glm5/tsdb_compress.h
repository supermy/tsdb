#ifndef TSDB_COMPRESS_H
#define TSDB_COMPRESS_H

#include "tsdb_types.h"

typedef enum {
    TSDB_COMPRESS_NONE = 0,
    TSDB_COMPRESS_SNAPPY = 1,
    TSDB_COMPRESS_ZSTD = 2,
    TSDB_COMPRESS_LZ4 = 3,
    TSDB_COMPRESS_GORILLA = 4,
} tsdb_compress_type_t;

typedef struct {
    tsdb_compress_type_t type;
    int level;
    bool enabled;
} tsdb_compress_config_t;

typedef struct {
    uint8_t* data;
    size_t size;
    size_t capacity;
} tsdb_buffer_t;

tsdb_buffer_t* tsdb_buffer_create(size_t initial_capacity);
void tsdb_buffer_destroy(tsdb_buffer_t* buf);
tsdb_status_t tsdb_buffer_ensure_capacity(tsdb_buffer_t* buf, size_t needed);
tsdb_status_t tsdb_buffer_append(tsdb_buffer_t* buf, const void* data, size_t size);
void tsdb_buffer_clear(tsdb_buffer_t* buf);

tsdb_compress_config_t tsdb_compress_default_config(void);

tsdb_status_t tsdb_compress_init(const tsdb_compress_config_t* config);
void tsdb_compress_shutdown(void);

tsdb_status_t tsdb_compress_data(
    const tsdb_compress_config_t* config,
    const uint8_t* input,
    size_t input_size,
    uint8_t** output,
    size_t* output_size
);

tsdb_status_t tsdb_decompress_data(
    const tsdb_compress_config_t* config,
    const uint8_t* input,
    size_t input_size,
    uint8_t** output,
    size_t* output_size
);

tsdb_status_t tsdb_compress_timestamps(
    const tsdb_timestamp_t* timestamps,
    size_t count,
    uint8_t** output,
    size_t* output_size
);

tsdb_status_t tsdb_decompress_timestamps(
    const uint8_t* input,
    size_t input_size,
    tsdb_timestamp_t** timestamps,
    size_t* count
);

tsdb_status_t tsdb_compress_values(
    const tsdb_value_t* values,
    size_t count,
    uint8_t** output,
    size_t* output_size
);

tsdb_status_t tsdb_decompress_values(
    const uint8_t* input,
    size_t input_size,
    tsdb_value_t** values,
    size_t* count
);

size_t tsdb_compress_bound(size_t input_size, tsdb_compress_type_t type);

const char* tsdb_compress_type_name(tsdb_compress_type_t type);

#endif
