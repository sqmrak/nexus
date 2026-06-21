# storage backends

a backend is the filesystem strategy behind the content store. it composes a
layer's root at a target path and reverses that at eviction. the contract is the
`StoreBackend` trait; nexus ships two implementations, `OverlayBackend` and
`ErofsBackend`, selected by the `backend` token in `nexus.toml`.

## the StoreBackend contract

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

`mount_root` composes the layer's root at `target`. when `userns` is given, the
mount is idmapped before it is attached, so the layer's files appear under
remapped uids without chowning anything on disk. it returns a `Mounts` receipt:
the mount points it created, in creation order.

`unmount_root` reverses `mount_root` for an evicted layer. it is best-effort and
idempotent: it drops transient scratch but keeps persistent writes so a rebuild
is fast. it tears down in `Mounts::teardown_order`, the reverse of creation, so
an overlay comes down before the lower it was stacked on.

the trait is symmetric so teardown is clean, and a fork can hang its own
strategy (composefs, a network store) off the same seam without touching the
core.

## the new mount api

both backends build mounts through the new mount api (linux 5.2+), wrapped in
`sys/mount.rs`. the pattern is always the same:

```
fsopen(fstype)                 open a filesystem configuration context
fsconfig_string(fs, key, val)  set each option (lowerdir, upperdir, source, ...)
fsconfig_create(fs)            realise the superblock
fsmount(fs)                    get a detached mount fd
[idmap_mount(fd, userns)]      optionally apply the idmap to the detached tree
move_mount(fd, target)         attach it at the destination
```

opening the filesystem with `FSOPEN_CLOEXEC` and the mount with
`FSMOUNT_CLOEXEC` keeps the fds from leaking across `exec`. `move_mount` uses
`MOVE_MOUNT_F_EMPTY_PATH` so the source is named purely by the fd. idmapping a
detached mount before `move_mount` is why the new api is used here: the idmap is
applied while the tree is detached and unreachable, then the tree is attached in
one `move_mount` call.

## the writable upper, by flag

both backends decide the writable layer the same way, from the layer's flags:

- atomic > no upper. the mount is read only; changes land in a new generation,
  not on a writable overlay.
- ephemeral > a tmpfs upper. the writable layer lives in memory and is gone at
  reboot.
- neither > a persistent on-disk upper under the state root.

a tmpfs upper is one tmpfs mounted at the layer's ephemeral base, with `upper`
and `work` subdirectories created inside it (overlay requires upper and work on
the same filesystem). `LayerFlags::no_persistent_upper` and `ephemeral_upper`
encode the precedence; atomic wins if both atomic and ephemeral are set.

## OverlayBackend

the reference backend. it composes a layer by stacking an overlay:

```
lowerdir = <layer tree>[:<shared base>]
upperdir = ephemeral tmpfs | persistent on-disk | (none if atomic)
workdir  = same filesystem as upperdir
```

the lowerdir is the layer's own tree, optionally with a shared base appended
when a base directory exists and is not the layer's own directory. shadowed
layers carry a full rootfs; forge-built layers may share a base. the layer's own
tree comes first so it wins shadowing. a layer literally named `base` uses a
single lowerdir to avoid overlapping the same path twice (which overlay rejects
with `ELOOP`).

before configuring a persistent upper or work directory, the backend calls
`require_no_symlink_ancestor` on the path: `create_dir_all` follows symlinks, so
an attacker who planted a symlink ancestor could divert writable state outside
the tree. the check refuses that.

`unmount_root` reverses the mounts in teardown order, removes the ephemeral
tmpfs scratch if present, and keeps any persistent upper and work so a rebuild
does not lose writes.

## ErofsBackend

the erofs backend mounts a flat erofs image built ahead of time (at install or
forge time); the backend only mounts it, it does not build it.

```
mount the erofs image read-only at the per-layer lower mountpoint
overlay a writable upper over it (tmpfs / on-disk / none, by flag)
[idmap the overlay if a userns was given]
move the composed mount to target
```

the image lives at `<store>/<layer-id>.erofs`, mounted at
`<state>/lower/<layer-id>`. erofs is a compact read-only image, so the lower is
immutable by construction and only the overlay upper is writable. `unmount_root`
reclaims the lower mountpoint and any ephemeral scratch, and keeps the
persistent upper and work, exactly as overlay does.

both backends apply the optional idmap identically through `idmap_mount`, so the
choice of backend never changes how a layer's ids are remapped.
