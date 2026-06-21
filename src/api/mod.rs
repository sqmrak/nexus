//! stable public contract. types and traits here are semver; everything
//! outside api is an implementation detail a fork may replace

mod err;
mod flags;
mod hooks;
mod layer;
mod libc;
mod mounts;
mod resources;
mod sandbox;
mod store;

pub use err::{Error, Result};
pub use flags::LayerFlags;
pub use hooks::{HealthCheck, LaunchStrategy, LayerSelector, StoreBackend};
pub use layer::{Layer, LayerDescriptor, LayerId, LayerType};
pub use mounts::Mounts;
pub use libc::Libc;
pub use resources::{CpuMax, Limits};
pub use sandbox::{Cap, IdMap, Sandbox, Seccomp};
pub use store::{Gen, ObjectHash};
