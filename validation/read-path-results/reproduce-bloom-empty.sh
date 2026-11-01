#!/usr/bin/env bash
set -eu
cd "$(dirname "$0")/../.."
taskset -c 2-5 cargo build --release --lib
taskset -c 2-5 rustc --edition=2024 \
  validation/read-path-results/bloom-empty-repro.rs \
  --extern kiban=target/release/libkiban.rlib \
  -L dependency=target/release/deps \
  -o target/read-path-tools/bloom-empty-repro
taskset -c 3 target/read-path-tools/bloom-empty-repro
