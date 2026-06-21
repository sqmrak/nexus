// errors crossing the core boundary

use std::fmt;
use std::sync::Arc;

/// the single error type returned by every fallible operation in the public api.
/// variants carry a human-readable message; the variant itself is the stable tag
/// a policy layer matches on to decide between retry, fallback, or abort
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub enum Error {
    /// a failed syscall. errno is kept for cheap matching; source is the std
    /// io error wrapped in Arc so the variant stays Clone
    Sys { op: &'static str, errno: i32, source: Arc<std::io::Error> },
    /// the named layer was not found in the loaded descriptors
    UnknownLayer(String),
    /// every candidate is unhealthy; no layer can be selected for boot
    NoHealthyLayer,
    /// a store object failed its blake3 integrity check
    Corrupt { hash: String },
    /// toml parse error, invalid value, or unknown token in state files
    Config(String),
    /// filesystem operation (mkdir, open, write) that is not a syscall
    Io(String),
    /// boot-path error: handoff, early_mounts, reaper, exec
    Init(String),
    /// libc loader not found or wrong libc for a layer
    Libc(String),
}

impl Error {
    // tag a failed syscall with the op that failed
    pub(crate) fn sys(op: &'static str, e: rustix::io::Errno) -> Self {
        let errno = e.raw_os_error();
        Error::Sys { op, errno, source: Arc::new(std::io::Error::from_raw_os_error(errno)) }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Sys { op, errno, .. } => write!(f, "{op} failed: errno {errno}"),
            Error::UnknownLayer(n) => write!(f, "unknown layer: {n}"),
            Error::NoHealthyLayer => write!(f, "no healthy layer to select"),
            Error::Corrupt { hash } => write!(f, "corrupt object: {hash}"),
            Error::Config(m) => write!(f, "config: {m}"),
            Error::Io(m) => write!(f, "io: {m}"),
            Error::Init(m) => write!(f, "init: {m}"),
            Error::Libc(m) => write!(f, "libc: {m}"),
        }
    }
}

impl std::error::Error for Error {
    // string variants carry no structured cause, so they return no source;
    // Sys exposes the real errno so anyhow/log adapters see the OS error
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Sys { source, .. } => Some(&**source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn sys_error_exposes_os_source() {
        let e = Error::sys("open", rustix::io::Errno::NOENT);
        // the source is the real OS error, with the same raw errno
        let src = e.source().expect("Sys has a source");
        let io = src.downcast_ref::<std::io::Error>().expect("source is io::Error");
        assert_eq!(io.raw_os_error(), Some(libc::ENOENT));
    }

    #[test]
    fn string_errors_have_no_source() {
        assert!(Error::Config("boom".into()).source().is_none());
        assert!(Error::NoHealthyLayer.source().is_none());
        assert!(Error::Corrupt { hash: "x".into() }.source().is_none());
    }

    #[test]
    fn display_unchanged() {
        let e = Error::sys("mount", rustix::io::Errno::PERM);
        assert!(e.to_string().starts_with("mount failed: errno"));
    }
}
