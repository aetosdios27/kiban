/* cc -O2 -I/path/to/xxHash-v0.8.3 xxh3-vectors.c -o /tmp/xxh3-vectors
 * /tmp/xxh3-vectors > xxh3-vectors.txt; no engine dependency. */
#define XXH_INLINE_ALL
#include "xxhash.h"
#include <stdio.h>
#include <stdlib.h>
static void emit(size_t n, unsigned pattern) {
    unsigned char *b = malloc(n + 1);
    for (size_t i = 0; i < n; ++i) b[i+1] = (unsigned char)(i*131 + pattern*17 + (i>>3));
    printf("%zu %u %016llx\n", n, pattern, (unsigned long long)XXH3_64bits(b+1,n));
    free(b);
}
int main(void) {
    for (unsigned p=0; p<4; ++p) {
        for (size_t n=0; n<=4097; ++n) emit(n,p);
        size_t edges[] = {8191,8192,8193,65535,65536,65537,1048576};
        for (size_t i=0; i<sizeof(edges)/sizeof(edges[0]); ++i) emit(edges[i],p);
    }
}
