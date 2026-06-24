use crate::api::{LayerDescriptor, Mounts, Result, StoreBackend};
use crate::paths::{mkdir_all, require_no_symlink_ancestor};
use crate::sys::{mount, nsproc};
use crate::vocab;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};

pub struct OverlayBackend {
    store_root: PathBuf,
    // writable state root, sibling of the store (upper and work live here)
    state_root: PathBuf,
}

impl OverlayBackend {
    pub fn new(store_root: impl Into<PathBuf>, state_root: impl Into<PathBuf>) -> Self {
        OverlayBackend { store_root: store_root.into(), state_root: state_root.into() }
    }

    // layer tree plus optional shared base. shadowed layers carry a full
    // rootfs; forge layers may share a base. overlay rejects missing paths
    fn lowerdir(&self, layer: &LayerDescriptor) -> String {
        let usr = self.store_root.join(layer.id.as_str());
        let base = self.store_root.join("base");
        // a layer named "base" is its own base: listing the dir
        // twice is an overlapping lower, which overlay rejects with ELOOP
        if base.is_dir() && base != usr {
            format!("{}:{}", usr.display(), base.display())
        } else {
            usr.display().to_string()
        }
    }

    fn upperdir(&self, layer: &LayerDescriptor) -> PathBuf {
        self.state_root.join("upper").join(layer.id.as_str())
    }

    fn workdir(&self, layer: &LayerDescriptor) -> PathBuf {
        self.state_root.join("work").join(layer.id.as_str())
    }

    // one tmpfs mounted here with upper and work subdirs (overlay requires
    // them on the same filesystem)
    fn ephemeral_base(&self, layer: &LayerDescriptor) -> PathBuf {
        self.state_root.join("ephemeral").join(layer.id.as_str())
    }

    // mount a fresh tmpfs for the layer and lay out its upper and work dirs
    // inside it. returns the two paths as strings for the overlay config
    fn tmpfs_upper(&self, layer: &LayerDescriptor) -> Result<(String, String)> {
        let base = self.ephemeral_base(layer);
        require_no_symlink_ancestor(&base, &self.state_root)?;
        mkdir_all(&base)?;
        mount::mount_tmpfs(&base.display().to_string())?;
        let upper = base.join("upper");
        let work = base.join("work");
        if let Err(e) = mkdir_all(&upper) {
            let _ = nsproc::unmount_detach(&base.display().to_string());
            return Err(e);
        }
        if let Err(e) = mkdir_all(&work) {
            let _ = nsproc::unmount_detach(&base.display().to_string());
            return Err(e);
        }
        Ok((upper.display().to_string(), work.display().to_string()))
    }
}

impl StoreBackend for OverlayBackend {
    fn mount_root(
        &self,
        layer: &LayerDescriptor,
        target: &Path,
        userns: Option<BorrowedFd<'_>>,
    ) -> Result<Mounts> {
        let mut mounts = Mounts::new();
        let fs = mount::fsopen(vocab::FS_OVERLAY)?;
        mount::fsconfig_string(fs.as_fd(), vocab::OPT_LOWERDIR, &self.lowerdir(layer))?;

        // upper by flag: atomic=none (read-only), ephemeral=tmpfs,
        // default=on-disk. atomic overrides ephemeral
        if layer.flags.ephemeral_upper() {
            let (upper, work) = self.tmpfs_upper(layer)?;
            mounts.record(self.ephemeral_base(layer));
            mount::fsconfig_string(fs.as_fd(), vocab::OPT_UPPERDIR, &upper)?;
            mount::fsconfig_string(fs.as_fd(), vocab::OPT_WORKDIR, &work)?;
        } else if !layer.flags.no_persistent_upper() {
            let upper = self.upperdir(layer);
            let work = self.workdir(layer);
            // verify no ancestor has been replaced with a symlink that would
            // divert the dirs outside the intended tree
            require_no_symlink_ancestor(&upper, &self.state_root)?;
            require_no_symlink_ancestor(&work, &self.state_root)?;
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

    // best-effort reverse of mount_root. the in-ns mounts die with the pin;
    // we reclaim the ephemeral tmpfs dir but keep a persistent upper/work
    fn unmount_root(&self, layer: &LayerDescriptor, mounts: &Mounts) {
        for point in mounts.teardown_order() {
            let _ = nsproc::unmount_detach(&point.display().to_string());
        }
        if layer.flags.ephemeral_upper() {
            let _ = std::fs::remove_dir_all(self.ephemeral_base(layer));
        }
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
    fn lower_is_layer_only_without_base() {
        let b = OverlayBackend::new("/store", "/state");
        // no base dir on disk, so the lower is just the layer tree
        assert_eq!(b.lowerdir(&desc("void")), "/store/void");
    }

    #[test]
    fn lower_puts_layer_before_base_when_present() {
        let tmp = crate::tmp::TmpDir::new("lower");
        let _ = std::fs::create_dir_all(tmp.join("base"));
        let b = OverlayBackend::new(tmp.path(), "/state");
        // the layer's own tree must win over the shared base, so it comes
        // first in the colon list
        let want = format!("{}/void:{}/base", tmp.display(), tmp.display());
        assert_eq!(b.lowerdir(&desc("void")), want);
    }

    #[test]
    fn lower_does_not_list_base_twice_for_a_base_named_layer() {
        let tmp = crate::tmp::TmpDir::new("lower-base");
        let _ = std::fs::create_dir_all(tmp.join("base"));
        let b = OverlayBackend::new(tmp.path(), "/state");
        // the layer's dir is the base dir: a single lower, not "base:base",
        // which overlay would reject as overlapping (ELOOP)
        assert_eq!(b.lowerdir(&desc("base")), format!("{}/base", tmp.display()));
    }

    #[test]
    fn upper_and_work_are_per_layer() {
        let b = OverlayBackend::new("/store", "/state");
        assert_eq!(b.upperdir(&desc("void")), Path::new("/state/upper/void"));
        assert_eq!(b.workdir(&desc("void")), Path::new("/state/work/void"));
    }

    #[test]
    fn ephemeral_base_is_per_layer_under_state() {
        let b = OverlayBackend::new("/store", "/state");
        assert_eq!(b.ephemeral_base(&desc("void")), Path::new("/state/ephemeral/void"));
    }

    #[test]
    fn ephemeral_layer_takes_tmpfs_upper_not_disk() {
        let mut d = desc("void");
        d.flags.ephemeral = true;
        // ephemeral wants a (tmpfs) upper, and that upper is not the
        // persistent on-disk one
        assert!(d.flags.ephemeral_upper());
        assert!(d.flags.no_persistent_upper());
    }

    #[test]
    fn atomic_overrides_ephemeral_no_upper() {
        let mut d = desc("void");
        d.flags.ephemeral = true;
        d.flags.atomic = true;
        // atomic takes no upper, even when ephemeral is also set
        assert!(!d.flags.ephemeral_upper());
        assert!(d.flags.no_persistent_upper());
    }

    #[test]
    fn unmount_root_reclaims_ephemeral_scratch() {
        let tmp = crate::tmp::TmpDir::new("ovl-teardown");
        let b = OverlayBackend::new(tmp.join("store"), tmp.join("state"));
        let mut d = desc("void");
        d.flags.ephemeral = true;
        // stand in for the tmpfs mountpoint mount_root would lay down
        let base = b.ephemeral_base(&d);
        mkdir_all(&base).unwrap();
        let mut m = Mounts::new();
        m.record(&base);
        b.unmount_root(&d, &m);
        assert!(!base.exists(), "ephemeral scratch must be reclaimed");
    }

    #[test]
    fn unmount_root_keeps_persistent_upper() {
        let tmp = crate::tmp::TmpDir::new("ovl-keep");
        let b = OverlayBackend::new(tmp.join("store"), tmp.join("state"));
        let d = desc("void");
        let (upper, work) = (b.upperdir(&d), b.workdir(&d));
        mkdir_all(&upper).unwrap();
        mkdir_all(&work).unwrap();
        b.unmount_root(&d, &Mounts::new());
        // evict bounds memory, not state: a rebuild must restore these writes
        assert!(upper.exists() && work.exists(), "persistent upper/work must survive evict");
    }
}
