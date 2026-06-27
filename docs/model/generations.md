# generations and rollback

a generation is a numbered snapshot of which trees make up the system. it is
monotonic and never reused. rollback is just activating an older one. the
implementation lives in `store/generations.rs`; generations are stored under
the layout's `.gen` directory with a `current` symlink naming the active one.

## the on-disk shape

```
.gen/
  1/layers        a manifest: one tree hash per line
  2/layers
  3/layers
  current > 3     a symlink naming the active generation
```

each generation directory holds a `layers` file listing the tree hashes that
make up that generation, one `ObjectHash` per line. the `current` symlink's
target is the active generation number.

## commit then activate

recording a generation and switching to it are two steps, on purpose.

- `commit(hashes)` writes a new generation directory, numbered one past the
  highest existing one (or 1 if none), listing the given tree hashes. it does
  not change `current`. the allocate-and-write runs under an exclusive `flock`
  on the generation root: two concurrent commits never pick the same number.
  the manifest, its directory and the root are fsynced before commit returns.
- `activate(g)` switches `current` to point at generation `g`.

separating them means a meta-distro can build a generation, verify it, and only
then make it live. it also means rollback and roll-forward are the same
primitive: activate is the only thing that changes what boots.

## why the swap is atomic

`activate` does not rewrite the `current` symlink in place. it creates a
temporary symlink pointing at the target generation and renames it over
`current`. `rename(2)` over an existing path is atomic on a posix filesystem, so
at every instant `current` names exactly one generation: the old one or the new
one, never a half state. a crash during activation leaves a fully valid system
pointing at whichever generation won the race.

```rust
let g = core.commit(&tree_hashes)?; // record, does not switch
core.activate_gen(g)?;              // atomic symlink swap

let now = core.current_gen()?;      // read current
core.activate_gen(nexus::Gen::new(now.get() - 1))?; // roll back one
```

## generations as the gc root set

generations are also the liveness root for the store. a generation keeps its
trees alive for rollback; without one, an old tree would be swept the moment it
stopped being current. `Core::gc()` reads every generation's tree list, unions
them into the live set, and calls `Store::gc` with it. so the store is bounded
by how many generations you keep, and pruning old generations is what
ultimately lets gc reclaim their unique objects.

## the Gen and ObjectHash types

```rust
pub struct Gen(u64);          // monotonic generation number
pub struct ObjectHash(String); // a blake3 tree hash
```

`Gen` is `Copy` and ordered, so comparing and stepping generations is cheap.
`ObjectHash` is the name of a tree in the store; it is what `commit` records and
what gc treats as live.
