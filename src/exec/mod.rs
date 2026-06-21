// enter a layer and exec. the hot path: open the persisted namespace,
// setns, execve. no daemon round trip

#[allow(clippy::module_inception)]
mod exec;
mod launch;

pub use exec::enter;
pub use launch::SetnsExec;
