# the sandbox

confinement is per-layer data, not hardcoded policy. a layer with an empty
sandbox runs unconfined; every field is optional. the sandbox is applied on the
run path, after `setns` and before `execve`, in a fixed order so each
restriction inherits across the exec. the mechanism lives in `sys/sandbox.rs`,
`sys/caps.rs` and `sys/userns.rs`.

## the Sandbox descriptor

```rust
pub struct Sandbox {
    pub read: Vec<PathBuf>,      // subtrees the process may read/traverse (landlock)
    pub write: Vec<PathBuf>,     // subtrees it may write; write implies read
    pub seccomp: Seccomp,        // Off | Baseline
    pub idmap: Option<IdMap>,    // uid/gid remap for the layer's mounts
    pub keep_caps: Vec<Cap>,     // caps kept across execve; empty drops all
}
```

paths are real, so landlock applies to the real mounts a process sees, without
any rewriting. `Sandbox::is_empty` is true when nothing is configured, and the
launch then skips every confinement step.

## the order

```
no_new_privs > landlock > drop caps > seccomp
```

this order is deliberate:

- `no_new_privs` first, unconditionally. it sets `PR_SET_NO_NEW_PRIVS`, so no
  `execve` in this process or its children can gain privilege through a setuid
  binary. landlock requires it, and it is the floor under everything else.
- landlock next, only when read or write paths are set.
- capability drop next, so the seccomp filter is installed from an
  already-unprivileged state.
- seccomp last, only when the layer asks for the baseline.

## landlock

landlock confines the process to a set of filesystem subtrees. nexus targets
the newest abi it knows, `ABI::V7`, with `CompatLevel::BestEffort`. best-effort
means every kernel gets the strongest ruleset it actually supports, and an older
kernel that lacks newer access rights still gets the rules it can enforce
instead of failing the launch.

```
handle the full read-write access set of the target abi
for each read path:  add PathBeneath(path, read access)
for each write path: add PathBeneath(path, full access)   (write implies read)
restrict_self()
```

a write path grants read as well, so a layer lists a directory once under
`write` to get both. `restrict_self` applies the ruleset to the current thread,
which is then carried across `execve`.

## capabilities

`drop_to(keep)` reduces every capability set to exactly the caps in `keep`. an
empty list, the default, drops all of them. the order inside the drop matters:

```
1. drop the bounding set     (while CAP_SETPCAP is still effective; PR_CAPBSET_DROP needs it)
2. restrict effective, permitted and inheritable to keep
3. clear the ambient set, then re-raise only the kept caps
```

re-raising the kept caps in the ambient set is what makes them survive `execve`
into a binary that carries no file capabilities of its own. the count of
capabilities is read from `/proc/sys/kernel/cap_last_cap`, clamped to 63 because
the capability set is a u64 bitmask, and falls back to 40 (the linux 5.12 floor)
if that file is missing.

a `Cap` is just the kernel capability number, a `u8`. that number is a stable,
arch-independent abi, so config and syscall agree with no mapping table.
`Cap::from_name` parses the canonical lowercase name without the `CAP_` prefix,
for example `net_bind_service`, and returns `None` for an unknown name (which is
how `layers.toml` rejects a typo in `keep_caps`).

## seccomp baseline

the baseline is a deny-by-exception filter: allow everything, trap a small set
of universally dangerous syscalls with `EPERM`. the allowlist of a real profile
is policy; this is the core's safe default.

```
default action: Allow
matched action: Errno(1)   (EPERM)
denied:  mount, umount2, the new mount api
         (open_tree, move_mount, fsopen, fsconfig, fsmount, fspick, mount_setattr),
         kexec_load, kexec_file_load, init_module, finit_module, delete_module
```

these are the calls that reconfigure the process's mounts or load kernel code.
both the classic `mount(2)` and the new mount api (5.2+) are denied. the new
api is the path the kernel mounts through; blocking only the classic call
leaves a hole. the deny table carries one row per syscall
with both the x86_64 and aarch64 numbers, so the two architectures cannot drift
apart, and the right column is selected at compile time for the target arch. an
unsupported architecture yields no filter and the baseline reports it as an
error rather than silently running unfiltered.

## idmap

an idmapped mount makes a layer's files appear under a remapped uid/gid range
without chowning them on disk. it is configured by `IdMap`:

```rust
pub struct IdMap { outer_start: u32, count: u32 }
```

`IdMap::new` rejects a zero count and a range that would overflow the u32 id
space, because either would make the kernel reject the `uid_map`/`gid_map`
write.

building the mapping needs a user namespace fd. `sys/userns.rs` forks a child
that unshares `CLONE_NEWUSER` and blocks; the parent writes the child's
`uid_map`, denies `setgroups`, writes its `gid_map`, then opens
`/proc/<child>/ns/user` to get the namespace fd. a pipe synchronises the two so
the parent never writes the maps before the child has unshared. that fd is then
handed to `mount_setattr` with `MOUNT_ATTR_IDMAP` (see
[storage backends](backends.md)) to apply the mapping to the composed mount
while it is still detached.

the fork is safe because it runs only during single-threaded startup and the
child does async-signal-safe work (`unshare`, `read`, `_exit`) before the
parent reaps it.
