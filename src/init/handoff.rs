use crate::api::{Error, Layer, Result};
use crate::exec;
use crate::init::signals;
use std::os::unix::process::CommandExt;
use std::process::Command;

// reject an empty init path before entering the namespace because
// exec("") would fail with ENOENT after we've already crossed the ns boundary
pub fn handoff(layer: &Layer, native_init: &str) -> Result<()> {
    if native_init.is_empty() {
        return Err(Error::Init("native init path is empty".into()));
    }
    exec::enter(layer)?;
    signals::unblock_signals()?;
    let err = Command::new(native_init).exec();
    Err(Error::Init(format!("exec init {native_init}: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_init_rejected_before_entering() {
        // a real Layer is unnecessary: the empty check runs before exec::enter,
        // so this returns the State error without touching the namespace
        let layer = Layer {
            desc: crate::api::LayerDescriptor {
                id: crate::api::LayerId::new("void").unwrap(),
                r#type: crate::api::LayerType::Native,
                priority: 1,
                libc: crate::api::Libc { name: "static".into(), loader: None },
                flags: Default::default(),
                sandbox: Default::default(),
                resources: Default::default(),
            },
            ns_path: std::path::PathBuf::from("/nonexistent/ns"),
        };
        let err = handoff(&layer, "").unwrap_err();
        assert!(err.to_string().contains("native init path is empty"), "got: {err}");
    }
}
