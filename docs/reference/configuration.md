# configuration

nexus reads two toml files from the state directory: `layers.toml` (the layer
descriptors) and `nexus.toml` (system settings). both are validated at load
time, so an invalid value is an error at startup, not a surprise mid-boot. the
parsing lives in `state/layer.rs` and `state/system.rs`. the public format is
insulated from the internal types by private deserialization structs, so the
format can evolve without breaking the api.

## layers.toml

one table per layer, keyed by id. libc is read as data, never branched on by
name.

```toml
[layer.void]
type = "native"          # native | shadowed | rescue
priority = 2             # u32, lower wins

# any other bool key is a flag; only true ones need to be present
non_fhs = false
atomic = false
ephemeral = false
pinned = false
hidden = false
meta = false

[layer.void.libc]
name = "glibc"           # free form: glibc, musl, uclibc, static
loader = "/lib/ld-linux-x86-64.so.2"   # optional; omit for a static layer

[layer.void.sandbox]     # optional; every field optional
read = ["/usr", "/etc"]
write = ["/home"]
seccomp = "baseline"     # off | baseline
keep_caps = ["net_bind_service"]   # canonical names, no CAP_ prefix

[layer.void.sandbox.idmap]   # optional
outer_start = 100000
count = 65536

[layer.void.resources]   # optional
memory_max = 1073741824  # bytes
pids_max = 256           # task count
cpu_weight = 100         # 1..=10000

[layer.void.resources.cpu_max]
quota_us = 50000
period_us = 100000
```

validation rules applied on load:

- `type` must be one of `native`, `shadowed`, `rescue`.
- every bool key other than the known flags is captured as a flag. only `true`
  values are kept; a stray `false` is dropped. unknown flag names round-trip
  through `LayerFlags::extra`, so a fork's own flags survive a load/save cycle.
- `seccomp` absent or `off` means `Seccomp::Off`; `baseline` means
  `Seccomp::Baseline`.
- `keep_caps` names are parsed by `Cap::from_name`; an unknown capability name
  is rejected.
- `idmap` is validated by `IdMap::new`: count must be at least 1 and the range
  must not overflow the u32 id space.
- `cpu_max` is validated by `CpuMax::new`: quota and period must each be at
  least 1 microsecond.
- the layer id is validated by `LayerId::new`: no empty, slash, NUL, `.` or
  `..`.

## nexus.toml

system settings the core acts on. defaults apply when a key is absent.

```toml
backend = "overlay"       # overlay | erofs
idle_evict_secs = 900     # evict a namespace after this long idle; 0 = never
cgroup_root = "/sys/fs/cgroup/nexus"   # test override point
```

- `backend` must be `overlay` or `erofs`. it is validated here even though
  `Core::open` falls back to overlay for an unknown token, because a silent
  fallback would otherwise run a typo on the wrong backend. it is data, not a
  build-time choice, so a fork switches backend without recompiling.
- `idle_evict_secs` is the idle window before a built namespace is torn down
  and rebuilt on next use. zero disables eviction.
- `cgroup_root` exists for tests: a harness points it inside its own sandbox so
  a run never touches the outer system's cgroup hierarchy. it defaults to
  `/sys/fs/cgroup/nexus`.

## defaults

`System::default` is `backend = overlay`, `idle_evict = 900s`,
`cgroup_root = /sys/fs/cgroup/nexus`. a missing `nexus.toml` is equivalent to
those defaults.
