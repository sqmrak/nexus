use crate::api::{Error, Layer, LayerDescriptor, Mounts, Result, StoreBackend};
use crate::mount;
use crate::ns::pin;
use crate::paths::Layout;
use crate::sys::{nsproc, probe};
use rustix::thread::Pid;
use std::os::fd::AsFd;
use std::sync::mpsc::{Receiver, Sender};

// where the old root lands inside the new one, to be detached after pivot
const OLD_ROOT_NAME: &str = ".oldroot";

// must run its own thread: mount ns unshare/pivot must not leak to callers
// pinning deferred to parent; kernel rejects binding ns onto same-ns path (EINVAL)
pub fn build_namespace(
    layout: &Layout,
    backend: &dyn StoreBackend,
    desc: &LayerDescriptor,
    globals: &[String],
    tid_tx: &Sender<Result<Pid>>,
    go_rx: &Receiver<()>,
) -> Result<(Layer, Mounts)> {
    let mounts = match compose_and_pivot(layout, backend, desc, globals) {
        Ok(mounts) => {
            // hand the parent our tid so it can pin us from outside
            if tid_tx.send(Ok(rustix::thread::gettid())).is_err() {
                return Err(Error::Init("ns pin coordinator gone".into()));
            }
            mounts
        }
        Err(e) => {
            // unblock the parent with the failure rather than leaving it to
            // wait on a tid that will never arrive
            let _ = tid_tx.send(Err(e.clone()));
            return Err(e);
        }
    };

    // stay in the namespace until the parent has pinned it (or given up)
    let _ = go_rx.recv();

    let layer = Layer { desc: desc.clone(), ns_path: pin::ns_path(layout, &desc.id) };
    Ok((layer, mounts))
}

fn compose_and_pivot(
    layout: &Layout,
    backend: &dyn StoreBackend,
    desc: &LayerDescriptor,
    globals: &[String],
) -> Result<Mounts> {
    probe::require_mount_api()?;
    trace_field!("compose layer {}", desc.id);

    nsproc::unshare(nsproc::Unshare::NEWNS)?;
    nsproc::make_private("/")?;

    let stage = layout.stage();
    std::fs::create_dir_all(&stage).map_err(|e| Error::Init(format!("mkdir {stage:?}: {e}")))?;
    let stage_str = stage.display().to_string();

    // build a userns fd if the layer is idmapped; it lives until compose
    // has applied it to the mount
    let userns = match &desc.sandbox.idmap {
        Some(m) => {
            probe::require_idmapped()?;
            Some(crate::sys::userns::open_idmap(m)?)
        }
        None => None,
    };
    // mount the layer root at the stage through the chosen backend,
    // idmapping it when a userns is given
    let mounts = backend.mount_root(desc, &stage, userns.as_ref().map(|f| f.as_fd()))?;
    mount::bind_globals(&stage, globals)?;

    let old_root = stage.join(OLD_ROOT_NAME);
    std::fs::create_dir_all(&old_root).map_err(|e| Error::Init(format!("mkdir oldroot: {e}")))?;
    nsproc::pivot_root(&stage_str, &old_root.display().to_string())?;
    // after pivot the old root hangs at /<name>; detach and forget it
    nsproc::unmount_detach(&format!("/{OLD_ROOT_NAME}"))?;
    trace_step!("composed and pivoted into layer root");

    Ok(mounts)
}
