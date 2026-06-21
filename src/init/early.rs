// must not depend on any layer because the system must come up even when
// every layer is broken; runs out of initramfs before layers are touched

use crate::api::{Error, Result};
use crate::paths::{self, Layout};
use crate::sys::{mount, nsproc};
use std::os::fd::AsFd;
use std::path::Path;

// mount the linux pseudo-filesystems and a tmpfs for runtime state. the
// content store is read from disk, not mounted here
pub fn early_mounts(layout: &Layout) -> Result<()> {
    for (fstype, target) in paths::PSEUDO {
        mount_pseudo(fstype, target)?;
    }
    let run = layout.run();
    paths::mkdir_all(&run)?;
    mount_pseudo(crate::vocab::FS_TMPFS, &run.display().to_string())?;
    paths::mkdir_all(&layout.run_ns())?;
    Ok(())
}

fn mount_pseudo(fstype: &str, target: &str) -> Result<()> {
    paths::mkdir_all(Path::new(target))?;
    let fs = mount::fsopen(fstype)?;
    mount::fsconfig_create(fs.as_fd())?;
    let mnt = mount::fsmount(fs.as_fd())?;
    mount::move_mount(mnt.as_fd(), target)
}

// pseudo-filesystems are carried across instead of remounted so live state
// (processes, mounts) isn't lost during the pivot
pub fn switch_root(new_root: &str) -> Result<()> {
    // move each pseudo-mount to its target under new_root so /proc, /sys
    // and /dev survive the pivot
    for (_, m) in paths::PSEUDO {
        let dst = Path::new(new_root).join(m.trim_start_matches('/'));
        paths::mkdir_all(&dst)?;
        nsproc::move_mount_path(m, &dst.display().to_string())?;
    }
    // move the runtime tmpfs across as well; the daemon lives on it
    let run_dst = Path::new(new_root).join("run");
    paths::mkdir_all(&run_dst)?;
    nsproc::move_mount_path("/run", &run_dst.display().to_string())?;

    // pivot_root makes new_root the new / and exposes the old root at
    // /<OLD_ROOT_NAME> for cleanup
    const OLD_ROOT_NAME: &str = ".oldroot";
    let old = Path::new(new_root).join(OLD_ROOT_NAME);
    std::fs::create_dir_all(&old).map_err(|e| Error::Init(format!("mkdir oldroot: {e}")))?;
    nsproc::pivot_root(new_root, &old.display().to_string())?;

    // chroot into the new root so ".." is bounded
    nsproc::chroot(".")?;
    std::env::set_current_dir("/").map_err(|e| Error::Init(format!("chdir /: {e}")))?;

    // detach the old initramfs root now that it is no longer reachable
    nsproc::unmount_detach(&format!("/{OLD_ROOT_NAME}"))?;
    std::fs::remove_dir(format!("/{OLD_ROOT_NAME}"))
        .map_err(|e| Error::Init(format!("rmdir oldroot: {e}")))
}
