#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <sys/uio.h>
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
/* std::io formatting may split a command into multiple writes or iovecs. */
static void control(int fd, const void *buf, size_t n) {
    static int command_fd = -1;
    static size_t used;
    static const char *expected;
    const unsigned char *p = buf;
    for (size_t i = 0; i < n; i++) {
        if (fd != command_fd) used = 0;
        if (!used) {
            if (p[i] != 'e' && p[i] != 'd') continue;
            command_fd = fd;
            expected = p[i] == 'e' ? "enable\n" : "disable\n";
        }
        if (p[i] != (unsigned char)expected[used]) {
            used = 0;
            continue;
        }
        used++;
        if (!expected[used]) {
            int enable = expected[0] == 'e';
            used = 0;
            if (enable) {
                calls = bytes = nanos = 0;
                active = 1;
            } else if (active) {
                active = 0;
                dprintf(2, "SYSCALL_PROBE calls=%lu bytes=%lu ns=%lu\n", calls, bytes, nanos);
            }
        }
    }
}
ssize_t write(int fd, const void *buf, size_t n) {
    static ssize_t (*real_write)(int, const void *, size_t);
    if (!real_write) real_write = dlsym(RTLD_NEXT, "write");
    control(fd, buf, n);
    return real_write(fd, buf, n);
}
ssize_t writev(int fd, const struct iovec *iov, int count) {
    static ssize_t (*real_writev)(int, const struct iovec *, int);
    if (!real_writev) real_writev = dlsym(RTLD_NEXT, "writev");
    for (int i = 0; i < count; i++) control(fd, iov[i].iov_base, iov[i].iov_len);
    return real_writev(fd, iov, count);
}
