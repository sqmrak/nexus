# worked examples

end to end uses of the core and its hooks. each overrides only what it needs and
leaves the rest of the mechanism untouched.

## boot as pid 1

```rust
core.boot("/sbin/init")?;
```

runs the full sequence `block_signals > reaper > early_mounts > select >
compose > enter_scope > handoff`. on success the process image is the native
init and the call never returns. on failure it returns an error naming the step
that broke.

## run a command in a layer

```rust
let id = nexus::LayerId::new("void")?;
core.run(&id, &["/usr/bin/vlc".into()], &[])?;
```

composes the layer if cold, enters its cgroup scope, applies the sandbox, and
execs. the empty env slice means the program starts with no inherited
environment; pass `&[("PATH".into(), "/usr/bin".into())]` to set it.

## a custom selector: rescue only

a recovery image that always boots the rescue layer, ignoring priority:

```rust
use nexus::{Error, HealthCheck, LayerDescriptor, LayerId, LayerSelector, LayerType, Result};
use std::path::Path;

struct RescueOnly;

impl LayerSelector for RescueOnly {
    fn select(
        &self,
        candidates: &[LayerDescriptor],
        health: &dyn HealthCheck,
        store_root: &Path,
    ) -> Result<LayerId> {
        candidates.iter()
            .filter(|l| l.r#type == LayerType::Rescue
                     && health.is_healthy(l, &store_root.join(l.id.as_str())))
            .min_by_key(|l| l.priority)
            .map(|l| l.id.clone())
            .ok_or(Error::NoHealthyLayer)
    }
}

core.boot_with(&RescueOnly, &nexus::DefaultHealthCheck, "/sbin/init")?;
```

note the health check is still consulted, and resolved against the layer's own
tree under `store_root`.

## a stricter health check

require the loader and an `/etc/os-release`, so a layer missing core files is
skipped even if its loader is present:

```rust
use nexus::{HealthCheck, LayerDescriptor};
use std::path::Path;

struct StrictHealth;

impl HealthCheck for StrictHealth {
    fn is_healthy(&self, layer: &LayerDescriptor, root: &Path) -> bool {
        let loader_ok = match &layer.libc.loader {
            Some(l) => root.join(l.trim_start_matches('/')).exists(),
            None => true, // static layer
        };
        loader_ok && root.join("etc/os-release").exists()
    }
}

core.boot_with(&nexus::DefaultSelector, &StrictHealth, "/sbin/init")?;
```

## a custom backend

implement `StoreBackend`. `mount_root` composes the layer root at `target` and
returns a `Mounts` receipt recording each mount point in creation order;
`unmount_root` reverses it in `teardown_order`. record every mount you make so
teardown can undo it:

```rust
fn mount_root(&self, layer: &LayerDescriptor, target: &Path,
              userns: Option<BorrowedFd<'_>>) -> Result<Mounts> {
    let mut m = Mounts::new();
    // ... compose at target ...
    m.record(target);
    Ok(m)
}
```

the default `OverlayBackend` and `ErofsBackend` are the reference for the
new-mount-api pattern and the upper-by-flag logic.

## warm a layer from the daemon

the control daemon turns a request into a core call. to warm a layer so its
first user-visible launch is hot:

```
$ printf 'build void\n' | nc -U /run/nexus.sock
ok

$ printf 'list\n' | nc -U /run/nexus.sock
void
```

`build` maps to `core.warm`, `list` to `core.built_layers`. see
[the control daemon](../subsystems/control.md) for the protocol.

## generations and rollback

```rust
let g = core.commit(&tree_hashes)?; // record a generation, does not switch
core.activate_gen(g)?;              // make it live (atomic symlink swap)

// later, roll back one generation
let now = core.current_gen()?;
core.activate_gen(nexus::Gen::new(now.get() - 1))?;

// reclaim store objects no generation pins
let removed = core.gc()?;
```
