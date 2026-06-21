// runs after setns/idmap because those operations need privilege; runs
// before seccomp so the filter drops from an already-unprivileged state

use crate::api::{Cap, Error, Result};
use rustix::thread::{
    clear_ambient_capability_set, configure_capability_in_ambient_set,
    remove_capability_from_bounding_set, set_capabilities, CapabilitySet, CapabilitySets,
};

// reduce every capability set to exactly `keep`. an empty `keep` (the default)
// leaves the process with no capabilities at all
pub fn drop_to(keep: &[Cap]) -> Result<()> {
    let keep_set = keep.iter().fold(CapabilitySet::empty(), |s, c| s | bit(*c));

    // drop the bounding set first, while we still hold CAP_SETPCAP in the
    // effective set: PR_CAPBSET_DROP needs it, and capset below removes it
    for n in 0..=cap_last_cap() {
        let c = CapabilitySet::from_bits_retain(1u64 << n);
        if !keep_set.contains(c) {
            remove_capability_from_bounding_set(c).map_err(|e| Error::sys("capbset_drop", e))?;
        }
    }

    // now restrict the thread's sets to `keep`. raising a cap into ambient
    // below requires it to be in both permitted and inheritable, so set those
    set_capabilities(
        None,
        CapabilitySets { effective: keep_set, permitted: keep_set, inheritable: keep_set },
    )
    .map_err(|e| Error::sys("capset", e))?;

    // clear ambient, then re-raise only the kept caps so they survive execve
    // into an ordinary binary that carries no file capabilities
    clear_ambient_capability_set().map_err(|e| Error::sys("cap_ambient_clear", e))?;
    for c in keep {
        configure_capability_in_ambient_set(bit(*c), true)
            .map_err(|e| Error::sys("cap_ambient_raise", e))?;
    }
    Ok(())
}

// the single-bit set for one capability number
fn bit(c: Cap) -> CapabilitySet {
    CapabilitySet::from_bits_retain(1u64 << c.raw())
}

// clamped to 63 because CapabilitySet is a u64 bitmask; 1u64 << n past 63
// would overflow. falls back to 40 (5.12 floor) if /proc is missing
fn cap_last_cap() -> u8 {
    std::fs::read_to_string("/proc/sys/kernel/cap_last_cap")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(40)
        .min(63)
}
