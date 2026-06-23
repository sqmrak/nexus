// enter a layer and exec. open the persisted namespace, setns, then execve

mod launch;

pub use launch::SetnsExec;

use crate::api::{Error, Layer, Result};
use crate::sys::nsproc;
use std::fs::File;
use std::os::fd::AsFd;

/// enter a built layer by opening its namespace file and calling setns.
/// caller execs afterwards, already inside the layer
pub fn enter(layer: &Layer) -> Result<()> {
    let f = File::open(&layer.ns_path)
        .map_err(|e| Error::Init(format!("open ns {}: {e}", layer.ns_path.display())))?;
    nsproc::setns(f.as_fd())?;
    Ok(())
}
