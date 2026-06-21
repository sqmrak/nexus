//! the meta-distribution engine. open a core from state on disk, compose
//! layer namespaces from a content-addressed store, and run processes inside
//! them or boot a native init as pid 1

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod api;
pub mod paths;
pub mod vocab;

// no-op without "trace" feature so the hot path stays allocation-free
// in release; declared before modules so every caller sees them
#[macro_use]
mod trace;

mod control;
mod core;
mod exec;
mod init;
mod mount;
mod ns;
mod state;
mod store;
mod sys;

#[cfg(test)]
mod tmp;

pub use api::*;
pub use control::{install_signal_stop, listener, serve, Reply, Request, Shutdown};
pub use init::{block_signals, early_mounts, switch_root, DefaultHealthCheck, DefaultSelector, Reaper};
pub use paths::Layout;
pub use state::{System, load_layers, load_system};
pub use store::{ErofsBackend, Gens, OverlayBackend, Store};

pub use sys::cgroup::Cgroups;

/// the facade policy drives. holds layers, backend, the namespace registry,
/// and exposes boot (pid 1) and run (setns + exec) operations
pub use core::Core;

/// whether idmapped mounts actually apply in this environment (not just
/// whether the kernel has the syscall). probes end to end; result cached
pub fn idmap_usable() -> bool {
    sys::probe::idmap_usable()
}

/// whether the unified cgroup v2 hierarchy is mounted, so resource limits can
/// be enforced. result cached
pub fn cgroup_usable() -> bool {
    sys::probe::cgroup_usable()
}
