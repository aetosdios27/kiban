#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
/* Diagnostic only. The perf FIFO brackets the operation loop. Clock overhead
   is included. Never use this preload for latency comparisons. */
static int active;
static uint64_t calls, bytes, nanos;
static uint64_t now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (uint64_t)t.tv_sec * 1000000000 + t.tv_nsec;
}
ssize_t pread64(int fd, void *buf, size_t n, off64_t off) {
    static ssize_t (*real_pread)(int, void *, size_t, off64_t);
    if (!real_pread) real_pread = dlsym(RTLD_NEXT, "pread64");
    uint64_t tick = active ? now() : 0;
    ssize_t result = real_pread(fd, buf, n, off);
    if (active) {
        calls++;
        if (result > 0) bytes += (uint64_t)result;
        nanos += now() - tick;
    }
    return result;
}
ssize_t write(int fd, const void *buf, size_t n) {
    static ssize_t (*real_write)(int, const void *, size_t);
    if (!real_write) real_write = dlsym(RTLD_NEXT, "write");
    if (n == 7 && memcmp(buf, "enable\n", 7) == 0) {
        calls = bytes = nanos = 0;
        active = 1;
    }
    if (n == 8 && memcmp(buf, "disable\n", 8) == 0 && active) {
        active = 0;
        dprintf(2, "SYSCALL_PROBE calls=%lu bytes=%lu ns=%lu\n", calls, bytes, nanos);
    }
    return real_write(fd, buf, n);
}
