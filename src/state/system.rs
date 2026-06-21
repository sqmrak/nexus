// private mirror struct so the toml format can change without touching
// the config the core acts on

use crate::api::{Error, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

// settings the core acts on. defaults apply when a key is absent
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct System {
    // store backend: "overlay" today, "composefs" later. data, not a build
    // time choice, so a fork can switch without recompiling
    pub backend: String,
    // evict a namespace after this long idle. zero means never evict
    pub idle_evict: Duration,
    // a test harness points this inside its sandbox so a test run creates
    // scopes there, never touching the outer system's cgroup hierarchy
    pub cgroup_root: PathBuf,
}

impl Default for System {
    fn default() -> Self {
        System {
            backend: crate::vocab::BACKEND_OVERLAY.into(),
            idle_evict: Duration::from_secs(900),
            cgroup_root: Path::new(crate::paths::CGROUP2_ROOT).join(crate::vocab::CG_SUBTREE),
        }
    }
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    backend: Option<String>,
    // seconds; absent or 0 disables eviction
    #[serde(default)]
    idle_evict_secs: Option<u64>,
    #[serde(default)]
    cgroup_root: Option<String>,
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
    // validated here because Core::open silently falls back to overlay for
    // any unrecognized token, so a typo would otherwise run on the wrong backend
    if backend != crate::vocab::BACKEND_OVERLAY && backend != crate::vocab::BACKEND_EROFS {
        return Err(Error::Config(format!(
            "unknown backend {backend:?}; expected \"overlay\" or \"erofs\""
        )));
    }
    Ok(System {
        backend,
        idle_evict: f.idle_evict_secs.map(Duration::from_secs).unwrap_or(d.idle_evict),
        cgroup_root: f.cgroup_root.map(PathBuf::from).unwrap_or(d.cgroup_root),
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
    }

    #[test]
    fn reads_overrides() {
        let s = parse_system("backend = \"erofs\"\nidle_evict_secs = 60\n").unwrap();
        assert_eq!(s.backend, "erofs");
        assert_eq!(s.idle_evict, Duration::from_secs(60));
    }

    #[test]
    fn rejects_unknown_backend() {
        let err = parse_system("backend = \"compsefs\"\n").unwrap_err();
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
        let s = parse_system("cgroup_root = \"/tmp/spark/cgroup\"\n").unwrap();
        assert_eq!(s.cgroup_root, Path::new("/tmp/spark/cgroup"));
    }
}
