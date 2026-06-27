# the content store

the store holds objects by content hash and trees by manifest hash. it is the
single source of truth for layer files. importing is idempotent: re-importing
identical content reuses the objects already there. the implementation lives in
`store/cas.rs`.

## objects and trees

- an object is the bytes of one regular file, named by its content. the on-disk
  object name is `<blake3-hex>.<octal-mode>`, so the same bytes with different
  permissions stay distinct objects.
- a tree is one rootfs: a directory of hardlinks to objects, plus the
  directories and symlinks that structure them. a tree is named by the blake3
  hash of its manifest, not of its bytes.

the manifest is a stable text listing, one line per entry, sorted by path:

```
d <octal-mode> <path>             a directory
f <octal-mode> <hash> <path>      a regular file, by object hash
l <target> <path>                 a symlink
```

the tree hash is `blake3(manifest)`. two imports that produce byte-identical
manifests produce the same tree hash and reuse the same tree directory. because
the manifest names files by object hash, the tree hash changes if any file's
content, mode, path, or any symlink target changes.

## import

`Store::import(src)` walks a source rootfs and returns its tree hash.

```
acquire the object lock (exclusive flock on objects/.lock)
walk src recursively:
  regular file > stream through blake3 in 64 KiB chunks, stage to a temp
                 object, then place it if absent (reuse if present)
  directory    > record a manifest entry (the root dir itself is skipped)
  symlink      > record the link target
sort entries by path
build the manifest text and hash it > tree hash
if the tree dir is absent, materialize it (hardlink objects, mkdir, symlink)
return the tree hash
```

three details matter for correctness:

- the object lock is an exclusive `flock` on `objects/.lock`, held while
  objects are placed. it serializes across processes, not just threads; two
  concurrent imports never write a torn duplicate.
- staged temp objects are named `.stage-<pid>-<nonce>`, the nonce an atomic
  per-process counter. concurrent imports never collide on a temp name. a
  `.stage-` file left by a crashed import is swept on the next import, under
  the lock.
- each object's content is fsynced before the rename that places it. the
  objects directory is fsynced before the tree hardlinks to it. the tree
  directory entry is fsynced after its rename. a power loss leaves a placed
  object whole or absent, never named with unwritten bytes.

materialization is atomic: the tree is built at a `.tmp` sibling and renamed
into place. a crash never leaves a partial tree. the sorted manifest lets the
materializer create parents before children with no `create_dir_all`.

## deduplication and hardlinks

a checked-out tree consists of hardlinks into the object pool. one unique file
is stored once however many trees reference it; each tree adds a link, not a
copy. so adding a layer costs disk only for the files it does not share with an
existing object.

`Store::checkout(tree, dst)` copies a tree to an arbitrary destination,
hardlinking files so it stays cheap, for a backend that wants the tree at a
fixed path.

## verification

the store is content-addressed, so a hash mismatch means corruption or
tampering. two read-only checks:

- `verify()` hashes every object and compares against its name, failing with
  `Corrupt { hash }` on the first mismatch. a missing objects directory is an
  empty store, which is fine. call it before serving so corruption surfaces
  early, not mid-compose. `Core::verify_store()` is the public api for it.
- `verify_tree(tree)` checks the objects one tree references. tree files are
  hardlinks to their objects, so tampering with a file changes both the link
  and the object and the hash stops matching.

## garbage collection

`Store::gc(live)` sweeps every object and tree no generation pins. it holds the
pool lock for the whole sweep.

```
build the set of live tree names from the live tree hashes
collect every inode reachable from a live tree (hardlinks share an inode)
remove tree directories not in the live set
remove objects whose inode no live tree pins
```

tracking by inode is what makes this safe: a live tree pins a shared object
through its hardlink, so an object stays as long as any kept tree links it. the
`Core::gc()` gathers the live set by reading every generation's tree
list, then calls this.
