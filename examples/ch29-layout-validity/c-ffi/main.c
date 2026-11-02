#include "checked_add.h"
#include <assert.h>
#include <stddef.h>

_Static_assert(offsetof(RustDeepAddResult, status) == 4, "C field offset");
_Static_assert(sizeof(RustDeepAddResult) == 8, "C struct size");

int main(void) {
    RustDeepAddResult sum = rust_deep_checked_add(20, 22);
    assert(sum.value == 42 && sum.status == 0);
    RustDeepAddResult overflow = rust_deep_checked_add(INT32_MAX, 1);
    assert(overflow.value == 0 && overflow.status == 1);
    return 0;
}
