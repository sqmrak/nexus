// extension seams so a fork changes policy without forking the code;
// each has a working default so the core runs out of the box

use super::err::Result;
use super::layer::{Layer, LayerDescriptor, LayerId};
use super::mounts::Mounts;
use std::os::fd::BorrowedFd;
use std::path::Path;

/// decide which layer boots. the default picks the healthy layer with the
/// lowest priority; replace to implement a custom boot policy
pub trait LayerSelector {
    /// return the id of the selected layer, or `NoHealthyLayer` if none is ready.
    /// `store_root` is the directory holding each layer's tree, passed to the
    /// health check so it inspects real files
    fn select(
        &self,
        candidates: &[LayerDescriptor],
        health: &dyn HealthCheck,
        store_root: &Path,
    ) -> Result<LayerId>;
}

/// decide whether a layer is healthy enough to boot. `root` is the layer's own
/// tree; the check inspects layer files, never the caller's
pub trait HealthCheck {
    /// true when the layer is ready to boot. the default checks that the loader
    /// exists inside the layer's tree
    fn is_healthy(&self, layer: &LayerDescriptor, root: &Path) -> bool;
}

/// enter a layer and exec a command. the default does setns + execve.
/// returns `Infallible` because a successful launch replaces the process
pub trait LaunchStrategy {
    fn launch(
        &self,
        layer: &Layer,
        argv: &[String],
        env: &[(String, String)],
    ) -> Result<std::convert::Infallible>;
}

/// filesystem strategy behind the content store. symmetric mount/unmount so
/// teardown is clean and a fork hangs its own unmount logic there
pub trait StoreBackend {
    /// compose the layer's root at `target`. `userns`, when given, idmaps the
    /// mount before it is attached. returns a receipt of what was mounted
    fn mount_root(
        &self,
        layer: &LayerDescriptor,
        target: &Path,
        userns: Option<BorrowedFd<'_>>,
    ) -> Result<Mounts>;

    /// reverse mount_root for an evicted layer. best-effort and idempotent;
    /// drops transient scratch but keeps persistent writes for a rebuild
    fn unmount_root(&self, layer: &LayerDescriptor, mounts: &Mounts);
}
