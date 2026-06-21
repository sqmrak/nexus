// apply sandbox after setns, before exec: the order
// NNP->landlock->drop caps->seccomp ensures restrictions inherit across execve

use crate::api::{Error, Result, Sandbox, Seccomp};
use landlock::{
    Access, AccessFs, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset, RulesetAttr,
    RulesetCreatedAttr, ABI,
};

pub fn apply(sb: &Sandbox) -> Result<()> {
    trace_field!("apply sandbox: keep_caps={:?}, seccomp={:?}", sb.keep_caps, sb.seccomp);
    // no_new_privs is unconditional: landlock requires it and it stops
    // setuid escalation via execve
    no_new_privs()?;
    if !sb.read.is_empty() || !sb.write.is_empty() {
        landlock(sb)?;
    }
    // drop privileges before the filter, so seccomp guards an unprivileged
    // process. unconditional: an empty keep-list drops every capability
    crate::sys::caps::drop_to(&sb.keep_caps)?;
    match sb.seccomp {
        Seccomp::Off => {}
        Seccomp::Baseline => seccomp_baseline()?,
    }
    Ok(())
}

// set PR_SET_NO_NEW_PRIVS so no execve in this process or its children can
// gain privileges. a hard requirement for landlock and the seccomp filter
fn no_new_privs() -> Result<()> {
    rustix::thread::set_no_new_privs(true).map_err(|e| Error::sys("no_new_privs", e))
}

// target the newest landlock ABI with BestEffort so every kernel gets the
// strongest rules it supports, never failing on older ones
const TARGET_LANDLOCK: ABI = ABI::V7;

fn landlock(sb: &Sandbox) -> Result<()> {
    let ll = TARGET_LANDLOCK;
    let read = AccessFs::from_read(ll);
    let rw = AccessFs::from_all(ll);

    let mut rs = Ruleset::default()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(rw)
        .map_err(|e| Error::Io(format!("landlock handle: {e}")))?
        .create()
        .map_err(|e| Error::Io(format!("landlock create: {e}")))?;

    for p in &sb.read {
        let fd = PathFd::new(p).map_err(|e| Error::Io(format!("path {p:?}: {e}")))?;
        rs = rs
            .add_rule(PathBeneath::new(fd, read))
            .map_err(|e| Error::Io(format!("landlock rule: {e}")))?;
    }
    for p in &sb.write {
        let fd = PathFd::new(p).map_err(|e| Error::Io(format!("path {p:?}: {e}")))?;
        rs = rs
            .add_rule(PathBeneath::new(fd, rw))
            .map_err(|e| Error::Io(format!("landlock rule: {e}")))?;
    }
    rs.restrict_self().map_err(|e| Error::Io(format!("landlock restrict: {e}")))?;
    Ok(())
}

// block a small set of universally dangerous syscalls. the allowlist of a
// real profile is policy; this is the core's safe default
fn seccomp_baseline() -> Result<()> {
    use seccompiler::{apply_filter, BpfProgram, SeccompAction, SeccompFilter};

    let (cpu, denied) =
        deny_list().ok_or_else(|| Error::Io("seccomp: unsupported target cpu".into()))?;

    // deny by exception: allow everything, trap the dangerous few
    let rules = denied.iter().map(|&nr| (nr, vec![])).collect::<std::collections::BTreeMap<_, _>>();

    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,    // default: allow
        SeccompAction::Errno(1), // matched: fail with EPERM
        cpu,
    )
    .map_err(|e| Error::Io(format!("seccomp build: {e}")))?;

    let prog: BpfProgram =
        filter.try_into().map_err(|e| Error::Io(format!("seccomp compile: {e}")))?;
    apply_filter(&prog).map_err(|e| Error::Io(format!("seccomp apply: {e}")))
}

// one row per syscall covers x86_64 and aarch64 so the two arches cannot
// drift apart. the calls reconfigure the process or load kernel code
const DENY: &[(&str, i64, i64)] = &[
    ("mount", 165, 40),
    ("umount2", 166, 39),
    ("kexec_load", 246, 104),
    ("init_module", 175, 105),
    ("finit_module", 313, 273),
    ("delete_module", 176, 106),
];

// the target cpu and its deny-list numbers; returns None on unsupported arch
fn deny_list() -> Option<(seccompiler::TargetArch, Vec<i64>)> {
    use seccompiler::TargetArch;
    #[cfg(target_arch = "x86_64")]
    return Some((TargetArch::x86_64, DENY.iter().map(|&(_, x, _)| x).collect()));
    #[cfg(target_arch = "aarch64")]
    return Some((TargetArch::aarch64, DENY.iter().map(|&(_, _, a)| a).collect()));
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_list_is_nonempty_on_supported_arch() {
        // on the arches we ship numbers for, the baseline must actually deny
        // something; an empty list would silently disable the filter
        if let Some((_, list)) = deny_list() {
            assert!(!list.is_empty(), "baseline deny list is empty");
            // every row maps to a distinct number on this arch
            let mut sorted = list.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(sorted.len(), list.len(), "duplicate syscall numbers in deny list");
        }
    }

    #[test]
    fn deny_table_covers_both_arches() {
        // each row carries a number for both arches, so the two never drift
        for (name, x, a) in DENY {
            assert!(*x > 0, "x86_64 number missing for {name}");
            assert!(*a > 0, "aarch64 number missing for {name}");
        }
    }
}
