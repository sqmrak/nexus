# namespaces

a layer runs in its own mount namespace. nexus builds that namespace once,
pins it to a file on disk, and from then on entering the layer is a single
`setns` on the pinned file. no daemon sits in the path. the implementation
spans `ns/build.rs`, `ns/pin.rs` and `ns/reg.rs`.

## why a separate thread builds it

composing a namespace means `unshare(CLONE_NEWNS)` and `pivot_root`. those
mutate the calling thread's mount namespace, and that change must not leak back
to the rest of the process. so the build runs on a dedicated scoped thread:
when the thread ends, its namespace membership ends with it, and the rest of
the process is untouched. the namespace survives only because it was pinned to
disk before the thread let go.

## compose and pivot

`compose_and_pivot` is the body that runs on the build thread:

```
unshare(CLONE_NEWNS)              new mount namespace for this thread
make_private("/")                 recursively, so layer mounts do not propagate
mkdir stage                       the staging root
[if idmap] open_idmap()           build a user namespace fd for the mapping
backend.mount_root(desc, stage)   compose the layer root, idmapping if asked
bind_globals(stage)               bind /home /dev /proc /sys /run/user /tmp /boot
mkdir stage/.oldroot
pivot_root(stage, .oldroot)       stage becomes /
unmount_detach("/.oldroot")       drop the old root, lazily
```

`make_private("/")` matters because systemd leaves `/` shared, and a shared
mount rejects being pinned with `EINVAL`. recursive private propagation also
keeps the layer's mounts from escaping into the caller's namespace.

`bind_globals` binds the shared paths from `paths::GLOBALS` (`/home`, `/dev`,
`/proc`, `/sys`, `/run/user`, `/tmp`, `/boot`) into the composed root. a global
that is absent in the source is skipped, not fatal, so the bind path works
across different caller and guest profiles.

## why pinning is deferred to the parent

the kernel rejects binding a namespace onto a path inside that same namespace
with `EINVAL`. so the build thread cannot pin itself. instead:

```
1. parent: pin::prepare(ns_path)               before spawning the build thread
2. build:  compose_and_pivot, then send its tid to the parent
3. parent: recv tid, then pin_tid(tid, ns_path) binds /proc/<tid>/ns/mnt
4. parent: signal the build thread to continue (go)
5. build:  unblock on go, return (Layer, Mounts)
```

the build thread reports its kernel thread id and then blocks. the parent,
which is in a different mount namespace, binds `/proc/<tid>/ns/mnt` onto the pin
path, then releases the build thread. the namespace now has a reference (the
bind mount) that outlives the thread.

`pin::prepare` sets up the pin target on a private self-bind mount so it has no
peers, again because a shared mount rejects pinning with `EINVAL`. it also
best-effort unmounts any prior crashed pin at that path first.

## the registry

`Registry` holds built namespaces and is lazy: a layer is built on first use,
pinned, then kept. `ensure(layout, desc, backend)` returns a cached `Layer` if
the namespace is already built (updating its last-used time), otherwise it runs
the build-and-pin dance above and records the result.

each entry remembers the `Mounts` receipt the backend produced, replayed in
reverse at eviction, and the time it was last used.

## eviction

to bound memory, idle namespaces are dropped and rebuilt on next use.

- `evict_idle(max_idle, backend)` tears down every entry idle at least
  `max_idle`, driven by `nexus.toml`'s `idle_evict_secs`.
- `evict(id, backend)` drops one layer by id.

teardown drops the pin first so the kernel reclaims the namespace, then reverses
the backend mounts in `Mounts::teardown_order` (the reverse of creation, so an
overlay comes down before the lower it was stacked on). detaching the pin
unmounts the namespace file and then the private mount covering its directory,
so repeated evict and rebuild cycles do not stack mounts. all of teardown is
best-effort: a stale empty mount is swept later, not treated as fatal.
