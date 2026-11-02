#ifndef RUST_DEEP_CHECKED_ADD_H
#define RUST_DEEP_CHECKED_ADD_H

#include <stdint.h>

typedef struct {
    int32_t value;
    uint8_t status;
} RustDeepAddResult;

RustDeepAddResult rust_deep_checked_add(int32_t left, int32_t right);

#endif
