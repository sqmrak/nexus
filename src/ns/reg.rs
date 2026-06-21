use crate::api::{Error, Layer, LayerDescriptor, LayerId, Mounts, Result, StoreBackend};
use crate::ns::build::build_namespace;
use crate::ns::pin;
use crate::paths::Layout;
use crate::sys::nsproc;
use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Instant;

struct Entry {
    layer: Layer,
    // what the backend mounted, replayed in reverse at eviction
    mounts: Mounts,
    last_used: Instant,
}

// holds built namespaces. lazy: a layer is built on first use, pinned to
// disk, then kept. idle eviction unmounts the pin to bound memory
#[derive(Default)]
pub struct Registry {
    built: HashMap<LayerId, Entry>,
}

impl Registry {
    pub fn new() -> Self {
        Registry::default()
    }

    // build in a scoped thread, pin from the parent: the kernel rejects
    // binding a ns onto a path inside that same ns (EINVAL)
    pub fn ensure(
        &mut self,
        layout: &Layout,
        desc: &LayerDescriptor,
        backend: &(dyn StoreBackend + Sync),
    ) -> Result<Layer> {
        if let Some(e) = self.built.get_mut(&desc.id) {
            e.last_used = Instant::now();
            return Ok(e.layer.clone());
        }

        let ns_path = pin::ns_path(layout, &desc.id);
        // the bind target must exist before the build thread reports in
        pin::prepare(&ns_path)?;

        let (tid_tx, tid_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel();

        let (layer, mounts) = std::thread::scope(|s| -> Result<(Layer, Mounts)> {
            let handle = s.spawn(move || build_namespace(layout, backend, desc, &tid_tx, &go_rx));

            // pin the namespace then release the build thread regardless of outcome
            let pinned = match tid_rx.recv() {
                Ok(Ok(tid)) => pin::pin_tid(tid, &ns_path),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(Error::Init("ns build thread died before pinning".into())),
            };
            let _ = go_tx.send(());
            pinned?;

            handle.join().map_err(|_| Error::Init("ns build thread panicked".into()))?
        })?;

        self.built.insert(
            desc.id.clone(),
            Entry { layer: layer.clone(), mounts, last_used: Instant::now() },
        );
        Ok(layer)
    }

    // drop namespaces idle longer than max_idle, tearing each down. a
    // later ensure rebuilds them. returns how many were evicted
    pub fn evict_idle(
        &mut self,
        max_idle: std::time::Duration,
        backend: &(dyn StoreBackend + Sync),
    ) -> usize {
        let now = Instant::now();
        let stale: Vec<LayerId> = self
            .built
            .iter()
            .filter(|(_, e)| now.duration_since(e.last_used) >= max_idle)
            .map(|(id, _)| id.clone())
            .collect();

        for id in &stale {
            if let Some(e) = self.built.remove(id) {
                teardown(&e, backend);
            }
        }
        stale.len()
    }

    // drop one layer's namespace and backend mounts. a later ensure
    // rebuilds it
    pub fn evict(&mut self, id: &LayerId, backend: &(dyn StoreBackend + Sync)) {
        if let Some(e) = self.built.remove(id) {
            teardown(&e, backend);
        }
    }

    pub fn ids(&self) -> impl Iterator<Item = &LayerId> {
        self.built.keys()
    }
}

// drop the pin first so the namespace is reclaimed, then reverse backend
// mounts. best-effort throughout
fn teardown(entry: &Entry, backend: &(dyn StoreBackend + Sync)) {
    detach_pin(&entry.layer.ns_path);
    backend.unmount_root(&entry.layer.desc, &entry.mounts);
}

// detach the namespace file then the private mount covering its directory,
// so repeated evict/rebuild cycles do not stack mounts. best effort
fn detach_pin(ns_path: &std::path::Path) {
    let _ = nsproc::unmount_detach(&ns_path.display().to_string());
    if let Some(dir) = ns_path.parent() {
        let _ = nsproc::unmount_detach(&dir.display().to_string());
    }
}
