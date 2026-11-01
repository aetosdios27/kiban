/* LD_PRELOAD observation tool. Use only in excluded diagnostic runs. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>

static _Atomic uint64_t sync_calls, sync_ns, sync_errors;
static _Atomic uint64_t pwrite_calls, pwrite_bytes, durable_pwrite_calls, durable_pwrite_ns;
static uint64_t started;

static uint64_t now(clockid_t clock) {
    struct timespec t;
    clock_gettime(clock, &t);
    return (uint64_t)t.tv_sec * 1000000000 + t.tv_nsec;
}

__attribute__((constructor)) static void begin(void) { started = now(CLOCK_MONOTONIC); }

int fdatasync(int fd) {
    static _Atomic(void*) address;
    int (*real_fn)(int) = atomic_load(&address);
    if (!real_fn) { real_fn = dlsym(RTLD_NEXT, "fdatasync"); atomic_store(&address, real_fn); }
    uint64_t start = now(CLOCK_MONOTONIC);
    int result = real_fn(fd);
    atomic_fetch_add(&sync_calls, 1);
    atomic_fetch_add(&sync_ns, now(CLOCK_MONOTONIC)-start);
    if (result) atomic_fetch_add(&sync_errors, 1);
    return result;
}

int fsync(int fd) {
    static _Atomic(void*) address;
    int (*real_fn)(int) = atomic_load(&address);
    if (!real_fn) { real_fn = dlsym(RTLD_NEXT, "fsync"); atomic_store(&address, real_fn); }
    uint64_t start = now(CLOCK_MONOTONIC);
    int result = real_fn(fd);
    atomic_fetch_add(&sync_calls, 1);
    atomic_fetch_add(&sync_ns, now(CLOCK_MONOTONIC)-start);
    if (result) atomic_fetch_add(&sync_errors, 1);
    return result;
}

#define OBSERVE_PWRITE(name, offset_type) \
ssize_t name(int fd, const void *buffer, size_t size, offset_type offset) { \
    static _Atomic(void*) address; \
    ssize_t (*real_fn)(int,const void*,size_t,offset_type) = atomic_load(&address); \
    if (!real_fn) { real_fn = dlsym(RTLD_NEXT, #name); atomic_store(&address, real_fn); } \
    int flags = fcntl(fd,F_GETFL); \
    int durable = flags >= 0 && (flags & (O_SYNC | O_DSYNC)); \
    uint64_t start = now(CLOCK_MONOTONIC); \
    ssize_t result = real_fn(fd,buffer,size,offset); \
    uint64_t elapsed = now(CLOCK_MONOTONIC)-start; \
    atomic_fetch_add(&pwrite_calls,1); \
    if (result > 0) atomic_fetch_add(&pwrite_bytes,result); \
    if (durable) { atomic_fetch_add(&durable_pwrite_calls,1); atomic_fetch_add(&durable_pwrite_ns,elapsed); } \
    return result; \
}

OBSERVE_PWRITE(pwrite, off_t)
OBSERVE_PWRITE(pwrite64, off64_t)

__attribute__((destructor)) static void finish(void) {
    const char *path = getenv("ENGINEBENCH_SYNC_PROFILE");
    if (!path) return;
    FILE *out = fopen(path,"wx");
    if (!out) return;
    fprintf(out,"{\"wall_ns\":%llu,\"process_cpu_ns\":%llu,\"sync_calls\":%llu,\"sync_ns\":%llu,\"sync_errors\":%llu,\"pwrite_calls\":%llu,\"pwrite_bytes\":%llu,\"durable_pwrite_calls\":%llu,\"durable_pwrite_ns\":%llu}\n",
        (unsigned long long)(now(CLOCK_MONOTONIC)-started),
        (unsigned long long)now(CLOCK_PROCESS_CPUTIME_ID),
        (unsigned long long)sync_calls,(unsigned long long)sync_ns,(unsigned long long)sync_errors,
        (unsigned long long)pwrite_calls,(unsigned long long)pwrite_bytes,
        (unsigned long long)durable_pwrite_calls,(unsigned long long)durable_pwrite_ns);
    fclose(out);
}
