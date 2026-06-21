// the registry daemon: a management socket over the core. the hot path
// never uses it; it warms, evicts and reports layers for tooling

mod proto;
mod serve;

pub use proto::{Reply, Request};
pub use serve::{install_signal_stop, listener, serve, Shutdown};
