# kernel features and probing

nexus depends on kernel features that vary by version and environment: the new
mount api (5.2+), idmapped mounts (5.12+), and a mounted cgroup v2 hierarchy. it
detects what is present once, while single-threaded, and caches the result. the
mechanism lives in `sys/probe.rs`.

## why probing happens at open, single-threaded

one of the probes forks (see below), and `fork` in a multi-threaded process is
unsafe: the child inherits only the calling thread and any lock held by another
thread at the instant of fork is frozen, so the child can deadlock or hit
undefined behaviour. so `Core::open` calls `probe::warmup()` before it spawns
any thread, forcing every probe to run and cache its result while exactly one
thread exists. `sys/proc.rs` backs this with `assert_fork_safe`, which counts
threads via `/proc/self/stat` and panics on a fork attempt from a
multi-threaded process (relaxed in test builds, where the test harness is
itself multi-threaded).

## the features

```rust
pub struct Features {
    pub mount_api: bool,  // fsopen/fsconfig/fsmount available (5.2+)
    pub idmapped: bool,   // mount_setattr for idmapped mounts (5.12+)
    pub cgroup2: bool,    // the unified cgroup v2 hierarchy is mounted
}
```

- `mount_api` is probed by trying `fsopen` on tmpfs; `ENOSYS` means absent.
- `idmapped` is probed by calling `mount_setattr` with a bad fd and a null attr:
  the kernel rejects it before dereferencing anything, so `EBADF` means the
  syscall exists and `ENOSYS` means it does not.
- `cgroup2` is a plain stat for `cgroup.controllers` in the cgroup root.

`features()` returns the cached set. `require_mount_api` and `require_idmapped`
turn an absent feature into an error for the paths that need it; `cgroup_usable`
and `idmap_usable` are soft checks for features that degrade gracefully.

## probing whether idmap actually applies

a kernel can have the `mount_setattr` syscall yet still not apply an idmapped
mount in the current environment. so `idmap_usable` does an end-to-end probe, in
a fork so no namespace state leaks back into the parent:

```
child:
  unshare(CLONE_NEWNS)
  make "/" private
  build a throwaway idmap and open its userns fd
  create a detached tmpfs
  idmap_mount(tmpfs, userns)
  _exit(0 on success, 1 on failure)
parent:
  waitpid the child, success iff it exited 0
```

the child may allocate freely because the probe runs during single-threaded
warmup, so there is no frozen-lock hazard. the result is cached, so the cost is
paid once.

## degradation

each feature has a defined behaviour when absent:

- no mount api > the paths that require it error. nexus needs 5.2+ to compose at
  all.
- no idmapped mounts > a layer that asks for an idmap cannot be composed with
  one; layers without idmap are unaffected.
- no cgroup v2 > scopes are skipped and programs run unconfined, rather than
  failing to launch.

`Core::open` reflects this: it build  the probes, picks the backend, and enables
cgroup controllers only when cgroup v2 is usable.
