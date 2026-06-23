// cleaned up on drop so a test run never leaves garbage in /tmp, even
// when a test panics or returns early

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

pub(crate) struct TmpDir(PathBuf);

impl TmpDir {
    // a fresh, empty dir under the temp dir, unique per call, removed on drop
    pub(crate) fn new(tag: &str) -> Self {
        let n = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("nexus-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("create scratch dir");
        TmpDir(p)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Deref for TmpDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
