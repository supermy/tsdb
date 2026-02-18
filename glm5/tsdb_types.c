#include "tsdb_types.h"
#include <string.h>

static const char* error_messages[] = {
    "Success",
    "Invalid parameter",
    "Out of memory",
    "Not found",
    "I/O error",
    "Data corrupted",
    "Storage full",
    "Already exists",
    "Invalid state",
    "Compression error",
    "Timeout",
};

const char* tsdb_strerror(tsdb_status_t status) {
    int idx = -status;
    if (status == TSDB_OK) {
        return error_messages[0];
    }
    if (idx > 0 && idx < (int)(sizeof(error_messages) / sizeof(error_messages[0]))) {
        return error_messages[idx];
    }
    return "Unknown error";
}
