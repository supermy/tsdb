#include "tsdb25_types.h"
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
    "Locked",
};

const char* tsdb25_strerror(tsdb25_status_t status) {
    if (status == TSDB25_OK) return error_messages[0];
    int idx = -status;
    if (idx > 0 && idx < (int)(sizeof(error_messages) / sizeof(error_messages[0]))) {
        return error_messages[idx];
    }
    return "Unknown error";
}
