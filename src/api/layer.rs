// a layer: a donor root the core can compose and run. every layer is
// equal; differences are data on the descriptor, not special cases

use super::flags::LayerFlags;
use super::libc::Libc;
use super::resources::Limits;
use super::sandbox::Sandbox;

/// a layer's unique id, used as a directory name in the store. constructed only
/// through new(), which rejects empty, slash, NUL, or "." / ".."
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LayerId(String);

impl LayerId {
    /// return a valid id or an error describing the violation
    pub fn new(id: impl Into<String>) -> Result<Self, String> {
        let id = id.into();
        if id.is_empty() {
            return Err("layer id is empty".into());
        }
        if id == "." || id == ".." {
            return Err(format!("layer id is a reserved name: {id:?}"));
        }
        if let Some(bad) = id.chars().find(|c| *c == '/' || *c == '\0') {
            return Err(format!("layer id contains an invalid character: {bad:?}"));
        }
        Ok(LayerId(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for LayerId {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        LayerId::new(s)
    }
}

impl std::fmt::Display for LayerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// the kind of a layer: how it was acquired and its role at boot
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerType {
    /// independently installed layer, equal in privilege to every other native
    Native,
    /// the former root, the layer the meta-distro was installed onto. exactly
    /// one, equal in privilege to native
    Shadowed,
    /// guaranteed-healthy fallback so the system is never unbootable
    Rescue,
}

/// a layer's configured identity: id, type, priority, libc, flags, sandbox
/// and resource limits. read from system state, never inferred from a distro name
#[derive(Clone, Debug)]
pub struct LayerDescriptor {
    pub id: LayerId,
    pub r#type: LayerType,
    /// lower number wins. shadowed is 1 at install, native is 1 + n
    pub priority: u32,
    pub libc: Libc,
    /// typed behaviour flags the core understands, plus fork extras
    pub flags: LayerFlags,
    /// confinement applied at launch. empty means unconfined
    pub sandbox: Sandbox,
    /// resource ceilings written to the layer's cgroup v2 scope at launch.
    /// empty means a scope with accounting but no caps
    pub resources: Limits,
}

/// a descriptor whose namespace is built and persisted. open ns_path and
/// setns to enter without a daemon round trip
#[derive(Clone, Debug)]
pub struct Layer {
    pub desc: LayerDescriptor,
    /// persisted mount namespace file. open it and setns to enter
    pub ns_path: std::path::PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_ids_accepted() {
        for id in ["void", "debian-12", "my.layer", "a_b", "x"] {
            assert!(LayerId::new(id).is_ok(), "{id} should be valid");
        }
    }

    #[test]
    fn empty_id_rejected() {
        assert!(LayerId::new("").is_err());
    }

    #[test]
    fn path_traversal_ids_rejected() {
        // a slash would escape the store dir; "." / ".." are path specials
        for id in ["a/b", "/abs", ".", "..", "foo/", "with\0nul"] {
            assert!(LayerId::new(id).is_err(), "{id:?} should be rejected");
        }
    }

    #[test]
    fn roundtrips_through_str() {
        let id = LayerId::new("void").unwrap();
        assert_eq!(id.as_str(), "void");
        assert_eq!(id.to_string(), "void");
        assert_eq!("void".parse::<LayerId>().unwrap(), id);
    }
}
