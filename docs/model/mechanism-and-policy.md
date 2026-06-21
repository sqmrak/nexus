# mechanism and policy

nexus is a library, not a binary. a meta-distribution links against it, calls
`Core::open`, and the core does the rest. the public surface is the `api`
module; everything else is an implementation detail a fork may replace.

## the boundary

mechanism is what nexus does to the kernel: compose a layer's mount namespace,
pin it, enter it, confine the process, exec. it is fixed, auditable, and free
of distro knowledge.

policy is every decision about which mechanism to invoke and when: which layer
a command resolves to, which layer boots when several are healthy, what
"healthy" means, how a layer root is mounted, how a process is launched. policy
is supplied by the meta-distribution through four hook traits.

```
policy (a fork)                 mechanism (nexus)
-----------------               -----------------
LayerSelector    drives  >  select a layer at boot
HealthCheck      drives  >  decide a layer is bootable
StoreBackend     drives  >  compose a layer root from the store
LaunchStrategy   drives  >  enter a layer and exec
```

each trait ships a working default, so nexus runs out of the box. a fork
overrides only the traits whose decisions it wants to own.

## why a trait and not a flag

a flag enumerates the choices the core author imagined. a trait lets a fork
make a choice the core author never imagined, without editing the core. the
rule across the codebase is the same: prefer a trait impl or a data entry over
a new branch. when a decision depends on "which distribution", it is a hook,
not a `match`.

this is why layer behaviour is carried as data on a descriptor (flags, libc
identity, sandbox, resource limits) rather than inferred from a distro name.
adding a distro is adding a descriptor. adding a delivery method is adding a
`StoreBackend`. nothing in nexus says the word for any distribution.

## the facade

`Core` is the single object policy drives. it holds the layout, the loaded
layer descriptors, the chosen backend, the system settings, the namespace
registry, the generation store and the cgroup controller.

```rust
use nexus::{Core, Layout};

let layout = Layout::new("/rust");
let layers = nexus::load_layers(&layout.state().join("layers.toml"))?;
let system = nexus::load_system(&layout.state().join("nexus.toml"))?;
let mut core = Core::open(layout, layers, system);
```

`Core::open` resolves every kernel-feature probe while the process is still
single-threaded (probing forks, and fork in a multi-threaded process is unsafe;
see [kernel features and probing](../subsystems/probing.md)), then selects the
backend named in `nexus.toml`. an unrecognised backend token falls back to
overlay. it then enables the cgroup v2 controllers best-effort: if that fails,
limits are simply not enforced, the system still runs.

the operations on `Core` map one to one onto the things a meta-distro asks for:

- `boot(native_init)` / `boot_with(selector, health, native_init)` > run as
  pid 1, select a layer, hand off to its init.
- `run(id, argv, env)` > launch a command inside a layer.
- `warm(id)` / `evict(id)` / `evict_idle()` > manage built namespaces, for the
  daemon.
- `commit(hashes)` / `activate_gen(g)` / `current_gen()` / `gc()` > manage
  generations and reclaim the store.
- `verify_store()` > check every object against its name hash.

every other type in this book exists to serve one of those calls.
