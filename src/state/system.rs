// private mirror struct so the toml format can change without touching
// the config the core acts on

use crate::api::{Error, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

// settings the core acts on. defaults apply when a key is absent
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct System {
    pub backend: String,
    pub idle_evict: Duration,
    // a test harness points this inside its sandbox so a test run creates
    // scopes there, never touching the outer system's cgroup hierarchy
    pub cgroup_root: PathBuf,
    // (fstype, target) mounted at early boot. admin, not compiled in
    pub pseudo: Vec<(String, String)>,
    // paths shared across layers, bound in from outside
    pub globals: Vec<String>,
    // cgroup controllers to enable. empty means enable all available
    pub cgroup_controllers: Vec<String>,
}

impl Default for System {
    fn default() -> Self {
        System {
            backend: crate::vocab::BACKEND_OVERLAY.into(),
            idle_evict: Duration::from_secs(900),
            cgroup_root: Path::new(crate::paths::CGROUP2_ROOT).join(crate::vocab::CG_SUBTREE),
            pseudo: vec![
                ("proc".into(), "/proc".into()),
                ("sysfs".into(), "/sys".into()),
                ("devtmpfs".into(), "/dev".into()),
            ],
            globals: vec![
                "/home".into(),
                "/dev".into(),
                "/proc".into(),
                "/sys".into(),
                "/run/user".into(),
                "/tmp".into(),
                "/boot".into(),
            ],
            cgroup_controllers: vec!["memory".into(), "pids".into(), "cpu".into()],
        }
    }
}

#[derive(Deserialize)]
struct PseudoEntry {
    fstype: String,
    target: String,
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    backend: Option<String>,
    #[serde(default)]
    idle_evict_secs: Option<u64>,
    #[serde(default)]
    cgroup_root: Option<String>,
    #[serde(default)]
    pseudo: Option<Vec<PseudoEntry>>,
    #[serde(default)]
    globals: Option<Vec<String>>,
    #[serde(default)]
    cgroup_controllers: Option<Vec<String>>,
}

pub fn load_system(path: &Path) -> Result<System> {
    let text =
        std::fs::read_to_string(path).map_err(|e| Error::Config(format!("read {path:?}: {e}")))?;
    parse_system(&text)
}

fn parse_system(text: &str) -> Result<System> {
    let f: File = toml::from_str(text).map_err(|e| Error::Config(format!("parse: {e}")))?;
    let d = System::default();
    let backend = f.backend.unwrap_or(d.backend);
    if backend != crate::vocab::BACKEND_OVERLAY && backend != crate::vocab::BACKEND_EROFS {
        return Err(Error::Config(format!(
            "unknown backend {backend:?}; expected \"overlay\" or \"erofs\""
        )));
    }
    Ok(System {
        backend,
        idle_evict: f.idle_evict_secs.map(Duration::from_secs).unwrap_or(d.idle_evict),
        cgroup_root: f.cgroup_root.map(PathBuf::from).unwrap_or(d.cgroup_root),
        pseudo: f
            .pseudo
            .map(|v| v.into_iter().map(|e| (e.fstype, e.target)).collect())
            .unwrap_or(d.pseudo),
        globals: f.globals.unwrap_or(d.globals),
        cgroup_controllers: f.cgroup_controllers.unwrap_or(d.cgroup_controllers),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        let s = parse_system("").unwrap();
        assert_eq!(s, System::default());
        assert_eq!(s.backend, "overlay");
        assert_eq!(s.pseudo.len(), 3);
        assert_eq!(s.globals.len(), 7);
        assert_eq!(s.cgroup_controllers.len(), 3);
    }

    #[test]
    fn reads_overrides() {
        let s = parse_system("backend = \"erofs\"\nidle_evict_secs = 60\n").unwrap();
        assert_eq!(s.backend, "erofs");
        assert_eq!(s.idle_evict, Duration::from_secs(60));
    }

    #[test]
    fn rejects_unknown_backend() {
        let err = parse_system("backend = \"composefs\"\n").unwrap_err();
        assert!(err.to_string().contains("unknown backend"), "got: {err}");
    }

    #[test]
    fn zero_evict_disables() {
        let s = parse_system("idle_evict_secs = 0\n").unwrap();
        assert_eq!(s.idle_evict, Duration::from_secs(0));
    }

    #[test]
    fn cgroup_root_defaults_to_system_subtree() {
        let s = parse_system("").unwrap();
        assert_eq!(s.cgroup_root, Path::new("/sys/fs/cgroup").join("nexus"));
    }

    #[test]
    fn cgroup_root_override_is_read() {
        let s = parse_system("cgroup_root = \"/tmp/nexus-test/cgroup\"\n").unwrap();
        assert_eq!(s.cgroup_root, Path::new("/tmp/nexus-test/cgroup"));
    }

    #[test]
    fn reads_globals_override() {
        let s = parse_system("globals = [\"/home\", \"/media\"]\n").unwrap();
        assert_eq!(s.globals, vec!["/home", "/media"]);
    }

    #[test]
    fn reads_pseudo_override() {
        let s = parse_system(
            "[[pseudo]]\nfstype = \"proc\"\ntarget = \"/proc\"\n[[pseudo]]\nfstype = \"devtmpfs\"\ntarget = \"/dev\"\n",
        )
        .unwrap();
        assert_eq!(
            s.pseudo,
            vec![("proc".into(), "/proc".into()), ("devtmpfs".into(), "/dev".into())]
        );
    }

    #[test]
    fn reads_cgroup_controllers_override() {
        let s = parse_system("cgroup_controllers = [\"memory\", \"cpu\"]\n").unwrap();
        assert_eq!(s.cgroup_controllers, vec!["memory", "cpu"]);
    }

    #[test]
    fn empty_cgroup_controllers_is_allowed() {
        let s = parse_system("cgroup_controllers = []\n").unwrap();
        assert_eq!(s.cgroup_controllers.len(), 0);
    }
}
