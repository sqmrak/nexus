// kernel feature detection. probes resolve lazily; warmup() forces them
// during single-threaded startup: fork in a multi-threaded process is UB

use crate::api::{Error, Result};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug)]
pub struct Features {
    pub mount_api: bool,
    pub idmapped: bool,
    // the unified cgroup v2 hierarchy is mounted (not v1 or hybrid)
    pub cgroup2: bool,
}

static FEATURES: OnceLock<Features> = OnceLock::new();

// force every probe to resolve now, while the process is single-threaded
// call once from Core::open before spawning any thread
pub fn warmup() {
    features();
    idmap_usable();
}

pub fn features() -> Features {
    *FEATURES.get_or_init(probe)
}

// the mount api is mandatory: without it nothing can be composed. checked
// once before the first build
pub fn require_mount_api() -> Result<()> {
    if features().mount_api {
        Ok(())
    } else {
        Err(Error::Init("kernel lacks the mount api (need linux 5.2+)".into()))
    }
}

// idmapped mounts are needed only by layers that ask for an idmap
pub fn require_idmapped() -> Result<()> {
    if features().idmapped {
        Ok(())
    } else {
        Err(Error::Init("kernel lacks idmapped mounts (need linux 5.12+)".into()))
    }
}

// cgroup v2 is a feature, not a requirement: a layer without resource
// limits runs fine without it
pub fn cgroup_usable() -> bool {
    features().cgroup2
}

fn probe() -> Features {
    Features { mount_api: probe_mount_api(), idmapped: probe_idmapped(), cgroup2: probe_cgroup2() }
}

// the unified v2 root has cgroup.controllers; v1/hybrid do not. a plain stat,
// no fork needed
fn probe_cgroup2() -> bool {
    std::path::Path::new(crate::paths::CGROUP2_ROOT).join(crate::vocab::CG_CONTROLLERS).exists()
}

fn probe_mount_api() -> bool {
    let nosys = rustix::io::Errno::NOSYS.raw_os_error();
    match crate::sys::mount::fsopen(crate::vocab::FS_TMPFS) {
        Ok(_fd) => true,
        Err(crate::api::Error::Sys { errno, .. }) => errno != nosys,
        Err(_) => true,
    }
}

// call mount_setattr with a bad fd: ENOSYS means the syscall is absent
// (kernel below 5.12), any other errno means it is present
fn probe_idmapped() -> bool {
    crate::sys::mount::mount_setattr_supported()
}

// probes whether idmapped mounts actually apply (not just whether the
// syscall exists). runs in a fork so namespace state never leaks. cached
pub fn idmap_usable() -> bool {
    static USABLE: OnceLock<bool> = OnceLock::new();
    *USABLE.get_or_init(probe_idmap_usable)
}

fn probe_idmap_usable() -> bool {
    if !features().idmapped {
        return false;
    }
    // fork a probe child so namespace state does not leak; called during
    // single-threaded warmup so the child may allocate

    // safe: single-threaded, child does normal work then _exits
    crate::sys::proc::assert_fork_safe();
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return false;
    }
    if pid == 0 {
        let ok = probe_in_child();
        // safe: terminate the probe child with the verdict as its code
        unsafe { libc::_exit(if ok { 0 } else { 1 }) };
    }
    let mut status = 0;
    // safe: waitpid on our own child
    unsafe { libc::waitpid(pid, &mut status, 0) };
    libc_wifexited(status) && libc_wexitstatus(status) == 0
}

// runs in the probe child: make a private mount ns, build an idmap userns,
// and try to apply it to a detached tmpfs. returns whether the idmap took
fn probe_in_child() -> bool {
    use std::os::fd::AsFd;
    // idmapped mounts require operating in a private mount namespace
    if crate::sys::nsproc::unshare(crate::sys::nsproc::Unshare::NEWNS).is_err() {
        return false;
    }
    let _ = crate::sys::nsproc::make_private("/");

     // a temporary 1:1 id mapping; the values are arbitrary for the probe
    let map = match crate::api::IdMap::new(100_000, 1) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let userns = match crate::sys::userns::open_idmap(&map) {
        Ok(fd) => fd,
        Err(_) => return false,
    };
    let mnt = match crate::sys::mount::tmpfs_detached() {
        Ok(fd) => fd,
        Err(_) => return false,
    };
    crate::sys::mount::idmap_mount(mnt.as_fd(), userns.as_fd()).is_ok()
}

fn libc_wifexited(status: i32) -> bool {
    (status & 0x7f) == 0
}

fn libc_wexitstatus(status: i32) -> i32 {
    (status >> 8) & 0xff
}
