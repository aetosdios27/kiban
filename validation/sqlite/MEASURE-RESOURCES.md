# Measure resources

## read-1000000-bulk-512-fast

| Engine | Peak RSS MiB [min,max] | Logical disk MiB | Allocated disk MiB |
|---|---:|---:|---:|
| kiban-index0 | 433.609 [433.480, 433.836] | 314.564 | 314.574 |
| kiban-index1 | 422.598 [422.453, 422.699] | 391.176 | 391.184 |
| lmdb | 812.055 [811.949, 812.832] | 799.305 | 799.297 |
| sqlite-mmap | 316.891 [315.621, 317.023] | 314.559 | 314.559 |
| sqlite-normal | 337.504 [337.402, 338.312] | 314.559 | 314.559 |

## read-1000000-bulk-None-fast

| Engine | Peak RSS MiB [min,max] | Logical disk MiB | Allocated disk MiB |
|---|---:|---:|---:|
| kiban-index0 | 433.590 [433.539, 433.715] | 314.564 | 314.574 |
| kiban-index1 | 422.523 [422.508, 422.660] | 391.176 | 391.184 |
| lmdb | 812.324 [812.145, 812.727] | 799.305 | 799.297 |
| sqlite-mmap | 316.215 [316.090, 316.637] | 314.559 | 314.559 |
| sqlite-normal | 338.289 [337.629, 338.527] | 314.559 | 314.559 |

## read-1000000-normal-512-fast

| Engine | Peak RSS MiB [min,max] | Logical disk MiB | Allocated disk MiB |
|---|---:|---:|---:|
| kiban-index0 | 430.105 [426.281, 433.719] | 314.573 | 314.699 |
| kiban-index1 | 453.223 [436.703, 460.309] | 391.197 | 391.402 |
| lmdb | 812.172 [811.961, 812.801] | 799.305 | 799.297 |
| sqlite-mmap | 316.027 [315.891, 316.949] | 314.559 | 314.559 |
| sqlite-normal | 337.926 [337.473, 338.059] | 314.559 | 314.559 |

## read-1000000-normal-None-fast

| Engine | Peak RSS MiB [min,max] | Logical disk MiB | Allocated disk MiB |
|---|---:|---:|---:|
| kiban-index0 | 428.688 [424.594, 430.520] | 314.573 | 314.699 |
| kiban-index1 | 448.422 [432.766, 457.820] | 391.197 | 391.402 |
| lmdb | 812.309 [812.203, 812.883] | 799.305 | 799.297 |
| sqlite-mmap | 316.285 [316.148, 316.473] | 314.559 | 314.559 |
| sqlite-normal | 337.793 [337.484, 338.297] | 314.559 | 314.559 |

## write-10000-bulk-None-durable

| Engine | Peak RSS MiB [min,max] | Logical disk MiB | Allocated disk MiB |
|---|---:|---:|---:|
| kiban-index0 | 15.535 [15.445, 15.594] | 3.742 | 3.754 |
| kiban-index1 | 19.379 [19.328, 19.523] | 4.658 | 4.668 |
| lmdb | 14.375 [14.324, 14.438] | 8.633 | 8.625 |
| sqlite-full | 10.355 [10.133, 10.461] | 3.477 | 3.477 |
| sqlite-normal | 10.266 [10.195, 10.512] | 3.477 | 3.477 |

## write-10000-bulk-None-fast

| Engine | Peak RSS MiB [min,max] | Logical disk MiB | Allocated disk MiB |
|---|---:|---:|---:|
| kiban-index0 | 15.680 [15.562, 15.820] | 3.742 | 3.754 |
| kiban-index1 | 19.730 [19.652, 19.820] | 4.658 | 4.668 |
| lmdb | 14.336 [14.207, 14.379] | 8.633 | 8.625 |
| sqlite-normal | 10.398 [10.324, 10.449] | 3.477 | 3.477 |
| sqlite-off | 10.398 [10.191, 10.625] | 3.477 | 3.477 |
