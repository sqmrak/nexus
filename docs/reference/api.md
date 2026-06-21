# public api

everything in the `api` module is semver-stable; everything outside it is an
implementation detail a fork may replace. this chapter is a lookup list for the
public surface. for the generated rustdoc, run `cargo doc --open` against the
source tree.

## the facade

`Core` is the object policy drives. construct it, then call one operation.

- `Core::open(layout, layers, system)` probes kernel features while
  single-threaded, picks the backend named in `nexus.toml`, and enables cgroup
  controllers best-effort.
- `Core::new(layout, layers, backend, system)` constructs with an explicit
  backend instead of one chosen from config.

booting and running:

- `boot(native_init)` boots with the default selector and health check.
- `boot_with(selector, health, native_init)` boots with custom policy.
- `run(id, argv, env)` launches a command in a layer; its return type is
  `Infallible` because success replaces the process.

managing built namespaces (for the daemon):

- `warm(id)` builds and pins a layer's namespace ahead of use.
- `evict(id)` drops a built namespace and its resource scope.
- `evict_idle()` evicts namespaces idle past the configured window.
- `built_layers()` and `built_count()` list and count what is built.

store and generations:

- `verify_store()` checks every object against its name hash.
- `commit(hashes)` records a generation without switching to it.
- `activate_gen(g)` makes a generation live, atomically.
- `current_gen()` reads the active generation.
- `gc()` drops store objects no generation pins.

`Layout::new(root)` fixes the on-disk layout; see
[filesystem layout](layout.md).

## layer types

- `LayerId(String)` is a validated id: `new`, `as_str`, plus `FromStr` and
  `Display`. it rejects empty, slash, NUL, `.` and `..`.
- `LayerType` is `Native`, `Shadowed` or `Rescue`, the layer's role at boot.
- `LayerDescriptor` is the configured identity: `id`, `r#type`, `priority`,
  `libc`, `flags`, `sandbox`, `resources`.
- `Layer` is a descriptor whose namespace is built: `desc` and `ns_path`.
- `LayerFlags` is `meta`, `non_fhs`, `atomic`, `ephemeral`, `pinned`, `hidden`,
  `extra`, with helpers `no_persistent_upper()` and `ephemeral_upper()`.
- `Libc` is `name` and an optional `loader`, with `is_static()`.

## confinement and resources

- `Sandbox` is `read`, `write`, `seccomp`, `idmap`, `keep_caps`, with
  `is_empty()`.
- `Seccomp` is `Off` (default) or `Baseline`.
- `Cap(u8)` is a kernel capability number: `from_name(name)` and `raw()`.
- `IdMap` is `outer_start` and `count`; `new` rejects a zero count or a range
  that overflows the u32 id space.
- `Limits` is `memory_max`, `pids_max`, `cpu_weight`, `cpu_max`, with
  `is_empty()`.
- `CpuMax` is `quota_us` and `period_us`; `new` rejects zero for either.

## store and generations

- `ObjectHash(String)` is a blake3 tree hash: `new`, `as_str`, `Display`.
- `Gen(u64)` is a monotonic generation number, `Copy` and `Ord`.
- `Mounts` records mount points in creation order: `record(point)` and
  `teardown_order()` (the reverse).

## hook traits

each trait has a working default, so the core runs without a fork supplying any.

- `LayerSelector::select(candidates, health, store_root)`. default
  `DefaultSelector`: the healthy layer with the lowest priority, rescue last.
- `HealthCheck::is_healthy(layer, root)`. default `DefaultHealthCheck`: the
  layer's loader exists inside its own tree.
- `StoreBackend::mount_root` / `unmount_root`. defaults `OverlayBackend` and
  `ErofsBackend`.
- `LaunchStrategy::launch(layer, argv, env)`. default `SetnsExec`: setns, apply
  the sandbox, execve.

## loaders and probes

- `load_layers(path)` parses `layers.toml` into `Vec<LayerDescriptor>`.
- `load_system(path)` parses `nexus.toml` into `System`.
- `idmap_usable()` reports whether idmapped mounts actually apply here (cached).
- `cgroup_usable()` reports whether cgroup v2 is mounted (cached).

errors are `Error` and `Result<T>`; see [errors](errors.md).
