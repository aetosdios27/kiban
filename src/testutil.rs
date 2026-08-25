//! Shared test helpers. Compiled only under `cfg(test)`.

#![cfg(test)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new(label: &str) -> Self {
        let base = env::temp_dir();
        let n = DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = base.join(format!("kiban-test-{label}-{}-{}", std::process::id(), n));
        fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
