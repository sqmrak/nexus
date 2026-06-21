// a flat erofs image keeps lookups fast (no deep overlay stack); the image
// is built at install/forge time, this backend only mounts it

use crate::api::{LayerDescriptor, Mounts, Result, StoreBackend};
use crate::paths::mkdir_all;
use crate::sys::{mount, nsproc};
use crate::vocab;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};

pub struct ErofsBackend {
    store_root: PathBuf,
    state_root: PathBuf,
}

impl ErofsBackend {
    pub fn new(store_root: impl Into<PathBuf>, state_root: impl Into<PathBuf>) -> Self {
        ErofsBackend { store_root: store_root.into(), state_root: state_root.into() }
    }

    fn image(&self, layer: &LayerDescriptor) -> PathBuf {
        self.store_root.join(format!("{}.erofs", layer.id))
    }

    fn lower(&self, layer: &LayerDescriptor) -> PathBuf {
        self.state_root.join("lower").join(layer.id.as_str())
    }

    fn ephemeral_base(&self, layer: &LayerDescriptor) -> PathBuf {
        self.state_root.join("ephemeral").join(layer.id.as_str())
    }

    // mount a fresh tmpfs for the layer and lay out its upper and work dirs
    // inside it. returns the two paths as strings for the overlay config
    fn tmpfs_upper(&self, layer: &LayerDescriptor) -> Result<(String, String)> {
        let base = self.ephemeral_base(layer);
        mkdir_all(&base)?;
        mount::mount_tmpfs(&base.display().to_string())?;
        let upper = base.join("upper");
        let work = base.join("work");
        mkdir_all(&upper)?;
        mkdir_all(&work)?;
        Ok((upper.display().to_string(), work.display().to_string()))
    }
}

impl StoreBackend for ErofsBackend {
    fn mount_root(
        &self,
        layer: &LayerDescriptor,
        target: &Path,
        userns: Option<BorrowedFd<'_>>,
    ) -> Result<Mounts> {
        let mut mounts = Mounts::new();
        // mount the flat image as the read-only lower
        let lower = self.lower(layer);
        mkdir_all(&lower)?;
        let fs = mount::fsopen(vocab::FS_EROFS)?;
        mount::fsconfig_string(
            fs.as_fd(),
            vocab::OPT_SOURCE,
            &self.image(layer).display().to_string(),
        )?;
        mount::fsconfig_create(fs.as_fd())?;
        let mnt = mount::fsmount(fs.as_fd())?;
        mount::move_mount(mnt.as_fd(), &lower.display().to_string())?;
        mounts.record(&lower);

        // overlay a writable upper over it. upper by flag: atomic=none
        // (read-only), ephemeral=tmpfs, default=on-disk
        let fs = mount::fsopen(vocab::FS_OVERLAY)?;
        mount::fsconfig_string(fs.as_fd(), vocab::OPT_LOWERDIR, &lower.display().to_string())?;
        if layer.flags.ephemeral_upper() {
            let (upper, work) = self.tmpfs_upper(layer)?;
            mounts.record(self.ephemeral_base(layer));
            mount::fsconfig_string(fs.as_fd(), vocab::OPT_UPPERDIR, &upper)?;
            mount::fsconfig_string(fs.as_fd(), vocab::OPT_WORKDIR, &work)?;
        } else if !layer.flags.no_persistent_upper() {
            let upper = self.state_root.join("upper").join(layer.id.as_str());
            let work = self.state_root.join("work").join(layer.id.as_str());
            mkdir_all(&upper)?;
            mkdir_all(&work)?;
            mount::fsconfig_string(fs.as_fd(), vocab::OPT_UPPERDIR, &upper.display().to_string())?;
            mount::fsconfig_string(fs.as_fd(), vocab::OPT_WORKDIR, &work.display().to_string())?;
        }
        mount::fsconfig_create(fs.as_fd())?;
        let mnt = mount::fsmount(fs.as_fd())?;
        if let Some(ns) = userns {
            mount::idmap_mount(mnt.as_fd(), ns)?;
        }
        mount::move_mount(mnt.as_fd(), &target.display().to_string())?;
        mounts.record(target);
        Ok(mounts)
    }

    // best-effort reverse of mount_root. the in-ns mounts die with the pin; we
    // reclaim the lower mountpoint and any ephemeral scratch, keep persistent
    // upper/work
    fn unmount_root(&self, layer: &LayerDescriptor, mounts: &Mounts) {
        for point in mounts.teardown_order() {
            let _ = nsproc::unmount_detach(&point.display().to_string());
        }
        if layer.flags.ephemeral_upper() {
            let _ = std::fs::remove_dir_all(self.ephemeral_base(layer));
        }
        let _ = std::fs::remove_dir_all(self.lower(layer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{LayerFlags, LayerId, LayerType, Libc, Limits, Sandbox};

    fn desc(id: &str) -> LayerDescriptor {
        LayerDescriptor {
            id: LayerId::new(id).unwrap(),
            r#type: LayerType::Native,
            priority: 1,
            libc: Libc { name: "glibc".into(), loader: Some("/lib/ld.so".into()) },
            flags: LayerFlags::default(),
            sandbox: Sandbox::default(),
            resources: Limits::default(),
        }
    }

    #[test]
    fn image_is_flat_erofs_in_store() {
        let b = ErofsBackend::new("/store", "/state");
        // the lower is a single flat image named by the layer id
        assert_eq!(b.image(&desc("void")), Path::new("/store/void.erofs"));
    }

    #[test]
    fn lower_mountpoint_is_per_layer_under_state() {
        let b = ErofsBackend::new("/store", "/state");
        // the image is mounted at a per-layer lower, kept out of the store
        assert_eq!(b.lower(&desc("void")), Path::new("/state/lower/void"));
    }

    #[test]
    fn images_and_lowers_are_distinct_per_layer() {
        let b = ErofsBackend::new("/store", "/state");
        assert_ne!(b.image(&desc("void")), b.image(&desc("debian")));
        assert_ne!(b.lower(&desc("void")), b.lower(&desc("debian")));
    }

    #[test]
    fn unmount_root_reclaims_the_lower_mountpoint() {
        let tmp = crate::tmp::TmpDir::new("erofs-teardown");
        let b = ErofsBackend::new(tmp.join("store"), tmp.join("state"));
        let d = desc("void");
        // stand in for the per-layer erofs mountpoint mount_root would create
        let lower = b.lower(&d);
        mkdir_all(&lower).unwrap();
        let mut m = Mounts::new();
        m.record(&lower);
        b.unmount_root(&d, &m);
        assert!(!lower.exists(), "erofs lower mountpoint must be reclaimed");
    }

    #[test]
    fn unmount_root_reclaims_ephemeral_scratch() {
        let tmp = crate::tmp::TmpDir::new("erofs-ephemeral");
        let b = ErofsBackend::new(tmp.join("store"), tmp.join("state"));
        let mut d = desc("void");
        d.flags.ephemeral = true;
        // stand in for the tmpfs scratch mount_root would lay down
        let base = b.ephemeral_base(&d);
        mkdir_all(&base).unwrap();
        let mut m = Mounts::new();
        m.record(&base);
        b.unmount_root(&d, &m);
        assert!(!base.exists(), "erofs ephemeral scratch must be reclaimed");
    }
}
