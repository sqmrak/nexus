# run: the hot path

every process launch that is not pid 1 takes the run path. it is built to be
short: the namespace is already composed and pinned, so entering it is one
syscall, and confinement is applied in a fixed order that survives `execve`.
there is no daemon round trip. the sequence lives in `Core::run` and
`exec/launch.rs`:

```
ensure (compose if cold) > enter_scope > setns > no_new_privs > landlock > drop caps > seccomp > execve
```

## Core::run

```rust
let id = nexus::LayerId::new("void")?;
core.run(&id, &["/usr/bin/vlc".into()], &[])?;
```

`run` looks up the descriptor, calls `Registry::ensure` to get a built `Layer`
(composing and pinning it if this is the first use), enters the layer's cgroup
scope, then launches.

the scope is entered before launch so the cgroup limits survive `setns` and
`execve`: the child inherits the leaf cgroup it is moved into. `launch` only
returns on failure, so on a failed launch `run` exits the scope it entered,
leaving no orphaned half-applied cgroup behind. on success the process image is
replaced and `run` never returns (its type is `Infallible`).

## SetnsExec: the default launch strategy

`launch` is the `LaunchStrategy` hook. the default, `SetnsExec`, does the
minimum:

```rust
exec::enter(layer)?;              // open ns_path, setns
crate::sys::sandbox::apply(&layer.desc.sandbox)?;
Command::new(prog)
    .args(args)
    .env_clear()                  // drop the caller's whole environment
    .envs(env.iter().cloned())
    .exec();                      // execve; only returns on failure
```

`env_clear` matters: without it the child inherits pid 1's entire environment,
leaking caller state across the sandbox boundary. the child gets exactly the
environment passed to `run`, nothing more.

a fork that wants lower launch latency (a zygote, a pre-forked worker) supplies
its own `LaunchStrategy` and never touches the rest of the path.

## enter: setns

`exec::enter` is the whole namespace entry:

```rust
let fd = File::open(&layer.ns_path)?; // the pinned namespace file
nsproc::setns(fd)?;                   // enter it
```

after this the thread is inside the layer's mount namespace. opening a pinned
file and one `setns` is the entire cost of entering a layer; this is the payoff
for composing once at build time.

## the sandbox order

`sys::sandbox::apply` runs after `setns` because the steps that drop privilege
must happen inside the layer, and it runs in a fixed order so each restriction
inherits across `execve`:

```
no_new_privs > landlock > drop caps > seccomp
```

- `no_new_privs` is unconditional. it stops any `execve` in this process or its
  children from gaining privilege (no setuid escalation), and landlock requires
  it.
- landlock installs the filesystem ruleset, but only if the sandbox names read
  or write paths. it targets the newest abi with best-effort compatibility, so
  every kernel gets the strongest rules it supports and an older kernel never
  fails the launch.
- capabilities are dropped to exactly the layer's `keep_caps` (empty drops
  every capability). this happens before the filter, so seccomp guards an
  already-unprivileged process.
- the seccomp baseline filter is installed last, only if the layer asks for
  it.

the full mechanics of each step, including the landlock abi and the seccomp
deny list, are in [the sandbox](../subsystems/sandbox.md).

## what a cold first run does

if the layer's namespace is not built yet, `ensure` runs the full compose and
pin (see [namespaces](../model/namespaces.md)) before the path above continues.
that cost is paid once; subsequent runs of any command in the same layer skip
straight to `enter_scope > setns`. the control daemon's `build` command exists to
pay that cost ahead of time so the first user-visible launch is already hot; see
[the control daemon](../subsystems/control.md).
