// the new mount api over rustix: just error mapping. 5.2+

use crate::api::{Error, Result};
use rustix::fs::CWD;
use rustix::mount::{FsMountFlags, FsOpenFlags, MountAttrFlags, MoveMountFlags};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};

pub fn fsopen(fstype: &str) -> Result<OwnedFd> {
    rustix::mount::fsopen(fstype, FsOpenFlags::FSOPEN_CLOEXEC).map_err(|e| Error::sys("fsopen", e))
}

pub fn fsconfig_string(fs: BorrowedFd<'_>, key: &str, value: &str) -> Result<()> {
    rustix::mount::fsconfig_set_string(fs, key, value).map_err(|e| Error::sys("fsconfig(set)", e))
}

pub fn fsconfig_create(fs: BorrowedFd<'_>) -> Result<()> {
    rustix::mount::fsconfig_create(fs).map_err(|e| Error::sys("fsconfig(create)", e))
}

pub fn fsmount(fs: BorrowedFd<'_>) -> Result<OwnedFd> {
    rustix::mount::fsmount(fs, FsMountFlags::FSMOUNT_CLOEXEC, MountAttrFlags::empty())
        .map_err(|e| Error::sys("fsmount", e))
}

pub fn move_mount(mount_fd: BorrowedFd<'_>, dst: &str) -> Result<()> {
    let flags = MoveMountFlags::MOVE_MOUNT_F_EMPTY_PATH;
    rustix::mount::move_mount(mount_fd.as_fd(), "", CWD, dst, flags)
        .map_err(|e| Error::sys("move_mount", e))
}

// mount a fresh tmpfs at dst. used for an ephemeral layer's writable upper,
// which lives in memory and is gone on reboot
pub fn mount_tmpfs(dst: &str) -> Result<()> {
    let fs = fsopen(crate::vocab::FS_TMPFS)?;
    fsconfig_create(fs.as_fd())?;
    let mnt = fsmount(fs.as_fd())?;
    move_mount(mnt.as_fd(), dst)
}

// a fresh detached tmpfs mount fd, not attached anywhere. for probing whether
// idmapped mounts actually apply on this kernel/environment
pub fn tmpfs_detached() -> Result<OwnedFd> {
    let fs = fsopen(crate::vocab::FS_TMPFS)?;
    fsconfig_create(fs.as_fd())?;
    fsmount(fs.as_fd())
}

// idmap a mount through a userns fd. rustix has the flag but no
// mount_setattr wrapper, so this is the one raw syscall here
const MOUNT_ATTR_IDMAP: u64 = 0x0010_0000;
const AT_RECURSIVE: i32 = 0x8000;

pub fn idmap_mount(tree: BorrowedFd<'_>, userns: BorrowedFd<'_>) -> Result<()> {
    #[repr(C)]
    struct MountAttr {
        attr_set: u64,
        attr_clr: u64,
        propagation: u64,
        userns_fd: u64,
    }
    let attr = MountAttr {
        attr_set: MOUNT_ATTR_IDMAP,
        attr_clr: 0,
        propagation: 0,
        userns_fd: userns.as_raw_fd() as u64,
    };
    let empty = c"";
    // safe: tree is a live fd, attr is a valid mount_attr of the given
    // size, path is empty with AT_EMPTY_PATH so only the fd is used
    let r = unsafe {
        libc::syscall(
            libc::SYS_mount_setattr,
            tree.as_raw_fd(),
            empty.as_ptr(),
            (AT_RECURSIVE | libc::AT_EMPTY_PATH) as u32,
            &attr as *const MountAttr,
            size_of::<MountAttr>(),
        )
    };
    // capture errno as the very next step, before the branch: nothing may
    // run between the syscall and this read or a signal could clobber it
    let e = super::errno();
    if r != 0 {
        return Err(Error::sys("mount_setattr", rustix::io::Errno::from_raw_os_error(e)));
    }
    Ok(())
}

// probe for mount_setattr (linux 5.12+). a bad fd returns EBADF when the
// syscall exists, ENOSYS when it does not
pub fn mount_setattr_supported() -> bool {
    let empty = c"";
    // safe: all-zero args and a null attr pointer with size 0; the kernel
    // rejects the call before dereferencing anything
    let r = unsafe {
        libc::syscall(
            libc::SYS_mount_setattr,
            -1,
            empty.as_ptr(),
            0u32,
            std::ptr::null::<u8>(),
            0usize,
        )
    };
    let e = super::errno();
    r == 0 || e != libc::ENOSYS
}
