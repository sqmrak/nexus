use crate::api::{Error, LayerId, Result};
use crate::paths::Layout;
use crate::sys::nsproc;
use rustix::thread::Pid;
use std::path::{Path, PathBuf};

// where a layer's mount namespace file lives
pub fn ns_path(layout: &Layout, layer: &LayerId) -> PathBuf {
    layout.ns_file(layer.as_str())
}

// create the pin target on a private mount so it has no peers
// shared mounts (systemd default) reject pinning with EINVAL
pub fn prepare(dst: &Path) -> Result<()> {
    let parent =
        dst.parent().ok_or_else(|| Error::Io(format!("ns path has no parent: {dst:?}")))?;
    std::fs::create_dir_all(parent).map_err(|e| Error::Io(format!("mkdir {parent:?}: {e}")))?;

    let parent_str = parent.display().to_string();
    // drop any mount a crashed prior run left here, so repeated prepares do
    // not stack self-binds. best effort: detaching a non-mount just fails
    let _ = nsproc::unmount_detach(&parent_str);
    nsproc::bind(&parent_str, &parent_str, false)?;
    nsproc::make_private(&parent_str)?;

    if !dst.exists() {
        std::fs::File::create(dst).map_err(|e| Error::Io(format!("touch {dst:?}: {e}")))?;
    }
    Ok(())
}

// bind /proc/<tid>/ns/mnt onto dst. must be called from a different mount
// ns: the kernel rejects pinning onto a path inside that same ns (EINVAL)
pub fn pin_tid(tid: Pid, dst: &Path) -> Result<()> {
    let src = format!("/proc/{}/ns/mnt", tid.as_raw_nonzero().get());
    nsproc::bind(&src, &dst.display().to_string(), false)
}
