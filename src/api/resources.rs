// data so policy lives in config, not code; the core writes them to the
// cgroup scope at launch. empty means accounting without caps

/// per-layer resource limits applied to the cgroup v2 scope at launch.
/// every field is optional; a missing field means no limit is set
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    /// memory.max, in bytes. none leaves it at "max"
    pub memory_max: Option<u64>,
    /// pids.max, a task count. none leaves it at "max"
    pub pids_max: Option<u64>,
    /// cpu.weight, relative share in 1..=10000. none leaves the default
    pub cpu_weight: Option<u32>,
    /// cpu.max, an absolute quota per period. none leaves it at "max"
    pub cpu_max: Option<CpuMax>,
}

/// an absolute cpu cap: quota microseconds runnable per period microseconds.
/// constructed only through new(), which rejects zero values that the kernel
/// would reject with a cryptic error
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuMax {
    quota_us: u64,
    period_us: u64,
}

impl CpuMax {
    pub fn new(quota_us: u64, period_us: u64) -> Result<Self, String> {
        if period_us == 0 {
            return Err("cpu.max period must be at least 1us".into());
        }
        if quota_us == 0 {
            return Err("cpu.max quota must be at least 1us (use none for unlimited)".into());
        }
        Ok(CpuMax { quota_us, period_us })
    }

    pub fn quota_us(&self) -> u64 {
        self.quota_us
    }

    pub fn period_us(&self) -> u64 {
        self.period_us
    }
}

impl Limits {
    /// true when no limits are set. the scope is still created for accounting;
    /// it just carries no ceilings
    pub fn is_empty(&self) -> bool {
        self.memory_max.is_none()
            && self.pids_max.is_none()
            && self.cpu_weight.is_none()
            && self.cpu_max.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_by_default() {
        assert!(Limits::default().is_empty());
    }

    #[test]
    fn any_field_makes_it_non_empty() {
        let l = Limits { pids_max: Some(64), ..Default::default() };
        assert!(!l.is_empty());
    }

    #[test]
    fn cpumax_rejects_zero() {
        assert!(CpuMax::new(0, 100_000).is_err());
        assert!(CpuMax::new(50_000, 0).is_err());
        assert!(CpuMax::new(50_000, 100_000).is_ok());
    }
}
