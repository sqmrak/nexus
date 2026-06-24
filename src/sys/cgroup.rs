// cgroup v2 mechanism. processes live only in leaf cgroups; the subtree
// delegates controllers down. limit application is best effort

use crate::api::{CpuMax, Error, Limits, Result};
use crate::paths::{mkdir_all, CGROUP2_ROOT};
use crate::vocab;
use std::path::{Path, PathBuf};

pub struct Cgroups {
    // the nexus subtree dir. leaves are created beneath it
    root: PathBuf,
}

impl Cgroups {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Cgroups { root: root.into() }
    }

    // the conventional location: <unified cgroup root>/nexus
    pub fn system() -> Self {
        Cgroups::new(Path::new(CGROUP2_ROOT).join(vocab::CG_SUBTREE))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn scope_path(&self, scope: &str) -> PathBuf {
        self.root.join(scope)
    }

    // delegate wanted controllers from the parent first, so they appear in
    // our cgroup.controllers. the parent write may fail in delegated subtrees
    pub fn prepare(&self, want: &[&str]) -> Result<Vec<String>> {
        mkdir_all(&self.root)?;

        if let Some(parent) = self.root.parent() {
            let avail = read_controllers(&parent.join(vocab::CG_CONTROLLERS));
            let _ = enable(&parent.join(vocab::CG_SUBTREE_CONTROL), &filter(want, &avail));
        }

        let usable = filter(want, &read_controllers(&self.root.join(vocab::CG_CONTROLLERS)));
        enable(&self.root.join(vocab::CG_SUBTREE_CONTROL), &usable)?;
        Ok(usable)
    }

    // every controller the kernel exposes at the unified cgroup root.
    // structural: no name list, reads /sys/fs/cgroup/cgroup.controllers
    pub fn all_available(&self) -> Vec<String> {
        read_controllers(&Path::new(CGROUP2_ROOT).join(vocab::CG_CONTROLLERS))
    }

    pub fn create(&self, scope: &str) -> Result<PathBuf> {
        let p = self.scope_path(scope);
        mkdir_all(&p)?;
        Ok(p)
    }

    // move the calling process into the scope before execve so the child inherits the cgroup
    pub fn enter(&self, scope: &str) -> Result<()> {
        let pid = std::process::id();
        let path = self.scope_path(scope).join(vocab::CG_PROCS);
        std::fs::write(&path, pid.to_string())
            .map_err(|e| Error::Io(format!("enter cgroup {}: {e}", path.display())))
    }

    // write a scope's limits. best effort per field: an interface file that
    // is absent (its controller is not enabled here) is skipped
    pub fn apply(&self, scope: &str, lim: &Limits) -> Result<()> {
        let dir = self.scope_path(scope);
        if let Some(b) = lim.memory_max {
            write_limit(&dir.join(vocab::CG_MEMORY_MAX), &b.to_string())?;
        }
        if let Some(n) = lim.pids_max {
            write_limit(&dir.join(vocab::CG_PIDS_MAX), &n.to_string())?;
        }
        if let Some(w) = lim.cpu_weight {
            // cpu.weight is 1..=10000; clamp so a stray value does not turn a
            // launch into a hard failure
            write_limit(&dir.join(vocab::CG_CPU_WEIGHT), &w.clamp(1, 10_000).to_string())?;
        }
        if let Some(c) = lim.cpu_max {
            write_limit(&dir.join(vocab::CG_CPU_MAX), &cpu_max_line(&c))?;
        }
        Ok(())
    }

    // best effort: in a delegated subtree writing to the root cgroup.procs
    // may be forbidden. a failed leave only means the process stays in its
    // leaf, remove() cleans the scope regardless
    pub fn leave(&self) -> Result<()> {
        let path = Path::new(CGROUP2_ROOT).join(vocab::CG_PROCS);
        let _ = std::fs::write(&path, std::process::id().to_string());
        Ok(())
    }

    // remove an empty leaf scope. an absent scope is already gone, so that is
    // a no-op, not an error
    pub fn remove(&self, scope: &str) -> Result<()> {
        match std::fs::remove_dir(self.scope_path(scope)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(format!("remove cgroup scope {scope}: {e}"))),
        }
    }

    // leaf scope names, for the daemon to sweep empty ones. the cgroup.*
    // interface files in the subtree are not scopes
    pub fn scopes(&self) -> Result<Vec<String>> {
        let rd = match std::fs::read_dir(&self.root) {
            Ok(rd) => rd,
            // no subtree yet means no scopes
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::Io(format!("read cgroup subtree: {e}"))),
        };
        let mut out = Vec::new();
        for ent in rd {
            let ent = ent.map_err(|e| Error::Io(format!("read cgroup subtree: {e}")))?;
            if ent.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(name) = ent.file_name().to_str() {
                    out.push(name.to_string());
                }
            }
        }
        Ok(out)
    }
}

// the controllers in want that are present in avail, keeping want's order
fn filter(want: &[&str], avail: &[String]) -> Vec<String> {
    want.iter()
        .copied()
        .filter(|&w| avail.iter().any(|a| a.as_str() == w))
        .map(|w| w.to_string())
        .collect()
}

// "+a +b" for each controller. an empty set is a no-op: writing an empty
// string to subtree_control would error
fn enable(subtree_control: &Path, ctrls: &[String]) -> Result<()> {
    if ctrls.is_empty() {
        return Ok(());
    }
    let line = ctrls.iter().map(|c| format!("+{c}")).collect::<Vec<_>>().join(" ");
    std::fs::write(subtree_control, line)
        .map_err(|e| Error::Io(format!("enable controllers {}: {e}", subtree_control.display())))
}

// the cgroup.controllers / cgroup.subtree_control wire format: space
// separated names. an unreadable file means none are available here
fn read_controllers(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|s| s.split_whitespace().map(|w| w.to_string()).collect())
        .unwrap_or_default()
}

// cpu.max is "quota period" in microseconds
fn cpu_max_line(c: &CpuMax) -> String {
    format!("{} {}", c.quota_us(), c.period_us())
}

// skip missing interface files rather than failing: they are absent when
// their controller is off here, which is expected (best effort)
fn write_limit(path: &Path, val: &str) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    std::fs::write(path, val).map_err(|e| Error::Io(format!("write {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tmp::TmpDir;

    // a fake subtree with a given set of controllers available to it, as the
    // kernel would expose after the parent delegated them
    fn subtree(tag: &str, controllers: &str) -> (TmpDir, PathBuf) {
        let tmp = TmpDir::new(tag);
        let root = tmp.join(vocab::CG_SUBTREE);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(vocab::CG_CONTROLLERS), controllers).unwrap();
        (tmp, root)
    }

    #[test]
    fn prepare_enables_available_controllers_only() {
        let (_t, root) = subtree("cg-prep", "cpu memory pids\n");
        let cg = Cgroups::new(root.clone());
        // io is not available here, so it is dropped
        let usable = cg.prepare(&[vocab::CG_CTRL_MEMORY, vocab::CG_CTRL_PIDS, "io"]).unwrap();
        assert_eq!(usable, vec!["memory", "pids"]);

        let sc = std::fs::read_to_string(root.join(vocab::CG_SUBTREE_CONTROL)).unwrap();
        assert!(sc.contains("+memory"));
        assert!(sc.contains("+pids"));
        assert!(!sc.contains("+io"));
    }

    #[test]
    fn create_and_remove_scope() {
        let (_t, root) = subtree("cg-scope", "memory\n");
        let cg = Cgroups::new(root);
        let p = cg.create("void").unwrap();
        assert!(p.is_dir());
        cg.remove("void").unwrap();
        assert!(!p.exists());
        cg.remove("void").unwrap();
    }

    #[test]
    fn enter_writes_pid_to_procs() {
        let (_t, root) = subtree("cg-enter", "memory\n");
        let cg = Cgroups::new(root.clone());
        cg.create("void").unwrap();
        cg.enter("void").unwrap();
        let got = std::fs::read_to_string(root.join("void").join(vocab::CG_PROCS)).unwrap();
        assert_eq!(got, std::process::id().to_string());
    }

    #[test]
    fn apply_writes_each_present_limit() {
        let (_t, root) = subtree("cg-apply", "cpu memory pids\n");
        let cg = Cgroups::new(root.clone());
        let leaf = cg.create("void").unwrap();
        // the interface files exist when their controller is enabled
        for f in [vocab::CG_MEMORY_MAX, vocab::CG_PIDS_MAX, vocab::CG_CPU_WEIGHT, vocab::CG_CPU_MAX]
        {
            std::fs::write(leaf.join(f), "").unwrap();
        }

        let lim = Limits {
            memory_max: Some(104_857_600),
            pids_max: Some(128),
            cpu_weight: Some(200),
            cpu_max: Some(CpuMax::new(50_000, 100_000).unwrap()),
        };
        cg.apply("void", &lim).unwrap();

        assert_eq!(std::fs::read_to_string(leaf.join(vocab::CG_MEMORY_MAX)).unwrap(), "104857600");
        assert_eq!(std::fs::read_to_string(leaf.join(vocab::CG_PIDS_MAX)).unwrap(), "128");
        assert_eq!(std::fs::read_to_string(leaf.join(vocab::CG_CPU_WEIGHT)).unwrap(), "200");
        assert_eq!(std::fs::read_to_string(leaf.join(vocab::CG_CPU_MAX)).unwrap(), "50000 100000");
    }

    #[test]
    fn apply_skips_absent_interface_files() {
        let (_t, root) = subtree("cg-skip", "memory\n");
        let cg = Cgroups::new(root);
        let leaf = cg.create("void").unwrap();
        // only memory.max exists; the cpu controller is off here
        std::fs::write(leaf.join(vocab::CG_MEMORY_MAX), "").unwrap();

        let lim = Limits {
            memory_max: Some(1024),
            cpu_max: Some(CpuMax::new(50_000, 100_000).unwrap()),
            ..Default::default()
        };
        // the missing cpu.max must not turn into an error
        cg.apply("void", &lim).unwrap();
        assert_eq!(std::fs::read_to_string(leaf.join(vocab::CG_MEMORY_MAX)).unwrap(), "1024");
        assert!(!leaf.join(vocab::CG_CPU_MAX).exists());
    }

    #[test]
    fn cpu_weight_is_clamped() {
        let (_t, root) = subtree("cg-clamp", "cpu\n");
        let cg = Cgroups::new(root);
        let leaf = cg.create("void").unwrap();
        std::fs::write(leaf.join(vocab::CG_CPU_WEIGHT), "").unwrap();
        cg.apply("void", &Limits { cpu_weight: Some(99_999), ..Default::default() }).unwrap();
        assert_eq!(std::fs::read_to_string(leaf.join(vocab::CG_CPU_WEIGHT)).unwrap(), "10000");
    }

    #[test]
    fn scopes_lists_leaves_not_interface_files() {
        let (_t, root) = subtree("cg-list", "memory\n");
        let cg = Cgroups::new(root);
        cg.create("void").unwrap();
        cg.create("arch").unwrap();
        let mut got = cg.scopes().unwrap();
        got.sort();
        // the cgroup.controllers file in the subtree is not a scope
        assert_eq!(got, vec!["arch", "void"]);
    }
}
