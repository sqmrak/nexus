// the syscall layer, the only place unsafe lives. everything else calls
// these safe wrappers. first candidate to split into a nexus-sys crate

pub mod caps;
pub mod cgroup;
pub mod mount;
pub mod nsproc;
pub mod proc;
pub mod probe;
pub mod sandbox;
pub mod userns;

// must be the very next statement after a failed raw syscall because
// a signal handler could clobber the thread-local errno in between
pub(crate) fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

// tag the current errno with the op that failed. same timing rule as errno()
pub(crate) fn errno_err(op: &'static str) -> crate::api::Error {
    crate::api::Error::sys(op, rustix::io::Errno::from_raw_os_error(errno()))
}
