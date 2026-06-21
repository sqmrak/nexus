use crate::api::{HealthCheck, LayerDescriptor};
use std::path::Path;

// resolved relative to the layer root, never the caller, so a layer can't
// accidentally pass because the caller has a matching file
pub struct DefaultHealthCheck;

impl HealthCheck for DefaultHealthCheck {
    fn is_healthy(&self, layer: &LayerDescriptor, root: &Path) -> bool {
        match &layer.libc.loader {
            None => true,
            Some(loader) => {
                let rel = loader.strip_prefix('/').unwrap_or(loader);
                root.join(rel).exists()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Libc, LayerId, LayerType};
    use crate::tmp::TmpDir;

    fn desc(loader: Option<&str>) -> LayerDescriptor {
        LayerDescriptor {
            id: LayerId::new("void").unwrap(),
            r#type: LayerType::Native,
            priority: 1,
            libc: Libc { name: "test".into(), loader: loader.map(str::to_string) },
            flags: Default::default(),
            sandbox: Default::default(),
            resources: Default::default(),
        }
    }

    #[test]
    fn static_layer_is_always_healthy() {
        // no loader: healthy even against a root that does not exist
        assert!(DefaultHealthCheck.is_healthy(&desc(None), Path::new("/nonexistent/root")));
    }

    #[test]
    fn loader_present_in_layer_is_healthy() {
        let dir = TmpDir::new("health-present");
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("lib/ld.so"), b"x").unwrap();
        assert!(DefaultHealthCheck.is_healthy(&desc(Some("/lib/ld.so")), &dir));
    }

    #[test]
    fn loader_missing_from_layer_is_unhealthy() {
        let dir = TmpDir::new("health-missing");
        assert!(!DefaultHealthCheck.is_healthy(&desc(Some("/lib/ld.so")), &dir));
    }

    #[test]
    fn loader_resolves_inside_layer_not_host() {
        // /bin/sh exists on the caller but not in this empty layer root: the
        // check must fail, proving resolution is relative to root, never outside
        let dir = TmpDir::new("health-host");
        assert!(!DefaultHealthCheck.is_healthy(&desc(Some("/bin/sh")), &dir));
    }
}
