# why nexus

## the problem with userspace composition

a meta-distribution lets one machine run programs from several donor distros at
once: a void binary next to an arch binary next to a nixos package. the hard
part is making each program see the rootfs it expects without giving every
program its own machine.

bedrock linux solves this in userspace. it runs a fuse daemon, crossfs, that
synthesises a merged filesystem. every `open()` and `readdir()` from a program
crosses into a userspace process, gets resolved, and crosses back. etcfs merges
`/etc` the same way. the costs are structural, not incidental:

- latency on every path lookup, because the kernel cannot resolve the path
  itself; it has to ask a userspace daemon.
- selinux and apparmor see the virtual fuse paths, not the real files, so mac
  labels do not match and policy breaks.
- `strace` and audit show the synthesised paths, not the backing files, so a
  trace does not name the real path a program opened.
- there is one pid 1 for the whole machine, shared across every stratum.

## composition before launch

nexus composes a mount namespace from the content-addressed store before the
process starts. the layout is fixed at compose time by stacking real mounts.
after that the kernel handles every `open()` through native vfs and the page
cache. there is no daemon in the lookup path.

```
bedrock:  open() > fuse > userspace daemon > real fs > back   (every call)
nexus:    compose once (mounts) > open() > native vfs         (every call)
```

because the paths a process sees are real kernel mounts, tools that resolve
paths see the backing files directly: selinux and apparmor match labels on the
real inodes, audit and `strace` record the real paths, and the page cache is
shared across layers that map the same object.

## a layer is a donor rootfs

a layer is a donor root the core can compose and run. its files live in the
store as blake3-addressed objects: one copy per unique file, however many
layers reference it. two layers that ship the same `libc.so.6` share one
object on disk, linked into both trees by hardlink.

generations are sets of tree hashes. the active generation is whichever one the
`current` symlink points at. rolling back is switching that symlink to an older
generation. the swap is a single atomic rename, so a crash leaves either the
old generation or the new one, never a half-written state.

## a broken layer is not a dead system

the pid-1 sequence runs against a real kernel:

```
block_signals > reaper > early_mounts > select > compose > enter_scope > handoff
```

any layer can be init. selection filters by health first, so a layer whose
loader is missing is skipped, and the next healthy layer by priority wins. a
rescue layer is selected only when no native or shadowed layer is healthy, so a
broken layer does not leave the system unbootable as long as one healthy layer
remains.

## the principles this enforces

- compose before launch, not on access. no userspace fs daemon in the
  `open()` path. paths stay real.
- kernel primitives only. mount namespaces, overlay or erofs, idmapped mounts,
  the new mount api, landlock, seccomp, cgroup v2.
- data, not branches. behaviour comes from descriptors and traits, never from
  a match over distro names.
- content-addressed store. layers are objects by content hash, shared and
  deduplicated. state is a generation, rollback is a symlink swap.
- a broken layer is not a dead system. the boot path never depends on any
  single layer.

## not goals

- no fuse or userspace path rewriting in the hot path.
- no per-distro special casing. a distro name in a conditional is modelled as
  a descriptor instead.
- no privileged host. layers are equal, none is special.
- no destructive install. anything that touches an existing system is
  reversible.
- not a package manager and not a distribution. nexus ships no kernel, no
  repo and no libc of its own.
- no hidden writers to system state. all writes to system state go through one
  code path.
