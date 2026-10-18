/* Diagnostic only: count successful/failed libc sync calls for a whole process.
 * LD_PRELOAD this library on fresh WRITE copies after the latency matrix.
 * This is not an engine change and its timings are never reported.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdatomic.h>
#include <stdio.h>
#include <unistd.h>

static _Atomic unsigned long fsync_calls;
static _Atomic unsigned long fdatasync_calls;
static _Atomic unsigned long failures;

int fsync(int fd) {
    int (*next)(int) = (int (*)(int))dlsym(RTLD_NEXT, "fsync");
    int rc = next(fd);
    atomic_fetch_add(&fsync_calls, 1);
    if (rc) atomic_fetch_add(&failures, 1);
    return rc;
}

int fdatasync(int fd) {
    int (*next)(int) = (int (*)(int))dlsym(RTLD_NEXT, "fdatasync");
    int rc = next(fd);
    atomic_fetch_add(&fdatasync_calls, 1);
    if (rc) atomic_fetch_add(&failures, 1);
    return rc;
}

__attribute__((destructor)) static void report(void) {
    fprintf(stderr, "sync_probe fsync=%lu fdatasync=%lu failures=%lu\n",
            atomic_load(&fsync_calls), atomic_load(&fdatasync_calls), atomic_load(&failures));
}
