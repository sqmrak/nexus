// filesystem layout of the meta-distro. the root is chosen at runtime so a
// fork or test harness can place it anywhere. absolute paths stay const below

use crate::api::{Error, Result};
use std::path::{Path, PathBuf};

// create a directory and all parents, mapping failure to a stateful error
pub(crate) fn mkdir_all(p: &Path) -> Result<()> {
    std::fs::create_dir_all(p).map_err(|e| Error::Io(format!("mkdir {p:?}: {e}")))
}

// refuse a path if any ancestor is a symlink. create_dir_all follows
// symlinks, so an attacker could divert writable state outside the tree
pub(crate) fn require_no_symlink_ancestor(p: &Path, root: &Path) -> Result<()> {
    for anc in p.ancestors() {
        if anc == root {
            break;
        }
        if anc == p {
            continue;
        }
        match std::fs::symlink_metadata(anc) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(Error::Io(format!(
                    "refusing to use {p:?}: ancestor {anc:?} is a symlink"
                )));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // not created yet; mkdir_all covers it
            }
            Err(e) => {
                return Err(Error::Io(format!("stat {anc:?}: {e}")));
            }
            _ => {}
        }
    }
    Ok(())
}

/// filesystem layout of the meta-distro. the root is chosen at runtime so a
/// fork or test harness can place it anywhere
#[derive(Clone, Debug)]
pub struct Layout {
    root: PathBuf,
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Layout { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// tmpfs for runtime state, set up by early_mounts
    pub fn run(&self) -> PathBuf {
        self.root.join("run")
    }

    /// persisted namespace files live at run/ns/<layer>/mnt
    pub fn run_ns(&self) -> PathBuf {
        self.root.join("run/ns")
    }

    pub fn ns_file(&self, layer: &str) -> PathBuf {
        self.run_ns().join(layer).join("mnt")
    }

    /// staging root where a layer is composed before pivot
    pub fn stage(&self) -> PathBuf {
        self.root.join("run/root")
    }

    /// content-addressed object store
    pub fn store(&self) -> PathBuf {
        self.root.join(".store")
    }

    /// writable per-layer state (overlay upper/work)
    pub fn state(&self) -> PathBuf {
        self.root.join(".state")
    }

    /// gens for rollback
    pub fn gens(&self) -> PathBuf {
        self.root.join(".gen")
    }
}

impl Default for Layout {
    // the conventional root. change this to relocate the layout
    fn default() -> Self {
        Layout::new("/rust")
    }
}

// the calling thread's mount namespace link, bound to persist a layer
pub const SELF_NS_MNT: &str = "/proc/self/ns/mnt";

// the unified cgroup v2 hierarchy. absolute, like the pseudo-filesystems:
// the kernel mounts it here regardless of the meta-distro root
pub const CGROUP2_ROOT: &str = "/sys/fs/cgroup";
