# layers

a layer is a donor root the core can compose and run. every layer is equal;
differences are data on the descriptor, never special cases in code. there is
no privileged host: the layer the meta-distro was installed onto is just
another layer, marked `shadowed`.

## identity

a layer is named by a `LayerId`: a validated string used as a directory name in
the store. it is constructed only through `LayerId::new`, which rejects an
empty string, a slash, a NUL byte, and the names `.` and `..`. those rules keep
an id from escaping its directory or naming the wrong path.

```rust
let id = nexus::LayerId::new("void")?;   // ok
let bad = nexus::LayerId::new("../etc"); // Err: contains a slash / dot-dot
```

## type

`LayerType` records a layer's role at boot. it is the only axis the selector
treats specially, and even then only as a tie-break.

- `native` > an independently installed layer, equal in privilege to every
  other native.
- `shadowed` > the former root, the layer the meta-distro was installed onto.
  exactly one, equal in privilege to native.
- `rescue` > a guaranteed-healthy fallback so the system is never unbootable.
  it only wins selection when nothing else is healthy.

## the descriptor

`LayerDescriptor` is a layer's configured identity, read from system state and
never inferred from a distro name. its fields:

```rust
pub struct LayerDescriptor {
    pub id: LayerId,
    pub r#type: LayerType,
    pub priority: u32,      // lower wins; shadowed is 1 at install, native 1+n
    pub libc: Libc,
    pub flags: LayerFlags,
    pub sandbox: Sandbox,
    pub resources: Limits,
}
```

- `priority` orders selection. lower number wins. at install the shadowed
  layer is priority 1 and natives count up from there.
- `libc` carries the c runtime as data, not an enum, so a new libc adds no
  branch. it is a free-form `name` ("glibc", "musl", "uclibc", "static") and
  an optional `loader` path inside the layer. a layer with no loader is
  static; `Libc::is_static()` reports that.
- `flags` are the typed behaviours the core understands, plus fork extras.
- `sandbox` is the confinement applied at launch. empty means unconfined.
- `resources` are the cgroup v2 ceilings written at launch. empty means a
  scope with accounting but no caps.

## flags

`LayerFlags` separates the flags the core acts on from the flags it does not
own. fork-defined flags ride in `extra` so the core stays ignorant of policy it
does not understand.

```rust
pub struct LayerFlags {
    pub meta: bool,       // an imported meta-distro stratum (bedrock)
    pub non_fhs: bool,    // non-standard layout (nixos, guix, gobolinux)
    pub atomic: bool,     // immutable, atomically updated (silverblue, microos)
    pub ephemeral: bool,  // lives until reboot
    pub pinned: bool,     // version frozen; policy, core ignores
    pub hidden: bool,     // not shown in launcher or PATH; policy, core ignores
    pub extra: Vec<String>, // fork-defined flags, carried verbatim
}
```

the two flags the store acts on are `atomic` and `ephemeral`, surfaced through
two helpers:

- `no_persistent_upper()` is true for atomic or ephemeral: in both cases there
  is no persistent on-disk upper, so the backend skips the writable overlay.
- `ephemeral_upper()` is true when the layer wants a writable upper in tmpfs,
  lost at reboot. atomic wins if both are set: an atomic layer takes no upper
  at all and its changes land in a new generation instead.

`pinned` and `hidden` are policy: the core stores and round-trips them but acts
on neither.

## a built layer

once a descriptor's namespace is composed and pinned to disk it becomes a
`Layer`:

```rust
pub struct Layer {
    pub desc: LayerDescriptor,
    pub ns_path: std::path::PathBuf, // persisted mount namespace file
}
```

`ns_path` is the persisted mount namespace. open it and `setns` to enter the
layer with no daemon round trip. that is the whole hot path; see
[run](../paths/run.md).
