# hook traits

a meta-distribution is policy bolted onto nexus through four traits. each has a
working default, so nexus runs without any of them; a fork implements only the
ones whose decisions it wants to own. the traits live in `api/hooks.rs` and are
part of the semver-stable public contract.

## opening a core

```rust
use nexus::{Core, Layout};

let layout = Layout::new("/rust");
let layers = nexus::load_layers(&layout.state().join("layers.toml"))?;
let system = nexus::load_system(&layout.state().join("nexus.toml"))?;
let mut core = Core::open(layout, layers, system);
```

`Core::open` resolves kernel-feature probes while single-threaded and selects
the backend named in `nexus.toml`. after that the core is ready to boot or run.

## LayerSelector

decides which layer boots.

```rust
pub trait LayerSelector {
    fn select(
        &self,
        candidates: &[LayerDescriptor],
        health: &dyn HealthCheck,
        store_root: &Path,
    ) -> Result<LayerId>;
}
```

`store_root` is the directory holding each layer's tree; it is passed so the
health check inspects real files. the default picks the healthy layer with the
lowest priority, with rescue layers as a last resort. return `NoHealthyLayer`
when nothing qualifies.

## HealthCheck

decides whether a layer is ready to boot.

```rust
pub trait HealthCheck {
    fn is_healthy(&self, layer: &LayerDescriptor, root: &Path) -> bool;
}
```

`root` is the layer's own tree, so the check inspects layer files, never the
caller's. the default returns true when the layer's libc loader exists inside
`root` (a static layer with no loader is always healthy).

## StoreBackend

the filesystem strategy behind the content store.

```rust
pub trait StoreBackend {
    fn mount_root(
        &self,
        layer: &LayerDescriptor,
        target: &Path,
        userns: Option<BorrowedFd<'_>>,
    ) -> Result<Mounts>;

    fn unmount_root(&self, layer: &LayerDescriptor, mounts: &Mounts);
}
```

`mount_root` composes the layer's root at `target`, idmapping the mount when a
userns is supplied, and returns a receipt of what it mounted. `unmount_root`
reverses it, best-effort and idempotent. the default ships `OverlayBackend` and
`ErofsBackend`; see [storage backends](../subsystems/backends.md).

## LaunchStrategy

enters a layer and execs a command.

```rust
pub trait LaunchStrategy {
    fn launch(
        &self,
        layer: &Layer,
        argv: &[String],
        env: &[(String, String)],
    ) -> Result<std::convert::Infallible>;
}
```

it returns `Infallible` because a successful launch replaces the process and
never comes back; only a failure returns. the default `SetnsExec` does
`setns > apply sandbox > execve`. a fork wanting lower launch latency (a zygote,
a pre-built worker pool) implements its own strategy here.

## where each trait fires

```
boot  > LayerSelector (uses HealthCheck) > StoreBackend (compose) > handoff
run   > StoreBackend (compose, if cold)  > LaunchStrategy
```

selector and health run only at boot. the backend runs on both paths, whenever a
layer's namespace must be composed. the launch strategy runs on the run path.
