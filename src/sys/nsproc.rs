// namespace and process syscalls over rustix

use crate::api::{Error, Result};
use rustix::mount::{MountPropagationFlags as Prop, UnmountFlags};
use rustix::thread::{LinkNameSpaceType, UnshareFlags};
use std::os::fd::BorrowedFd;

pub use rustix::thread::UnshareFlags as Unshare;

// unsafe in rustix: detaching namespaces can break shared libc state
// nexus calls it on a fresh thread before any such state exists
pub fn unshare(flags: UnshareFlags) -> Result<()> {
    // safe: thread holds no namespace-dependent state
    unsafe { rustix::thread::unshare_unsafe(flags) }.map_err(|e| Error::sys("unshare", e))
}

pub fn setns(fd: BorrowedFd<'_>) -> Result<()> {
    rustix::thread::move_into_link_name_space(fd, Some(LinkNameSpaceType::Mount))
        .map_err(|e| Error::sys("setns", e))
}

pub fn pivot_root(new_root: &str, put_old: &str) -> Result<()> {
    rustix::process::pivot_root(new_root, put_old).map_err(|e| Error::sys("pivot_root", e))
}

pub fn bind(src: &str, dst: &str, recursive: bool) -> Result<()> {
    let r = if recursive {
        rustix::mount::mount_bind_recursive(src, dst)
    } else {
        rustix::mount::mount_bind(src, dst)
    };
    r.map_err(|e| Error::sys("bind", e))
}

// private + recursive, so layer mounts do not propagate outward
pub fn make_private(target: &str) -> Result<()> {
    rustix::mount::mount_change(target, Prop::PRIVATE | Prop::REC)
        .map_err(|e| Error::sys("private", e))
}

pub fn unmount_detach(target: &str) -> Result<()> {
    rustix::mount::unmount(target, UnmountFlags::DETACH).map_err(|e| Error::sys("unmount", e))
}

pub fn move_mount_path(src: &str, dst: &str) -> Result<()> {
    rustix::mount::mount_move(src, dst).map_err(|e| Error::sys("mount_move", e))
}

pub fn chroot(path: &str) -> Result<()> {
    rustix::process::chroot(path).map_err(|e| Error::sys("chroot", e))
}

// block every signal except `keep`. pid 1 dying to SIGTERM/SIGHUP would
// panic the kernel
pub fn block_signals_except(keep: &[i32]) -> Result<()> {
    // safe: set is a valid sigset_t we fully initialize before use
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    if unsafe { libc::sigfillset(&mut set) } != 0 {
        return Err(crate::sys::errno_err("sigfillset"));
    }
    for &sig in keep {
        // safe: set is initialized, sig is a small valid signal number
        if unsafe { libc::sigdelset(&mut set, sig) } != 0 {
            return Err(crate::sys::errno_err("sigdelset"));
        }
    }
    // safe: set is a valid, fully-initialized sigset_t; oldset is null
    if unsafe { libc::sigprocmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) } != 0 {
        return Err(crate::sys::errno_err("sigprocmask"));
    }
    Ok(())
}

// reset the signal mask to empty just before exec. the mask survives
// execve, so this must run in the same process about to exec
pub fn unblock_all_signals() -> Result<()> {
    // safe: set is a valid sigset_t we empty before use
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    if unsafe { libc::sigemptyset(&mut set) } != 0 {
        return Err(crate::sys::errno_err("sigemptyset"));
    }
    // safe: set is a valid, fully-initialized sigset_t; oldset is null
    if unsafe { libc::sigprocmask(libc::SIG_SETMASK, &set, std::ptr::null_mut()) } != 0 {
        return Err(crate::sys::errno_err("sigprocmask"));
    }
    Ok(())
}
