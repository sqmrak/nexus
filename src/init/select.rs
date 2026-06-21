use crate::api::{Error, HealthCheck, LayerDescriptor, LayerId, LayerSelector, LayerType, Result};
use std::path::Path;

// healthy layer with lowest priority wins; rescue layers are the last-resort
// fallback and only win when no non-rescue layer is healthy
pub struct DefaultSelector;

impl LayerSelector for DefaultSelector {
    fn select(
        &self,
        candidates: &[LayerDescriptor],
        health: &dyn HealthCheck,
        store_root: &Path,
    ) -> Result<LayerId> {
        candidates
            .iter()
            .filter(|l| health.is_healthy(l, &store_root.join(l.id.as_str())))
            .min_by_key(|l| (l.r#type == LayerType::Rescue, l.priority))
            .map(|l| l.id.clone())
            .ok_or(Error::NoHealthyLayer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Libc, LayerType};

    // a health check answering from a fixed set of healthy ids, so selection
    // is tested without touching the filesystem
    struct Healthy(&'static [&'static str]);

    impl HealthCheck for Healthy {
        fn is_healthy(&self, layer: &LayerDescriptor, _root: &Path) -> bool {
            self.0.contains(&layer.id.as_str())
        }
    }

    fn desc(id: &str, priority: u32) -> LayerDescriptor {
        LayerDescriptor {
            id: LayerId::new(id).unwrap(),
            r#type: LayerType::Native,
            priority,
            libc: Libc { name: "static".into(), loader: None },
            flags: Default::default(),
            sandbox: Default::default(),
            resources: Default::default(),
        }
    }

    fn rescue(id: &str, priority: u32) -> LayerDescriptor {
        LayerDescriptor { r#type: LayerType::Rescue, ..desc(id, priority) }
    }

    fn pick(candidates: &[LayerDescriptor], healthy: &'static [&'static str]) -> Result<LayerId> {
        DefaultSelector.select(candidates, &Healthy(healthy), Path::new("/store"))
    }

    #[test]
    fn picks_lowest_priority_healthy() {
        let layers = [desc("a", 3), desc("b", 1), desc("c", 2)];
        assert_eq!(pick(&layers, &["a", "b", "c"]).unwrap().as_str(), "b");
    }

    #[test]
    fn skips_unhealthy_for_next_lowest() {
        // b is lowest but unhealthy, so the healthy c at priority 2 wins
        let layers = [desc("a", 3), desc("b", 1), desc("c", 2)];
        assert_eq!(pick(&layers, &["a", "c"]).unwrap().as_str(), "c");
    }

    #[test]
    fn no_healthy_layer_is_an_error() {
        let layers = [desc("a", 1), desc("b", 2)];
        assert!(matches!(pick(&layers, &[]), Err(Error::NoHealthyLayer)));
    }

    #[test]
    fn empty_candidates_is_an_error() {
        assert!(matches!(pick(&[], &["a"]), Err(Error::NoHealthyLayer)));
    }

    #[test]
    fn rescue_is_last_resort_when_native_is_unhealthy() {
        // rescue has priority 9, native has 5 but is unhealthy; rescue wins
        let layers = [desc("n", 5), rescue("r", 9)];
        assert_eq!(pick(&layers, &["r"]).unwrap().as_str(), "r");
    }

    #[test]
    fn native_beats_rescue_when_both_are_healthy() {
        // rescue priority 1 vs native priority 99: native still wins
        let layers = [rescue("r", 1), desc("n", 99)];
        assert_eq!(pick(&layers, &["r", "n"]).unwrap().as_str(), "n");
    }
}
