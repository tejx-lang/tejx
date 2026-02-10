#include <stdio.h>

// TejX Runtime Library
// Currently we call printf directly from LLVM IR, 
// but we can add helpers here.

void tejx_hello() {
    printf("TejX Runtime Initialized\n");
}

// Declaration of the generated entry point
// Ensure C linkage if compiled as C++
#ifdef __cplusplus
extern "C" {
#endif

long long tejx_main();

#ifdef __cplusplus
}
#endif

int main() {
    // optional: tejx_hello();
    return (int)tejx_main();
}
