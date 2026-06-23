// gens under .gen, current is a symlink. rollback moves it. this
// type owns the path math; it resolves and proposes, does not write yet

use crate::api::{Error, Gen, ObjectHash, Result};
use std::path::{Path, PathBuf};

pub struct Generations {
    root: PathBuf,
}

impl Generations {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Generations { root: root.into() }
    }

    pub fn path(&self, g: Gen) -> PathBuf {
        self.root.join(g.get().to_string())
    }

    pub fn current_link(&self) -> PathBuf {
        self.root.join("current")
    }

    pub fn current(&self) -> Result<Gen> {
        let target = std::fs::read_link(self.current_link())
            .map_err(|e| Error::Io(format!("read current: {e}")))?;
        let name = target
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| Error::Io("current points nowhere".into()))?;
        let n =
            name.parse::<u64>().map_err(|_| Error::Io(format!("current is not a gen: {name}")))?;
        Ok(Gen::new(n))
    }

    // one past the highest existing gen, or 1 if none
    pub fn next(&self) -> Result<Gen> {
        let mut max = 0u64;
        let entries = match std::fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(_) => return Ok(Gen::new(1)),
        };
        for e in entries.flatten() {
            if let Some(n) = e.file_name().to_str().and_then(|s| s.parse::<u64>().ok()) {
                max = max.max(n);
            }
        }
        Ok(Gen::new(max + 1))
    }

    // write a new gen directory listing its layer hashes, one per
    // line. does not switch current; call activate for that
    pub fn commit(&self, hashes: &[ObjectHash]) -> Result<Gen> {
        let g = self.next()?;
        let dir = self.path(g);
        std::fs::create_dir_all(&dir).map_err(|e| Error::Io(format!("mkdir {dir:?}: {e}")))?;
        let manifest: String = hashes.iter().map(|h| format!("{h}\n")).collect();
        std::fs::write(dir.join("layers"), manifest)
            .map_err(|e| Error::Io(format!("write manifest: {e}")))?;
        Ok(g)
    }

    // atomic rename over the symlink so a crash leaves either old or new gen,
    // never a half state; this is also the rollback primitive
    pub fn activate(&self, g: Gen) -> Result<()> {
        if !self.path(g).is_dir() {
            return Err(Error::Io(format!("no such gen: {g}")));
        }
        let tmp = self.root.join(".current.next");
        let _ = std::fs::remove_file(&tmp);
        std::os::unix::fs::symlink(g.get().to_string(), &tmp)
            .map_err(|e| Error::Io(format!("symlink: {e}")))?;
        std::fs::rename(&tmp, self.current_link())
            .map_err(|e| Error::Io(format!("rename current: {e}")))
    }

    // every existing gen number, ascending. a missing root is no gens
    pub fn all(&self) -> Result<Vec<Gen>> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::Io(format!("read gens: {e}"))),
        };
        let mut gens: Vec<Gen> = entries
            .flatten()
            .filter_map(|e| e.file_name().to_str().and_then(|s| s.parse::<u64>().ok()))
            .map(Gen::new)
            .collect();
        gens.sort_by_key(|g| g.get());
        Ok(gens)
    }

    pub fn trees(&self, g: Gen) -> Result<Vec<ObjectHash>> {
        let manifest = self.path(g).join("layers");
        let text = std::fs::read_to_string(&manifest)
            .map_err(|e| Error::Io(format!("read manifest {manifest:?}: {e}")))?;
        Ok(text
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .map(|l| ObjectHash::new(l.to_string()))
            .collect())
    }
}

// hash an object and compare against its expected content hash. the store
// is content addressed, so a mismatch means corruption or tampering
pub fn verify(object: &Path, hash: &str) -> Result<()> {
    let mut hasher = blake3::Hasher::new();
    let mut f =
        std::fs::File::open(object).map_err(|e| Error::Io(format!("open {object:?}: {e}")))?;
    std::io::copy(&mut f, &mut hasher).map_err(|e| Error::Io(format!("read {object:?}: {e}")))?;
    let got = hasher.finalize().to_hex();
    if got.as_str() == hash {
        Ok(())
    } else {
        Err(Error::Corrupt { hash: hash.into() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::tmp::TmpDir;

    fn scratch(tag: &str) -> TmpDir {
        TmpDir::new(&format!("gen-{tag}"))
    }

    fn h(s: &str) -> ObjectHash {
        ObjectHash::new(s)
    }

    #[test]
    fn commit_then_activate_roundtrips() {
        let dir = scratch("commit");
        let g = Generations::new(dir.path());
        let first = g.commit(&[h("aaa"), h("bbb")]).unwrap();
        assert_eq!(first, Gen::new(1));
        g.activate(first).unwrap();
        assert_eq!(g.current().unwrap(), first);
    }

    #[test]
    fn rollback_to_previous() {
        let dir = scratch("rollback");
        let g = Generations::new(dir.path());
        let g1 = g.commit(&[h("v1")]).unwrap();
        g.activate(g1).unwrap();
        let g2 = g.commit(&[h("v2")]).unwrap();
        g.activate(g2).unwrap();
        assert_eq!(g.current().unwrap(), Gen::new(2));
        // rollback is just activating an older gen
        g.activate(g1).unwrap();
        assert_eq!(g.current().unwrap(), Gen::new(1));
    }

    #[test]
    fn activate_missing_fails() {
        let dir = scratch("missing");
        let g = Generations::new(dir.path());
        assert!(g.activate(Gen::new(7)).is_err());
    }

    #[test]
    fn all_lists_gens_and_trees_reads_manifest() {
        let dir = scratch("listing");
        let g = Generations::new(dir.path());
        let g1 = g.commit(&[h("treeA"), h("treeB")]).unwrap();
        let g2 = g.commit(&[h("treeC")]).unwrap();
        assert_eq!(g.all().unwrap(), vec![g1, g2]);
        assert_eq!(g.trees(g1).unwrap(), vec![h("treeA"), h("treeB")]);
        assert_eq!(g.trees(g2).unwrap(), vec![h("treeC")]);
    }
}
