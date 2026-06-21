# filesystem layout

`Layout` fixes where nexus keeps its state. the root is chosen at runtime
(`Layout::new(root)`, defaulting to `/rust`) so a fork or a test harness can
relocate the whole tree without touching code. the type lives in `paths.rs`.

## the tree

```
<root>/                 the meta-distro root (default /rust)
  store/                content-addressed object store
    objects/            objects, named <blake3-hex>.<octal-mode>
    objects/.lock       the exclusive import lock
    <tree-hash>/        a materialized tree (hardlink forest)
  state/                writable per-layer state
    upper/<layer>       overlay upper (persistent layers)
    work/<layer>        overlay work dir
    ephemeral/<layer>   tmpfs base for ephemeral upper/work
    lower/<layer>       erofs image mountpoint (erofs backend)
  .gen/                 generations
    <n>/layers          a generation's tree-hash manifest
    current -> <n>      the active generation symlink
  run/                  runtime tmpfs, brought up by early_mounts
    ns/<layer>/mnt      the pinned mount namespace file
  stage/                staging root where a layer is composed before pivot
```

## Layout methods

each returns a path under the root.

- `root()` - `<root>`, the meta-distro root.
- `store()` - `<root>/store`, the object store.
- `state()` - `<root>/state`, writable per-layer state and the toml files.
- `gens()` - `<root>/.gen`, generations for rollback.
- `run()` - `<root>/run`, the runtime tmpfs.
- `run_ns()` - `<root>/run/ns`, where namespace pins live.
- `ns_file(layer)` - `<root>/run/ns/<layer>/mnt`, one layer's pinned namespace.
- `stage()` - `<root>/stage`, the compose target before pivot.

`layers.toml` and `nexus.toml` are read from `state()`; see
[configuration](configuration.md).

## absolute paths

a few paths are absolute regardless of the meta-distro root, because the kernel
puts them there.

- `PSEUDO`: the pseudo-filesystems mounted in early userspace, as
  `(fstype, target)`: `(proc, /proc)`, `(sysfs, /sys)`, `(devtmpfs, /dev)`.
- `GLOBALS`: paths shared across all layers, bound in from outside at compose
  time: `/home`, `/dev`, `/proc`, `/sys`, `/run/user`, `/tmp`, `/boot`. a global
  that is absent is skipped at bind time.
- `CGROUP2_ROOT`: `/sys/fs/cgroup`, the unified cgroup v2 hierarchy.
- `SELF_NS_MNT`: `/proc/self/ns/mnt`, the calling thread's mount namespace
  link, bound to persist a layer.

## guarding the tree

two helpers in `paths.rs` keep writes inside the tree:

- `mkdir_all(p)` creates a directory and its parents, mapping a failure to a
  stateful error.
- `require_no_symlink_ancestor(p, root)` refuses a path if any ancestor is a
  symlink. `create_dir_all` follows symlinks, so without this check an attacker
  who planted a symlink ancestor could divert writable state outside the tree.
  the backends call it before creating a persistent upper or work directory.
