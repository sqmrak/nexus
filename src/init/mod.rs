// pid 1 mechanism: early mounts, select a layer, hand off to its native
// init. select and health are the core defaults, overridable via hooks

mod early;
mod handoff;
mod health;
mod reaper;
mod select;
mod signals;

pub use early::{early_mounts, switch_root};
pub use handoff::handoff;
pub use health::DefaultHealthCheck;
pub use reaper::Reaper;
pub use select::DefaultSelector;
pub use signals::block_signals;
