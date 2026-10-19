#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdint.h>
#include <sys/mman.h>
#include <sys/uio.h>
#include <unistd.h>
/* Counts durability and write syscalls process-wide. The benchmark brackets
   its timed region with gcp_reset/gcp_read (found through dlsym). Counting
   is one relaxed atomic add per call; it does not time or log anything. */
static uint64_t c[6]; /* fdatasync, fsync, msync, sync_file_range, writes, write_bytes */
#define ADD(i, n) __atomic_fetch_add(&c[i], (n), __ATOMIC_RELAXED)
void gcp_reset(void) { for (int i = 0; i < 6; i++) __atomic_store_n(&c[i], 0, __ATOMIC_RELAXED); }
void gcp_read(uint64_t *out) { for (int i = 0; i < 6; i++) out[i] = __atomic_load_n(&c[i], __ATOMIC_RELAXED); }

typedef int (*sync_fn)(int);
typedef int (*msync_fn)(void *, size_t, int);
typedef int (*range_fn)(int, off64_t, off64_t, unsigned);
typedef ssize_t (*write_fn)(int, const void *, size_t);
typedef ssize_t (*pwrite_fn)(int, const void *, size_t, off_t);
typedef ssize_t (*pwrite64_fn)(int, const void *, size_t, off64_t);
typedef ssize_t (*writev_fn)(int, const struct iovec *, int);
typedef ssize_t (*pwritev_fn)(int, const struct iovec *, int, off_t);
#define NEXT(name, T) static T real; if (!real) real = (T)dlsym(RTLD_NEXT, name)
#define WROTE(r) do { ADD(4, 1); if ((r) > 0) ADD(5, (uint64_t)(r)); } while (0)

int fdatasync(int fd) { NEXT("fdatasync", sync_fn); ADD(0, 1); return real(fd); }
int fsync(int fd) { NEXT("fsync", sync_fn); ADD(1, 1); return real(fd); }
int msync(void *a, size_t n, int f) { NEXT("msync", msync_fn); ADD(2, 1); return real(a, n, f); }
int sync_file_range(int fd, off64_t o, off64_t n, unsigned f) {
    NEXT("sync_file_range", range_fn); ADD(3, 1); return real(fd, o, n, f);
}
ssize_t write(int fd, const void *b, size_t n) { NEXT("write", write_fn); ssize_t r = real(fd, b, n); WROTE(r); return r; }
ssize_t pwrite(int fd, const void *b, size_t n, off_t o) { NEXT("pwrite", pwrite_fn); ssize_t r = real(fd, b, n, o); WROTE(r); return r; }
ssize_t pwrite64(int fd, const void *b, size_t n, off64_t o) { NEXT("pwrite64", pwrite64_fn); ssize_t r = real(fd, b, n, o); WROTE(r); return r; }
ssize_t writev(int fd, const struct iovec *v, int n) { NEXT("writev", writev_fn); ssize_t r = real(fd, v, n); WROTE(r); return r; }
ssize_t pwritev(int fd, const struct iovec *v, int n, off_t o) { NEXT("pwritev", pwritev_fn); ssize_t r = real(fd, v, n, o); WROTE(r); return r; }
