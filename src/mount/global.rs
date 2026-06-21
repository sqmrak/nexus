use crate::api::Result;
use crate::paths;
use crate::sys::nsproc;
use std::path::Path;

// absent globals (e.g. /boot in a namespace) are skipped, not fatal,
// so the bind path works across caller and guest profiles
pub fn bind_globals(target: &Path) -> Result<()> {
    for g in paths::GLOBALS {
        if !Path::new(g).exists() {
            continue;
        }
        let dst = target.join(g.trim_start_matches('/'));
        std::fs::create_dir_all(&dst).ok();
        nsproc::bind(g, &dst.display().to_string(), true)?;
    }
    Ok(())
}
