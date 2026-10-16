#ifndef KIBAN_H
#define KIBAN_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

/* Linux ABI. All functions return a kiban_status. Handles are process-local,
 * nonzero IDs, never pointers. Wrong-kind, closed, and unknown IDs are rejected.
 * Calls are thread-safe. Close waits for in-flight calls on that database.
 * Calls racing a release may finish if they already acquired that handle.
 * Close consumes the handle even on error. Snapshots remain valid after close
 * and retain directory ownership until release. Iterators own copied entries
 * and remain valid after their database/snapshot is closed/released.
 *
 * Inputs: non-NULL pointers must reference aligned, live memory of the stated
 * size for the call. NULL byte inputs are allowed only with length zero.
 * Outputs must be writable, nonoverlapping, and must not alias inputs. They
 * are initialized to zero/empty on failure (except invalid output pointers).
 * Arbitrary dangling or forged pointers cannot be checked by this ABI.
 *
 * Returned buffers belong to the library; free each once with kiban_free_buf.
 * Do not modify data/len or use a buffer after free. Empty buffers are {NULL,0}.
 * Last error is UTF-8, thread-local, and retained until the next failure.
 * NOT_FOUND and DONE are normal terminal statuses. A successful write is
 * buffered; call sync/checkpoint/close to acknowledge durability.
 * Panics are caught with status PANIC; process aborts cannot be caught.
 */
typedef uint64_t kiban_handle;
typedef int32_t kiban_status;
enum {
    KIBAN_OK = 0, KIBAN_NOT_FOUND = 1, KIBAN_DONE = 2,
    KIBAN_INVALID_ARGUMENT = 3, KIBAN_INVALID_HANDLE = 4, KIBAN_LOCKED = 5,
    KIBAN_IO = 6, KIBAN_CORRUPT = 7, KIBAN_COMMIT_FAILED = 8,
    KIBAN_COMMIT_AMBIGUOUS = 9, KIBAN_POISONED = 10, KIBAN_MAINTENANCE = 11,
    KIBAN_SEQUENCE_EXHAUSTED = 12, KIBAN_PANIC = 13
};
typedef struct { uint8_t *data; size_t len; } kiban_buf;
/* NULL options or zero fields select engine defaults. */
typedef struct {
    size_t write_buffer_bytes, block_cache_bytes, max_open_table_files;
} kiban_open_options;
enum { KIBAN_PUT = 1, KIBAN_DELETE = 2 };
typedef struct {
    uint32_t kind;
    const uint8_t *key;
    size_t key_len;
    const uint8_t *value;
    size_t value_len;
} kiban_batch_op;

/* Path is a nonempty Linux byte path without NUL, not NUL-terminated. */
kiban_status kiban_open(const uint8_t *path, size_t path_len, const kiban_open_options *options, kiban_handle *out);
kiban_status kiban_close(kiban_handle db);
kiban_status kiban_put(kiban_handle db, const uint8_t *key, size_t key_len, const uint8_t *value, size_t value_len);
kiban_status kiban_get(kiban_handle db, const uint8_t *key, size_t key_len, kiban_buf *out);
kiban_status kiban_delete(kiban_handle db, const uint8_t *key, size_t key_len);
/* Validates every operation before one atomic engine write; sync separately. */
kiban_status kiban_write_batch(kiban_handle db, const kiban_batch_op *ops, size_t count);
kiban_status kiban_sync(kiban_handle db);
kiban_status kiban_flush(kiban_handle db);
kiban_status kiban_checkpoint(kiban_handle db);
kiban_status kiban_snapshot_create(kiban_handle db, kiban_handle *out);
kiban_status kiban_snapshot_release(kiban_handle snapshot);
kiban_status kiban_get_at_snapshot(kiban_handle snapshot, const uint8_t *key, size_t key_len, kiban_buf *out);
/* Both scan functions accept a DB or snapshot handle. Creation materializes
 * the selected point-in-time entries: O(result bytes) memory, not streaming.
 * Range is [start,end); NULL end with zero length means unbounded.
 * Non-NULL end with zero length means the empty upper bound. */
kiban_status kiban_scan_range(kiban_handle source, const uint8_t *start, size_t start_len, const uint8_t *end, size_t end_len, kiban_handle *out);
kiban_status kiban_scan_prefix(kiban_handle source, const uint8_t *prefix, size_t prefix_len, kiban_handle *out);
kiban_status kiban_iter_next(kiban_handle iterator, kiban_buf *key, kiban_buf *value);
kiban_status kiban_iter_free(kiban_handle iterator);
kiban_status kiban_last_error(kiban_buf *out);
kiban_status kiban_free_buf(kiban_buf buf);
#ifdef __cplusplus
}
#endif
#endif
