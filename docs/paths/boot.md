# boot: pid 1

when nexus runs as pid 1 it owns the whole sequence from a bare kernel to a
running native init. every step must succeed or the system does not come up, so
the order is chosen to keep the machine recoverable even when layers are broken.
the sequence lives in `Core::boot_with`:

```
block_signals > Reaper::spawn > early_mounts > select > compose > enter_scope > handoff
```

`boot(native_init)` is the default-policy entry point; it calls `boot_with`
with `DefaultSelector` and `DefaultHealthCheck`. a fork supplies its own
selector and health check to `boot_with`.

## block_signals

signals are masked first. an unhandled `SIGTERM`, `SIGINT` or `SIGHUP` to pid 1
kills init, and a dead init panics the kernel. so before anything can spawn a
child or open a file, every signal is blocked except `SIGCHLD`.

`SIGCHLD` stays unblocked because pid 1 must reap the children it adopts.
`SIGKILL` and `SIGSTOP` are unblockable by kernel fiat. the mask survives
`execve`, so it is cleared again in the same process that execs the native init
(see handoff), leaving the real init a clean mask.

## Reaper::spawn

a dedicated thread reaps orphans during the boot window. kernel-spawned and
adopted children that exit between now and handoff would otherwise zombie under
pid 1. the reaper thread is small (a 16 KiB stack: it only calls `waitpid` and
sleeps) and named `nexus-reaper`. it polls `waitpid(-1, WNOHANG)` every 50 ms,
draining all exited children each sweep.

spawning it before `early_mounts` closes the window where an early child could
die unreaped. a failure to spawn the reaper is fatal: without it, orphans
zombie. the reaper is stopped at handoff (`reaper.stop()`), after which the
native init takes over reaping. stop joins the thread and drains once more,
because a child can die between the last sweep and the thread's exit.

## early_mounts

the pseudo-filesystems come up next. they do not depend on any layer, so they
succeed even if every layer is broken; this is what keeps the system bootable.
`early_mounts` runs out of initramfs before any layer is touched.

for each entry in `paths::PSEUDO` it mounts the filesystem through the new mount
api:

```
proc     > /proc
sysfs    > /sys
devtmpfs > /dev
```

each mount is `fsopen(fstype) > fsconfig_create > fsmount > move_mount(target)`.
it then creates `/run`, mounts a tmpfs there for runtime state, and creates
`/run/ns` where namespace pins will live.

## select

`select` picks the layer to boot. the default `DefaultSelector` filters the
candidates by the health check, resolving each layer's tree under the store
root, then sorts the survivors by `(is_rescue, priority)` and takes the
minimum.

```
keep only layers where health.is_healthy(layer, store_root/<id>)
sort by (type == Rescue, priority)
take the first
```

so the healthy non-rescue layer with the lowest priority wins. a rescue layer
sorts last and only wins when no native or shadowed layer is healthy. if nothing
is healthy, selection returns `NoHealthyLayer`.

the default `DefaultHealthCheck` calls a layer healthy when its libc loader
exists inside the layer's own tree. a static layer (no loader) is always
healthy. the loader path is resolved relative to the layer root, never the
caller, so a layer cannot pass by accident because the caller happens to have a
matching file. both of these are hooks; a fork can replace either. see
[hook traits](../build/hooks.md).

## compose

the selected layer's namespace is built and pinned through
`Registry::ensure`. on first boot this composes the layer root through the
backend, binds the globals, pivots, and pins the namespace to
`/run/ns/<id>/mnt`. see [namespaces](../model/namespaces.md) for the build and
pin mechanism and [storage backends](../subsystems/backends.md) for how the
root is mounted.

once the layer is composed the reaper is stopped: the namespace is up, and from
here the layer's own init owns the process tree.

## enter_scope

before handing off, the process enters the selected layer's cgroup v2 scope, so
the native init and everything it spawns run under the layer's resource
ceilings. this is a no-op when cgroup v2 is unavailable: the system runs
unconfined rather than failing to boot. see [cgroup v2 scopes](../subsystems/cgroups.md).

## handoff

handoff replaces nexus with the layer's native init.

```
reject an empty native_init path        (before crossing the ns boundary)
exec::enter(layer)                       open the pin file, setns into the layer
unblock_signals                          clear the mask the real init inherits
execve(native_init)                      replace the process image
```

the empty-path check happens before `setns` because `execve("")` would fail
with `ENOENT` after the namespace boundary has already been crossed, leaving
the process stranded inside the layer with no init. on success the process
image is the native init and `boot` never returns. on failure it returns an
error naming the step that broke.
