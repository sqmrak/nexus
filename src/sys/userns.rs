// build a user namespace fd for idmapped mounts. a child unshares
// CLONE_NEWUSER; the parent sets id maps, then opens /proc/<child>/ns/user

use crate::api::{Error, IdMap, Result};
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

pub fn open_idmap(map: &IdMap) -> Result<OwnedFd> {
    // sync pipe: child blocks until the parent has written its maps. owned
    // ends prevent fd leaks on fork failure or early return
    let mut fds = [0i32; 2];
    // safe: fds is a valid two-int buffer
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(crate::sys::errno_err("pipe"));
    }
    // safe: pipe() just handed us two fresh owned fds
    let rd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let wr = unsafe { OwnedFd::from_raw_fd(fds[1]) };

    // caller must be single-threaded ;  parent allocates after fork
    crate::sys::proc::assert_fork_safe();
    // safe: child is async-signal-safe (unshare, close, read, _exit)
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        // rd/wr drop here, closing both ends
        return Err(crate::sys::errno_err("fork"));
    }
    if pid == 0 {
        // never returns; the owned fds are not dropped (the child _exits)
        child(rd.as_raw_fd(), wr.as_raw_fd());
    }

    // parent: drop the read end, write the child's id maps, open its
    // userns, then release the child by dropping the write end
    drop(rd);
    let res = (|| {
        write_map(pid, "uid_map", map)?;
        // gid_map needs setgroups denied first on modern kernels
        deny_setgroups(pid)?;
        write_map(pid, "gid_map", map)?;
        open_userns(pid)
    })();
    drop(wr);
    reap(pid);
    res
}

fn child(rd: i32, wr: i32) -> ! {
    // safe: async-signal-safe calls only (unshare, read, _exit)
    unsafe {
        if libc::unshare(libc::CLONE_NEWUSER) != 0 {
            libc::_exit(1);
        }
        libc::close(wr);
        // block until the parent closes its write end
        let mut b = [0u8; 1];
        libc::read(rd, b.as_mut_ptr() as *mut _, 1);
        libc::_exit(0);
    }
}

fn write_map(pid: i32, file: &str, map: &IdMap) -> Result<()> {
    // "inside outside count": ids 0.. inside map to outer_start.. outside
    let line = format!("0 {} {}\n", map.outer_start(), map.count());
    std::fs::write(format!("/proc/{pid}/{file}"), line)
        .map_err(|e| Error::Io(format!("write {file}: {e}")))
}

fn deny_setgroups(pid: i32) -> Result<()> {
    let path = format!("/proc/{pid}/setgroups");
    match std::fs::write(&path, "deny") {
        Ok(()) => Ok(()),
        // the file appeared in linux 3.19; on older kernels there is nothing
        // to deny, so its absence is not an error
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::Io(format!("setgroups: {e}"))),
    }
}

fn open_userns(pid: i32) -> Result<OwnedFd> {
    File::open(format!("/proc/{pid}/ns/user"))
        .map(OwnedFd::from)
        .map_err(|e| Error::Io(format!("open userns: {e}")))
}

fn reap(pid: i32) {
    let mut status = 0;
    // safe: waitpid on our own child
    unsafe { libc::waitpid(pid, &mut status, 0) };
}
