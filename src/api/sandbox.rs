// data so confinement is per-layer config, not hardcoded; real paths so
// landlock and seccomp apply without rewriting policy

use std::path::PathBuf;

/// per-layer confinement applied at launch. a layer with an empty sandbox
/// runs unconfined; every field is optional
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sandbox {
    /// subtrees the process may read and traverse (landlock)
    pub read: Vec<PathBuf>,
    /// subtrees the process may write. write implies read (landlock)
    pub write: Vec<PathBuf>,
    /// seccomp posture for the process
    pub seccomp: Seccomp,
    /// uid/gid remap for the layer's mounts. none means no idmap
    pub idmap: Option<IdMap>,
    /// capabilities the launched process keeps across execve. empty (the
    /// default) means every capability is dropped at launch
    pub keep_caps: Vec<Cap>,
}

/// kernel capability number, stable ABI and arch-independent, so no mapping
/// layer is needed between config and the syscall
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cap(u8);

// canonical names (lowercase, no CAP_ prefix) to kernel numbers, 0..=40.
// the single source for parsing keep-lists and the bound on what exists
const CAP_TABLE: &[(&str, u8)] = &[
    ("chown", 0),
    ("dac_override", 1),
    ("dac_read_search", 2),
    ("fowner", 3),
    ("fsetid", 4),
    ("kill", 5),
    ("setgid", 6),
    ("setuid", 7),
    ("setpcap", 8),
    ("linux_immutable", 9),
    ("net_bind_service", 10),
    ("net_broadcast", 11),
    ("net_admin", 12),
    ("net_raw", 13),
    ("ipc_lock", 14),
    ("ipc_owner", 15),
    ("sys_module", 16),
    ("sys_rawio", 17),
    ("sys_chroot", 18),
    ("sys_ptrace", 19),
    ("sys_pacct", 20),
    ("sys_admin", 21),
    ("sys_boot", 22),
    ("sys_nice", 23),
    ("sys_resource", 24),
    ("sys_time", 25),
    ("sys_tty_config", 26),
    ("mknod", 27),
    ("lease", 28),
    ("audit_write", 29),
    ("audit_control", 30),
    ("setfcap", 31),
    ("mac_override", 32),
    ("mac_admin", 33),
    ("syslog", 34),
    ("wake_alarm", 35),
    ("block_suspend", 36),
    ("audit_read", 37),
    ("perfmon", 38),
    ("bpf", 39),
    ("checkpoint_restore", 40),
];

impl Cap {
    /// parse a capability by its canonical name (lowercase, no CAP_ prefix),
    /// e.g. "net_bind_service". None for an unknown name
    pub fn from_name(name: &str) -> Option<Cap> {
        CAP_TABLE.iter().find(|(n, _)| *n == name).map(|&(_, v)| Cap(v))
    }

    /// the kernel capability number
    pub fn raw(self) -> u8 {
        self.0
    }
}

/// the seccomp posture for a layer
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Seccomp {
    /// no filter installed
    #[default]
    Off,
    /// block a small set of universally dangerous syscalls
    Baseline,
}

/// a contiguous id range mapped through an idmapped mount, so a layer's files
/// appear under outer ids without chowning on disk. constructed only through
/// new(), which rejects an empty or overflowing range
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdMap {
    outer_start: u32,
    count: u32,
}

impl IdMap {
    /// a mapping must cover at least one id and must not overflow the u32
    /// id space, or the kernel would reject the uid_map/gid_map write
    pub fn new(outer_start: u32, count: u32) -> Result<Self, String> {
        if count == 0 {
            return Err("idmap count must be at least 1".into());
        }
        if outer_start.checked_add(count).is_none() {
            return Err(format!("idmap range {outer_start}+{count} overflows the id space"));
        }
        Ok(IdMap { outer_start, count })
    }

    pub fn outer_start(&self) -> u32 {
        self.outer_start
    }

    pub fn count(&self) -> u32 {
        self.count
    }
}

impl Sandbox {
    /// true when no confinement is configured: no landlock, no seccomp,
    /// no idmap, no caps kept
    pub fn is_empty(&self) -> bool {
        self.read.is_empty()
            && self.write.is_empty()
            && self.seccomp == Seccomp::Off
            && self.idmap.is_none()
            && self.keep_caps.is_empty()
    }
}
