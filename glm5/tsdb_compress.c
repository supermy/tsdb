#include "tsdb_compress.h"
#include <stdlib.h>
#include <string.h>

tsdb_compress_config_t tsdb_compress_default_config(void) {
    tsdb_compress_config_t config;
    memset(&config, 0, sizeof(config));
    
    config.type = TSDB_COMPRESS_NONE;
    config.level = 3;
    config.enabled = false;
    
    return config;
}

tsdb_status_t tsdb_compress_init(const tsdb_compress_config_t* config) {
    (void)config;
    return TSDB_OK;
}

void tsdb_compress_shutdown(void) {
}

tsdb_buffer_t* tsdb_buffer_create(size_t initial_capacity) {
    tsdb_buffer_t* buf = (tsdb_buffer_t*)malloc(sizeof(tsdb_buffer_t));
    if (!buf) return NULL;
    
    buf->capacity = initial_capacity > 0 ? initial_capacity : 256;
    buf->data = (uint8_t*)malloc(buf->capacity);
    buf->size = 0;
    
    if (!buf->data) {
        free(buf);
        return NULL;
    }
    
    return buf;
}

void tsdb_buffer_destroy(tsdb_buffer_t* buf) {
    if (!buf) return;
    
    free(buf->data);
    free(buf);
}

tsdb_status_t tsdb_buffer_ensure_capacity(tsdb_buffer_t* buf, size_t needed) {
    if (!buf) return TSDB_ERR_INVALID_PARAM;
    
    if (needed > buf->capacity) {
        size_t new_capacity = buf->capacity;
        while (new_capacity < needed) {
            new_capacity *= 2;
        }
        
        uint8_t* new_data = (uint8_t*)realloc(buf->data, new_capacity);
        if (!new_data) return TSDB_ERR_NO_MEMORY;
        
        buf->data = new_data;
        buf->capacity = new_capacity;
    }
    
    return TSDB_OK;
}

tsdb_status_t tsdb_buffer_append(tsdb_buffer_t* buf, const void* data, size_t size) {
    if (!buf || !data) return TSDB_ERR_INVALID_PARAM;
    
    tsdb_status_t status = tsdb_buffer_ensure_capacity(buf, buf->size + size);
    if (status != TSDB_OK) return status;
    
    memcpy(buf->data + buf->size, data, size);
    buf->size += size;
    
    return TSDB_OK;
}

void tsdb_buffer_clear(tsdb_buffer_t* buf) {
    if (buf) {
        buf->size = 0;
    }
}

size_t tsdb_compress_bound(size_t input_size, tsdb_compress_type_t type) {
    switch (type) {
        case TSDB_COMPRESS_NONE:
            return input_size;
        case TSDB_COMPRESS_SNAPPY:
        case TSDB_COMPRESS_ZSTD:
        case TSDB_COMPRESS_LZ4:
            return input_size + (input_size / 6) + 64;
        case TSDB_COMPRESS_GORILLA:
            return input_size;
        default:
            return input_size * 2;
    }
}

const char* tsdb_compress_type_name(tsdb_compress_type_t type) {
    switch (type) {
        case TSDB_COMPRESS_NONE: return "none";
        case TSDB_COMPRESS_SNAPPY: return "snappy";
        case TSDB_COMPRESS_ZSTD: return "zstd";
        case TSDB_COMPRESS_LZ4: return "lz4";
        case TSDB_COMPRESS_GORILLA: return "gorilla";
        default: return "unknown";
    }
}

tsdb_status_t tsdb_compress_data(
    const tsdb_compress_config_t* config,
    const uint8_t* input,
    size_t input_size,
    uint8_t** output,
    size_t* output_size
) {
    if (!config || !input || !output || !output_size) {
        return TSDB_ERR_INVALID_PARAM;
    }
    
    if (!config->enabled || config->type == TSDB_COMPRESS_NONE) {
        *output = (uint8_t*)malloc(input_size);
        if (!*output) return TSDB_ERR_NO_MEMORY;
        
        memcpy(*output, input, input_size);
        *output_size = input_size;
        return TSDB_OK;
    }
    
    size_t bound = tsdb_compress_bound(input_size, config->type);
    *output = (uint8_t*)malloc(bound);
    if (!*output) return TSDB_ERR_NO_MEMORY;
    
    memcpy(*output, input, input_size);
    *output_size = input_size;
    
    return TSDB_OK;
}

tsdb_status_t tsdb_decompress_data(
    const tsdb_compress_config_t* config,
    const uint8_t* input,
    size_t input_size,
    uint8_t** output,
    size_t* output_size
) {
    if (!config || !input || !output || !output_size) {
        return TSDB_ERR_INVALID_PARAM;
    }
    
    if (!config->enabled || config->type == TSDB_COMPRESS_NONE) {
        *output = (uint8_t*)malloc(input_size);
        if (!*output) return TSDB_ERR_NO_MEMORY;
        
        memcpy(*output, input, input_size);
        *output_size = input_size;
        return TSDB_OK;
    }
    
    *output = (uint8_t*)malloc(input_size);
    if (!*output) return TSDB_ERR_NO_MEMORY;
    
    memcpy(*output, input, input_size);
    *output_size = input_size;
    
    return TSDB_OK;
}

tsdb_status_t tsdb_compress_timestamps(
    const tsdb_timestamp_t* timestamps,
    size_t count,
    uint8_t** output,
    size_t* output_size
) {
    if (!timestamps || !output || !output_size) {
        return TSDB_ERR_INVALID_PARAM;
    }
    
    size_t data_size = count * sizeof(tsdb_timestamp_t);
    *output = (uint8_t*)malloc(data_size);
    if (!*output) return TSDB_ERR_NO_MEMORY;
    
    memcpy(*output, timestamps, data_size);
    *output_size = data_size;
    
    return TSDB_OK;
}

tsdb_status_t tsdb_decompress_timestamps(
    const uint8_t* input,
    size_t input_size,
    tsdb_timestamp_t** timestamps,
    size_t* count
) {
    if (!input || !timestamps || !count) {
        return TSDB_ERR_INVALID_PARAM;
    }
    
    size_t num_timestamps = input_size / sizeof(tsdb_timestamp_t);
    *timestamps = (tsdb_timestamp_t*)malloc(input_size);
    if (!*timestamps) return TSDB_ERR_NO_MEMORY;
    
    memcpy(*timestamps, input, input_size);
    *count = num_timestamps;
    
    return TSDB_OK;
}

tsdb_status_t tsdb_compress_values(
    const tsdb_value_t* values,
    size_t count,
    uint8_t** output,
    size_t* output_size
) {
    if (!values || !output || !output_size) {
        return TSDB_ERR_INVALID_PARAM;
    }
    
    size_t data_size = count * sizeof(tsdb_value_t);
    *output = (uint8_t*)malloc(data_size);
    if (!*output) return TSDB_ERR_NO_MEMORY;
    
    memcpy(*output, values, data_size);
    *output_size = data_size;
    
    return TSDB_OK;
}

tsdb_status_t tsdb_decompress_values(
    const uint8_t* input,
    size_t input_size,
    tsdb_value_t** values,
    size_t* count
) {
    if (!input || !values || !count) {
        return TSDB_ERR_INVALID_PARAM;
    }
    
    size_t num_values = input_size / sizeof(tsdb_value_t);
    *values = (tsdb_value_t*)malloc(input_size);
    if (!*values) return TSDB_ERR_NO_MEMORY;
    
    memcpy(*values, input, input_size);
    *count = num_values;
    
    return TSDB_OK;
}
