# cgroup v2 scopes

resource limits are written to a layer's cgroup v2 scope at launch. limits are
data on the descriptor (`Limits`), so policy lives in config, not code. the
whole subsystem is best-effort: when cgroup v2 is unavailable, a layer runs
unconfined rather than failing to launch. the mechanism lives in
`sys/cgroup.rs`.

## the hierarchy

nexus works under a single subtree of the unified cgroup v2 root, conventionally
`/sys/fs/cgroup/nexus`. processes live only in leaves, one leaf per scope; the
subtree itself holds no processes, in keeping with the cgroup v2
no-internal-process rule.

```
/sys/fs/cgroup/                  the unified root
  nexus/                         the nexus subtree (delegates controllers down)
    void/    cgroup.procs        a leaf scope: a layer's processes live here
    arch/    cgroup.procs
```

## controllers

`prepare(want)` enables controllers for the subtree. it first delegates the
wanted controllers from the parent (a write that may legitimately fail in a
delegated subtree, and is ignored when it does), filters the request against
what is actually available, and writes the survivors into `cgroup.subtree_control`
in the `+cpu +memory +pids` form. it returns the controllers that are actually
usable. `Core::open` calls this best-effort for memory, pids and cpu; a failure
means limits are not enforced, not that the system cannot run.

## the Limits descriptor

```rust
pub struct Limits {
    pub memory_max: Option<u64>,  // memory.max in bytes; none leaves "max"
    pub pids_max: Option<u64>,    // pids.max as a task count; none leaves "max"
    pub cpu_weight: Option<u32>,  // cpu.weight, 1..=10000; none leaves the default
    pub cpu_max: Option<CpuMax>,  // cpu.max absolute quota; none leaves "max"
}
```

`Limits::is_empty` is true when no field is set. the scope is still created in
that case, for accounting, it just carries no ceilings.

`CpuMax` is an absolute cap: `quota_us` microseconds runnable per `period_us`
microseconds. `CpuMax::new` rejects zero for either value, because the kernel
would reject the `cpu.max` write with a cryptic error otherwise.

## scope lifecycle

the scope is driven from `Core`, around a launch:

```
create(scope)         mkdir the leaf
apply(scope, limits)  write each limit file that exists
enter(scope)          write our pid into cgroup.procs, before execve
... launch ...
[on failure] leave()  step back to the unified root
[on failure] remove() rmdir the now-empty leaf
```

`enter` moves the calling process into the leaf before `execve` so the child
inherits the cgroup. `apply` is best-effort per field: an interface file that is
absent (its controller is not enabled here) is skipped, not an error. `cpu.max`
is written as `quota_us period_us`; `cpu.weight` is clamped to `1..=10000`.

on a failed launch the process is still in the leaf, so `leave` steps it back to
the unified root, the one cgroup exempt from the no-internal-process rule, which
lets the now-empty leaf be removed. teardown is best-effort: a stale empty scope
is swept later, never fatal.

## boot and run

both paths enter the scope, for the same reason: the limits must survive `setns`
and `execve` so the launched program and its descendants run under them.

- on the run path, `run` enters the scope before launch and exits it if the
  launch fails.
- on the boot path, the selected layer's scope is entered before handoff, so
  the native init and everything it spawns run under the layer's ceilings.

## the cgroup_root override

`nexus.toml` may set `cgroup_root`. its purpose is testing: a harness points it
inside its own sandbox so a test run creates scopes there and never touches the
outer system's cgroup hierarchy. in production it defaults to
`/sys/fs/cgroup/nexus`.
