// content-addressed store: objects/<hash>.<mode>, trees/<treehash>/
// import hashes+deduplicates; tree hash is blake3 of the manifest

use crate::api::{Error, ObjectHash, Result};
use rustix::fs::{FlockOperation, flock};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const STREAM_BUF: usize = 64 * 1024;

// unique-per-process suffix for staged object temp files, so concurrent
// imports never collide on the same name
static STAGE_SEQ: AtomicU64 = AtomicU64::new(0);

// an exclusive flock on a lock file under the store, held while objects
// are placed. serializes across processes, not just threads
struct ObjectLock(std::fs::File);

impl ObjectLock {
    fn acquire(objects: &Path) -> Result<Self> {
        std::fs::create_dir_all(objects).map_err(|e| mk("objects", e))?;
        let path = objects.join(".lock");
        let f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| mk("open lock", e))?;
        flock(&f, FlockOperation::LockExclusive)
            .map_err(|e| Error::Io(format!("lock objects: {e}")))?;
        Ok(ObjectLock(f))
    }
}

impl Drop for ObjectLock {
    fn drop(&mut self) {
        // best effort: closing the fd releases the lock regardless
        let _ = flock(&self.0, FlockOperation::Unlock);
    }
}

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Store { root: root.into() }
    }

    fn objects(&self) -> PathBuf {
        self.root.join("objects")
    }

    // objects keyed by content+mode so two files with identical bytes but
    // different permissions stay separate (a shared inode would clobber one)
    fn object_path(&self, hash: &str, mode: u32) -> PathBuf {
        self.objects().join(format!("{hash}.{:o}", mode & 0o7777))
    }

    fn trees(&self) -> PathBuf {
        self.root.join("trees")
    }

    // the materialized tree directory for a hash, usable as a mount lower
    pub fn tree_path(&self, tree: &ObjectHash) -> PathBuf {
        self.trees().join(tree.as_str())
    }

    pub fn has(&self, tree: &ObjectHash) -> bool {
        self.tree_path(tree).is_dir()
    }

    // import a rootfs into the content-addressed store, returning its tree
    // hash. idempotent: re-importing identical content reuses existing objects
    pub fn import(&self, src: &Path) -> Result<ObjectHash> {
        std::fs::create_dir_all(self.objects()).map_err(|e| mk("objects", e))?;
        // the pool lock serializes object placement across processes via flock
        let _lock = ObjectLock::acquire(&self.objects())?;
        let mut entries = Vec::new();
        self.ingest(src, Path::new(""), &mut entries)?;
        entries.sort_by(|a, b| a.rel.cmp(&b.rel));

        let manifest = manifest_text(&entries);
        let tree = ObjectHash::new(blake3::hash(manifest.as_bytes()).to_hex().to_string());

        let dst = self.tree_path(&tree);
        if !dst.is_dir() {
            self.materialize(&entries, &dst)?;
        }
        Ok(tree)
    }

    // verify every object's hash matches its on-disk content, failing with
    // Corrupt on first mismatch. a missing objects dir is an empty store, ok
    pub fn verify(&self) -> Result<usize> {
        let dir = self.objects();
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(mk("objects", e)),
        };
        let mut checked = 0;
        for ent in entries {
            let ent = ent.map_err(|e| mk("readdir", e))?;
            let name = ent.file_name();
            let name = name.to_string_lossy();
            // skip bookkeeping dotfiles: the pool lock and any staging temp
            // files left by an interrupted import
            if name.starts_with('.') {
                continue;
            }
            // object names are <hash>.<octal-mode>; a missing dot means the
            // store is in an unknown state
            let hash = match name.rsplit_once('.') {
                Some((h, _mode)) => h,
                None => {
                    return Err(Error::Corrupt {
                        hash: format!("{name}: object missing mode suffix"),
                    });
                }
            };
            super::generations::verify(&ent.path(), hash)?;
            checked += 1;
        }
        Ok(checked)
    }

    // copy a stored tree to an arbitrary destination, hardlinking files so
    // it stays cheap. for a backend that wants the tree at a fixed path
    pub fn checkout(&self, tree: &ObjectHash, dst: &Path) -> Result<()> {
        let src = self.tree_path(tree);
        if !src.is_dir() {
            return Err(Error::Corrupt { hash: tree.to_string() });
        }
        copy_tree(&src, dst, true)
    }

    // verify the objects one tree references. tree files are hardlinks to
    // their objects, so tampering changes both and the hash stops matching
    pub fn verify_tree(&self, tree: &ObjectHash) -> Result<usize> {
        let dir = self.tree_path(tree);
        if !dir.is_dir() {
            return Err(Error::Corrupt { hash: tree.to_string() });
        }
        let mut checked = 0usize;
        self.verify_tree_dir(&dir, &mut checked)?;
        Ok(checked)
    }

    fn verify_tree_dir(&self, dir: &Path, checked: &mut usize) -> Result<()> {
        for ent in std::fs::read_dir(dir).map_err(|e| mk("readdir", e))? {
            let ent = ent.map_err(|e| mk("readdir", e))?;
            let path = ent.path();
            let ft = ent.file_type().map_err(|e| mk("filetype", e))?;
            if ft.is_dir() {
                self.verify_tree_dir(&path, checked)?;
            } else if ft.is_file() {
                let meta = std::fs::symlink_metadata(&path).map_err(|e| mk("stat", e))?;
                let mode = meta.permissions().mode();
                let hash = hash_file(&path)?;
                // the canonical object for this content+mode must exist; if the
                // bytes were altered, this name no longer resolves
                if !self.object_path(&hash, mode).exists() {
                    return Err(Error::Corrupt { hash });
                }
                *checked += 1;
            }
            // symlinks carry no content object, nothing to verify
        }
        Ok(())
    }

    // sweep unreferenced objects and trees, tracked by inode so a live tree
    // pins shared objects through hardlinks. holds the pool lock during sweep
    pub fn gc(&self, live: &[ObjectHash]) -> Result<usize> {
        let trees = self.trees();
        if !trees.is_dir() {
            return Ok(0);
        }
        let _lock = ObjectLock::acquire(&self.objects())?;

        let keep: HashSet<&str> = live.iter().map(|t| t.as_str()).collect();

        // collect inodes reachable from kept trees; objects are shared by hardlink
        let mut keep_inodes: HashSet<u64> = HashSet::new();
        for ent in std::fs::read_dir(&trees).map_err(|e| mk("readdir trees", e))? {
            let ent = ent.map_err(|e| mk("readdir", e))?;
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if !ent.path().is_dir() || !keep.contains(name.as_ref()) {
                continue;
            }
            collect_inodes(&ent.path(), &mut keep_inodes)?;
        }

        // 2. drop tree dirs no longer named live
        let mut removed = 0usize;
        for ent in std::fs::read_dir(&trees).map_err(|e| mk("readdir trees", e))? {
            let ent = ent.map_err(|e| mk("readdir", e))?;
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || keep.contains(name.as_ref()) {
                continue;
            }
            if ent.path().is_dir() {
                let _ = std::fs::remove_dir_all(ent.path());
            }
        }

        // 3. sweep objects whose inode no kept tree pins
        let objects = self.objects();
        if objects.is_dir() {
            for ent in std::fs::read_dir(&objects).map_err(|e| mk("readdir objects", e))? {
                let ent = ent.map_err(|e| mk("readdir", e))?;
                let name = ent.file_name();
                if name.to_string_lossy().starts_with('.') {
                    continue;
                }
                let meta = match std::fs::symlink_metadata(ent.path()) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if !keep_inodes.contains(&meta.ino()) {
                    let _ = std::fs::remove_file(ent.path());
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    // walk src, store regular-file contents as objects, collect entries
    fn ingest(&self, src: &Path, rel: &Path, out: &mut Vec<Entry>) -> Result<()> {
        let here = src.join(rel);
        let meta = std::fs::symlink_metadata(&here).map_err(|e| mk("stat", e))?;
        let ft = meta.file_type();
        let mode = meta.permissions().mode();

        if ft.is_dir() {
            if !rel.as_os_str().is_empty() {
                out.push(Entry::dir(rel, mode));
            }
            let mut names: Vec<_> = std::fs::read_dir(&here)
                .map_err(|e| mk("readdir", e))?
                .filter_map(|e| e.ok().map(|e| e.file_name()))
                .collect();
            names.sort();
            for name in names {
                self.ingest(src, &rel.join(name), out)?;
            }
        } else if ft.is_symlink() {
            let target = std::fs::read_link(&here).map_err(|e| mk("readlink", e))?;
            out.push(Entry::link(rel, target));
        } else if ft.is_file() {
            // stream the file through the hasher into a temp object, never
            // holding it all in memory
            let (hash, tmp) = self.stage_object(&here)?;
            let obj = self.object_path(&hash, mode);
            if obj.exists() {
                let _ = std::fs::remove_file(&tmp);
            } else {
                // the object carries the file's mode, since the tree links to it
                set_mode(&tmp, mode)?;
                std::fs::rename(&tmp, &obj).map_err(|e| mk("place object", e))?;
            }
            out.push(Entry::file(rel, mode, hash));
        }
        Ok(())
    }

    // copy a file into a temp object while hashing it in one pass
    fn stage_object(&self, src: &Path) -> Result<(String, PathBuf)> {
        let nonce = STAGE_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = self.objects().join(format!(".stage-{}-{nonce}", std::process::id()));

        let mut input = std::fs::File::open(src).map_err(|e| mk("open", e))?;
        let mut out = std::fs::File::create(&tmp).map_err(|e| mk("stage object", e))?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; STREAM_BUF];
        loop {
            let n = input.read(&mut buf).map_err(|e| mk("read", e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            if let Err(e) = out.write_all(&buf[..n]) {
                let _ = std::fs::remove_file(&tmp);
                return Err(mk("stage object", e));
            }
        }
        Ok((hasher.finalize().to_hex().to_string(), tmp))
    }

    // materialize the tree at a temp sibling, then rename, so it is never
    // partial. entries are sorted by path so no create_dir_all is needed
    fn materialize(&self, entries: &[Entry], dst: &Path) -> Result<()> {
        let tmp = dst.with_extension("tmp");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| mk("mkdir tree", e))?;

        for e in entries {
            let p = tmp.join(&e.rel);
            match &e.class {
                Class::Dir => {
                    std::fs::create_dir(&p).map_err(|e| mk("mkdir", e))?;
                    set_mode(&p, e.mode)?;
                }
                Class::File(hash) => {
                    // the object already carries e.mode (it is keyed by it)
                    std::fs::hard_link(self.object_path(hash, e.mode), &p)
                        .map_err(|e| mk("hardlink", e))?;
                }
                Class::Link(target) => {
                    std::os::unix::fs::symlink(target, &p).map_err(|e| mk("symlink", e))?;
                }
            }
        }
        std::fs::create_dir_all(self.trees()).map_err(|e| mk("trees", e))?;
        std::fs::rename(&tmp, dst).map_err(|e| mk("rename tree", e))
    }
}

struct Entry {
    rel: PathBuf,
    mode: u32,
    class: Class,
}

enum Class {
    Dir,
    File(String),
    Link(PathBuf),
}

impl Entry {
    fn dir(rel: &Path, mode: u32) -> Self {
        Entry { rel: rel.into(), mode, class: Class::Dir }
    }
    fn file(rel: &Path, mode: u32, hash: String) -> Self {
        Entry { rel: rel.into(), mode, class: Class::File(hash) }
    }
    fn link(rel: &Path, target: PathBuf) -> Self {
        Entry { rel: rel.into(), mode: 0, class: Class::Link(target) }
    }
}

// one stable line per entry, so the same tree always hashes the same
fn manifest_text(entries: &[Entry]) -> String {
    let mut s = String::new();
    for e in entries {
        let rel = e.rel.display();
        match &e.class {
            Class::Dir => s.push_str(&format!("d {:o} {rel}\n", e.mode)),
            Class::File(h) => s.push_str(&format!("f {:o} {h} {rel}\n", e.mode)),
            Class::Link(t) => s.push_str(&format!("l {} {rel}\n", t.display())),
        }
    }
    s
}

fn copy_tree(src: &Path, dst: &Path, hardlink: bool) -> Result<()> {
    std::fs::create_dir_all(dst).map_err(|e| mk("mkdir", e))?;
    for ent in std::fs::read_dir(src).map_err(|e| mk("readdir", e))? {
        let ent = ent.map_err(|e| mk("readdir", e))?;
        let from = ent.path();
        let to = dst.join(ent.file_name());
        let ft = ent.file_type().map_err(|e| mk("filetype", e))?;
        if ft.is_dir() {
            copy_tree(&from, &to, hardlink)?;
        } else if ft.is_symlink() {
            let target = std::fs::read_link(&from).map_err(|e| mk("readlink", e))?;
            std::os::unix::fs::symlink(target, &to).map_err(|e| mk("symlink", e))?;
        } else if hardlink {
            std::fs::hard_link(&from, &to).map_err(|e| mk("hardlink", e))?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| mk("copy", e))?;
        }
    }
    Ok(())
}

fn set_mode(p: &Path, mode: u32) -> Result<()> {
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).map_err(|e| mk("chmod", e))
}

fn mk(op: &str, e: std::io::Error) -> Error {
    Error::Io(format!("store {op}: {e}"))
}

fn hash_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path).map_err(|e| mk("open", e))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; STREAM_BUF];
    loop {
        let n = f.read(&mut buf).map_err(|e| mk("read", e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

// collect inode of every regular file; gc keeps any object a live tree reaches by inode
fn collect_inodes(dir: &Path, out: &mut HashSet<u64>) -> Result<()> {
    for ent in std::fs::read_dir(dir).map_err(|e| mk("readdir", e))? {
        let ent = ent.map_err(|e| mk("readdir", e))?;
        let ft = ent.file_type().map_err(|e| mk("filetype", e))?;
        if ft.is_dir() {
            collect_inodes(&ent.path(), out)?;
        } else if ft.is_file() {
            let meta = std::fs::symlink_metadata(ent.path()).map_err(|e| mk("stat", e))?;
            out.insert(meta.ino());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::tmp::TmpDir;

    fn scratch(tag: &str) -> TmpDir {
        TmpDir::new(&format!("cas-{tag}"))
    }

    fn write(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn import_is_deterministic() {
        let base = scratch("det");
        let src = base.join("src");
        write(&src, "usr/bin/sh", "ELF");
        write(&src, "etc/hostname", "void");
        let store = Store::new(base.join("store"));

        let a = store.import(&src).unwrap();
        let b = store.import(&src).unwrap();
        assert_eq!(a, b);
        assert!(store.has(&a));
    }

    #[test]
    fn identical_files_dedup() {
        let base = scratch("dedup");
        let src = base.join("src");
        // two layers, same libc content
        write(&src, "a/libc.so", "SAMEBYTES");
        write(&src, "b/libc.so", "SAMEBYTES");
        let store = Store::new(base.join("store"));
        store.import(&src).unwrap();

        let objs = std::fs::read_dir(base.join("store/objects"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .count();
        // one object for the shared content, not two
        assert_eq!(objs, 1);
    }

    #[test]
    fn checkout_reconstructs() {
        let base = scratch("checkout");
        let src = base.join("src");
        write(&src, "usr/bin/ls", "BIN");
        let store = Store::new(base.join("store"));
        let h = store.import(&src).unwrap();

        let out = base.join("out");
        store.checkout(&h, &out).unwrap();
        assert_eq!(std::fs::read_to_string(out.join("usr/bin/ls")).unwrap(), "BIN");
    }

    #[test]
    fn verify_passes_on_intact_store() {
        let base = scratch("verify-ok");
        let src = base.join("src");
        write(&src, "usr/bin/sh", "ELF");
        write(&src, "etc/os-release", "void");
        let store = Store::new(base.join("store"));
        store.import(&src).unwrap();
        // two distinct contents -> two objects, both intact
        assert_eq!(store.verify().unwrap(), 2);
    }

    #[test]
    fn verify_empty_store_is_ok() {
        let base = scratch("verify-empty");
        let store = Store::new(base.join("store"));
        // no objects dir yet: an empty store verifies as zero objects
        assert_eq!(store.verify().unwrap(), 0);
    }

    #[test]
    fn verify_catches_tampered_object() {
        let base = scratch("verify-bad");
        let src = base.join("src");
        write(&src, "usr/bin/sh", "ELF");
        let store = Store::new(base.join("store"));
        store.import(&src).unwrap();

        // corrupt the one object on disk; its content no longer matches the
        // hash in its name
        let objects = base.join("store/objects");
        let obj = std::fs::read_dir(&objects)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .unwrap()
            .path();
        std::fs::write(&obj, "TAMPERED").unwrap();

        match store.verify() {
            Err(Error::Corrupt { .. }) => {}
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn import_is_idempotent_under_lock() {
        // a second import of the same content reuses objects without the
        // pool lock deadlocking on itself
        let base = scratch("lock");
        let src = base.join("src");
        write(&src, "a/x", "DATA");
        let store = Store::new(base.join("store"));
        let a = store.import(&src).unwrap();
        let b = store.import(&src).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn concurrent_imports_converge_and_verify() {
        // many threads import the same content concurrently; the pool lock
        // serializes object placement so they converge to one object each
        let base = scratch("concurrent");
        let src = base.join("src");
        write(&src, "usr/lib/libc.so", "SHARED");
        write(&src, "etc/hostname", "void");
        let store_root = base.join("store");

        let mut handles = Vec::new();
        for _ in 0..8 {
            let root = store_root.clone();
            let s = src.clone();
            handles.push(std::thread::spawn(move || Store::new(root).import(&s)));
        }
        let hashes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap().unwrap()).collect();
        // every import saw the same tree hash
        assert!(hashes.windows(2).all(|w| w[0] == w[1]));

        // two distinct contents -> exactly two objects, no torn duplicates
        let objs = std::fs::read_dir(store_root.join("objects"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .count();
        assert_eq!(objs, 2);

        // and the pool is intact
        assert_eq!(Store::new(&store_root).verify().unwrap(), 2);
    }

    #[test]
    fn verify_tree_checks_only_its_own_objects() {
        let base = scratch("vtree");
        let src = base.join("src");
        write(&src, "usr/bin/sh", "ELF");
        write(&src, "etc/hostname", "void");
        let store = Store::new(base.join("store"));
        let tree = store.import(&src).unwrap();
        // both files of the tree verify
        assert_eq!(store.verify_tree(&tree).unwrap(), 2);
    }

    #[test]
    fn verify_tree_catches_tampered_object() {
        let base = scratch("vtree-bad");
        let src = base.join("src");
        write(&src, "usr/bin/sh", "ELF");
        let store = Store::new(base.join("store"));
        let tree = store.import(&src).unwrap();
        // tamper the object; the tree file is the same inode, so its content
        // changes too and no object is named by the new hash
        let objects = base.join("store/objects");
        let obj = std::fs::read_dir(&objects)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .unwrap()
            .path();
        std::fs::write(&obj, "TAMPERED").unwrap();
        assert!(matches!(store.verify_tree(&tree), Err(Error::Corrupt { .. })));
    }

    #[test]
    fn gc_keeps_live_trees_and_sweeps_the_rest() {
        let base = scratch("gc");
        let store = Store::new(base.join("store"));

        // tree a: a unique object plus a shared one
        let sa = base.join("a");
        write(&sa, "bin/a", "AONLY");
        write(&sa, "lib/shared", "SHARED");
        let a = store.import(&sa).unwrap();

        // tree b: a different unique object plus the same shared one
        let sb = base.join("b");
        write(&sb, "bin/b", "BONLY");
        write(&sb, "lib/shared", "SHARED");
        let b = store.import(&sb).unwrap();

        let objects = base.join("store/objects");
        let count = |p: &Path| {
            std::fs::read_dir(p)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .count()
        };
        // three distinct contents: AONLY, BONLY, SHARED
        assert_eq!(count(&objects), 3);

        // keep only tree a. b's unique object goes; the shared one stays
        // because a still pins its inode; b's tree dir is removed
        let removed = store.gc(std::slice::from_ref(&a)).unwrap();
        assert_eq!(removed, 1, "only BONLY should be swept");
        assert_eq!(count(&objects), 2);
        assert!(store.has(&a));
        assert!(!store.has(&b));
        // the kept tree still verifies end to end
        assert_eq!(store.verify_tree(&a).unwrap(), 2);
    }

    #[test]
    fn gc_empty_live_set_sweeps_everything() {
        let base = scratch("gc-all");
        let store = Store::new(base.join("store"));
        let src = base.join("src");
        write(&src, "bin/x", "DATA");
        store.import(&src).unwrap();
        let removed = store.gc(&[]).unwrap();
        assert_eq!(removed, 1);
        let objects = base.join("store/objects");
        let live = std::fs::read_dir(&objects)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .count();
        assert_eq!(live, 0);
    }
}
