# nexus

nexus is a runtime for composable meta-distributions. it is pure mechanism: no
fuse, no path rewriting, no per-distro branching. everything it does is built
from kernel primitives (mount namespaces, overlay, erofs, idmapped mounts,
landlock, seccomp, cgroup v2) and a content-addressed store keyed by blake3.

nexus does not know what a distribution is. it composes a layer's mount
namespace from the store, runs processes inside it, and can boot as pid 1.
which layer a command resolves to, where layers come from, which layer boots:
that is policy, and policy lives in a separate project that plugs in through
hook traits. this book documents the mechanism, the contract a policy builds
on, and the kernel behaviour each step relies on.

## the one split

the whole design rests on a single line, drawn once and kept sharp.

- mechanism: compose a layer and run a process in it. this is nexus.
- policy: which layer a command resolves to, where layers come from, which
  layer boots. this is a meta-distribution built on top, plugged in through
  the hook traits.

the kernel never grows distro knowledge. anything that asks "which
distribution" is a hook impl, never a branch in nexus. adding a distro or a
delivery method is adding data or a trait impl, never editing a central
switch.

## what it does

- store > import rootfs trees into a content-addressed pool keyed by blake3.
  files are deduplicated by hash. a tree is checked out as a hardlink forest.
- generations > atomic rollback by symlink swap. commit a set of tree hashes,
  activate a generation, roll back to any previous one.
- compose > mount a layer root from the store through overlay or erofs. bind
  the shared globals (/home, /proc, /tmp, ...). idmap the mount when a user
  namespace is supplied.
- run > open a persisted mount namespace, setns into it, drop privilege
  (no_new_privs, capabilities, landlock, seccomp), execve the target.
- boot > the real pid-1 sequence: block fatal signals, spawn an orphan reaper,
  bring up /proc /sys /dev, select a healthy layer by priority, compose it,
  hand off to its native init.
- sandbox > per-layer confinement: landlock filesystem rules, a seccomp
  baseline filter, capability bounding sets, cgroup v2 resource ceilings
  (memory, pids, cpu).
- control daemon > a unix socket that warms, evicts and lists layers for
  tooling. socket-activation friendly. the hot path never touches it.

## status

92 tests, clippy-clean, GPL-2.0. exercised against a live kernel in real pid
and mount namespaces, including the full pid-1 path:

```
block_signals > reaper > early_mounts > select > compose > enter_scope > handoff
```

## how to read this book

- the model chapters describe the durable nouns: layers, the store,
  generations, namespaces. read these first.
- the path chapters walk the two sequences end to end: boot (pid 1) and run
  (every other launch).
- the subsystem chapters go deep on storage, the sandbox, cgroups, kernel
  feature probing and the control daemon.
- building a distro covers the four hook traits and worked overrides.
- the reference chapters are lookup tables: configuration, the public api,
  errors, vocabulary tokens and the on-disk layout.

## layout of the source

```
src/api/      stable public contract (semver)
src/sys/      syscall wrappers, the only unsafe
src/store/    content-addressed objects, generations, backends
src/mount/    bind the shared globals into a composed root
src/ns/       build, pin and register namespaces
src/exec/     enter a layer and exec
src/init/     pid 1 mechanism, select and health defaults
src/control/  the management socket daemon
src/state/    parse layers.toml and nexus.toml
src/core.rs   the facade: open, boot, run, warm, evict, gc
```
