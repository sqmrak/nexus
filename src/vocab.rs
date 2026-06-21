// on-disk vocabulary a fork retunes in one place without touching code
// every string the state format and the core agree on lives here

// layer type tokens in layers.toml
pub const TYPE_NATIVE: &str = "native";
pub const TYPE_SHADOWED: &str = "shadowed";
pub const TYPE_RESCUE: &str = "rescue";

// seccomp posture tokens
pub const SECCOMP_OFF: &str = "off";
pub const SECCOMP_BASELINE: &str = "baseline";

// store backend tokens in nexus.toml
pub const BACKEND_OVERLAY: &str = "overlay";
// the erofs backend (flat erofs image plus a writable overlay)
pub const BACKEND_EROFS: &str = "erofs";

// layer flag tokens. the known set the core understands; anything else a
// fork writes is carried verbatim in LayerFlags::extra
pub const FLAG_META: &str = "meta";
pub const FLAG_NON_FHS: &str = "non-fhs";
pub const FLAG_ATOMIC: &str = "atomic";
pub const FLAG_EPHEMERAL: &str = "ephemeral";
pub const FLAG_PINNED: &str = "pinned";
pub const FLAG_HIDDEN: &str = "hidden";

// linux filesystem type names
pub const FS_OVERLAY: &str = "overlay";
pub const FS_EROFS: &str = "erofs";
pub const FS_TMPFS: &str = "tmpfs";

// overlay mount option keys
pub const OPT_LOWERDIR: &str = "lowerdir";
pub const OPT_UPPERDIR: &str = "upperdir";
pub const OPT_WORKDIR: &str = "workdir";
pub const OPT_SOURCE: &str = "source";

// the nexus subtree under the unified cgroup v2 root. leaves (one per
// scope) live beneath it; the subtree itself holds no processes
pub const CG_SUBTREE: &str = "nexus";

// cgroup v2 controller names the core may enable on its subtree
pub const CG_CTRL_MEMORY: &str = "memory";
pub const CG_CTRL_PIDS: &str = "pids";
pub const CG_CTRL_CPU: &str = "cpu";

// cgroup v2 interface files
pub const CG_CONTROLLERS: &str = "cgroup.controllers";
pub const CG_SUBTREE_CONTROL: &str = "cgroup.subtree_control";
pub const CG_PROCS: &str = "cgroup.procs";
pub const CG_MEMORY_MAX: &str = "memory.max";
pub const CG_PIDS_MAX: &str = "pids.max";
pub const CG_CPU_WEIGHT: &str = "cpu.weight";
pub const CG_CPU_MAX: &str = "cpu.max";
