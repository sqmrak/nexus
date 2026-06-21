// a receipt of the mounts a backend made for one layer, in creation order.
// the registry stores it and hands it to unmount_root at eviction
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mounts {
    points: Vec<PathBuf>,
}

impl Mounts {
    pub fn new() -> Self {
        Mounts::default()
    }

    /// record a mount point the backend just created, in the order created
    pub fn record(&mut self, point: impl Into<PathBuf>) {
        self.points.push(point.into());
    }

    /// the mount points in teardown order: the reverse of creation, so an
    /// overlay is unmounted before the lower it was stacked on
    pub fn teardown_order(&self) -> impl Iterator<Item = &Path> {
        self.points.iter().rev().map(PathBuf::as_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teardown_reverses_creation_order() {
        let mut m = Mounts::new();
        m.record("/lower");
        m.record("/stage");
        let order: Vec<&Path> = m.teardown_order().collect();
        assert_eq!(order, [Path::new("/stage"), Path::new("/lower")]);
    }
}
