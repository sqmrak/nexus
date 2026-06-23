// the facade policy drives. holds layers, backend and the ns registry,
// and exposes the two operations: boot a layer, run a command in one

use crate::api::{
    Error, Gen, HealthCheck, LaunchStrategy, LayerDescriptor, LayerId, LayerSelector, ObjectHash,
    Result, StoreBackend,
};
use crate::exec;
use crate::init::{DefaultHealthCheck, DefaultSelector, Reaper, block_signals, early_mounts};
use crate::ns::Registry;
use crate::paths::Layout;
use crate::state::System;
use crate::store::{ErofsBackend, Generations, OverlayBackend, Store};
use crate::sys::cgroup::Cgroups;

type Backend = Box<dyn StoreBackend + Sync>;

pub struct Core {
    layout: Layout,
    layers: Vec<LayerDescriptor>,
    backend: Backend,
    system: System,
    registry: Registry,
    gens: Generations,
    cgroups: Cgroups,
}

impl Core {
    pub fn new(
        layout: Layout,
        layers: Vec<LayerDescriptor>,
        backend: Backend,
        system: System,
    ) -> Self {
        let gens = Generations::new(layout.gens());
        let cgroups = Cgroups::new(system.cgroup_root.clone());
        Core { layout, layers, backend, system, registry: Registry::new(), gens, cgroups }
    }

    // resolve kernel-feature probes while single-threaded, then pick the
    // backend. unknown names fall back to overlay
    pub fn open(layout: Layout, layers: Vec<LayerDescriptor>, system: System) -> Self {
        // resolve every kernel-feature probe now, while single-threaded
        crate::sys::probe::warmup();
        let store = layout.store();
        let state = layout.state();
        let backend: Backend = match system.backend.as_str() {
            crate::vocab::BACKEND_EROFS => Box::new(ErofsBackend::new(&store, &state)),
            _ => Box::new(OverlayBackend::new(&store, &state)),
        };
        let core = Self::new(layout, layers, backend, system);
        // enable cgroup controllers best-effort: a failure means limits are
        // not enforced, not that the system cannot run
        if crate::sys::probe::cgroup_usable() {
            let want: Vec<_> = if core.system.cgroup_controllers.is_empty() {
                core.cgroups.all_available()
            } else {
                core.system.cgroup_controllers.clone()
            };
            let want_refs: Vec<&str> = want.iter().map(|s| s.as_str()).collect();
            let _ = core.cgroups.prepare(&want_refs);
        }
        core
    }

    fn find(&self, id: &LayerId) -> Result<&LayerDescriptor> {
        self.layers.iter().find(|l| &l.id == id).ok_or_else(|| Error::UnknownLayer(id.to_string()))
    }

    pub fn run(
        &mut self,
        id: &LayerId,
        argv: &[String],
        env: &[(String, String)],
    ) -> Result<std::convert::Infallible> {
        let desc = self.find(id)?.clone();
        let layer =
            self.registry.ensure(&self.layout, &desc, &*self.backend, &self.system.globals)?;
        // enter the layer's resource scope before launch so cgroup limits
        // survive setns and execve
        self.enter_scope(&desc)?;
        let res = exec::SetnsExec.launch(&layer, argv, env);
        // launch only returns on failure; drop the scope we entered so a
        // failed run leaves no orphaned, half-applied cgroup behind
        self.exit_scope(&desc);
        res
    }

    // create and apply the layer's cgroup scope. no-op when cgroup v2 is
    // unavailable: the program runs unconfined rather than failing to launch
    fn enter_scope(&self, desc: &LayerDescriptor) -> Result<()> {
        if !crate::sys::probe::cgroup_usable() {
            return Ok(());
        }
        let scope = desc.id.as_str();
        self.cgroups.create(scope)?;
        self.cgroups.apply(scope, &desc.resources)?;
        self.cgroups.enter(scope)
    }

    // roll back enter_scope after a failed launch. the process is still in the
    // leaf, so step out before removing it. best effort: a stale empty scope is
    // swept later, not fatal
    fn exit_scope(&self, desc: &LayerDescriptor) {
        if !crate::sys::probe::cgroup_usable() {
            return;
        }
        let _ = self.cgroups.leave();
        let _ = self.cgroups.remove(desc.id.as_str());
    }

    // verify every stored object matches its name hash. call before serving
    // so corruption is caught early, not mid-compose. read-only
    pub fn verify_store(&self) -> Result<usize> {
        Store::new(self.layout.store()).verify()
    }

    // build and pin a layer's namespace ahead of use. for the daemon
    pub fn build(&mut self, id: &str) -> Result<()> {
        // an invalid id cannot name a real layer, so it is simply unknown
        let id = LayerId::new(id).map_err(|_| Error::UnknownLayer(id.to_string()))?;
        let desc = self.find(&id)?.clone();
        self.registry.ensure(&self.layout, &desc, &*self.backend, &self.system.globals)?;
        Ok(())
    }

    // drop a built layer's namespace. a later run rebuilds it. an id that is
    // invalid (and so was never built) is a no-op
    pub fn evict(&mut self, id: &str) {
        if let Ok(id) = LayerId::new(id) {
            self.registry.evict(&id, &*self.backend);
            // drop the layer's resource scope too; a later run recreates it
            // best effort: an absent or still-populated scope is not fatal
            let _ = self.cgroups.remove(id.as_str());
        }
    }

    pub fn built_layers(&self) -> Vec<String> {
        self.registry.ids().map(|i| i.as_str().to_string()).collect()
    }

    pub fn built_count(&self) -> usize {
        self.registry.ids().count()
    }

    // select a layer and hand pid 1 to its native init. defaults are used
    // unless a policy passes its own selector and health check
    pub fn boot(&mut self, native_init: &str) -> Result<()> {
        self.boot_with(&DefaultSelector, &DefaultHealthCheck, native_init)
    }

    pub fn boot_with(
        &mut self,
        selector: &dyn LayerSelector,
        health: &dyn HealthCheck,
        native_init: &str,
    ) -> Result<()> {
        // as pid 1, block stray signals: an unhandled SIGTERM/SIGINT/SIGHUP
        // would kill init and panic the kernel
        block_signals()?;
        // reap kernel-spawned children during the boot window; they would
        // zombie under pid 1 otherwise. stopped at handoff
        let reaper = Reaper::spawn()?;
        early_mounts(&self.layout, &self.system.pseudo)?;
        let id = selector.select(&self.layers, health, &self.layout.store())?;
        let desc = self.find(&id)?.clone();
        let layer =
            self.registry.ensure(&self.layout, &desc, &*self.backend, &self.system.globals)?;
        reaper.stop();
        // the selected layer, including the native init we are about to exec,
        // runs under the layer's resource scope
        self.enter_scope(&desc)?;
        crate::init::handoff(&layer, native_init)
    }

    pub fn evict_idle(&mut self) -> usize {
        self.registry.evict_idle(self.system.idle_evict, &*self.backend)
    }

    // record a new gen from the active layer hashes. does not switch
    // to it; call activate_gen for that
    pub fn commit(&self, hashes: &[ObjectHash]) -> Result<Gen> {
        self.gens.commit(hashes)
    }

    // switch the active gen. rollback is just activating an older
    // one. atomic: a crash leaves the old or the new, never a half state
    pub fn activate_gen(&self, g: Gen) -> Result<()> {
        self.gens.activate(g)
    }

    pub fn current_gen(&self) -> Result<Gen> {
        self.gens.current()
    }

    // drop store objects no generation pins. gens keep old trees for rollback;
    // gc bounds the store by sweeping unreferenced ones
    pub fn gc(&self) -> Result<usize> {
        let mut live: Vec<ObjectHash> = Vec::new();
        for g in self.gens.all()? {
            live.extend(self.gens.trees(g)?);
        }
        Store::new(self.layout.store()).gc(&live)
    }
}
