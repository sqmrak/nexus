// block all signals at startup, unblock just before exec(native_init)
// keeps pid 1 alive until handoff; the real init inherits a clean mask

use crate::api::Result;
use crate::sys::nsproc;

// SIGCHLD stays unblocked because pid 1 must reap the children it adopts;
// SIGKILL/SIGSTOP are unblockable by kernel fiat
const KEEP_UNBLOCKED: &[i32] = &[libc::SIGCHLD];

// called before early_mounts so no window exists where a stray signal
// can kill pid 1 before it hands off to native init
pub fn block_signals() -> Result<()> {
    nsproc::block_signals_except(KEEP_UNBLOCKED)
}

// the signal mask survives execve, so the new init would silently miss
// signals it means to handle; unblock in the same process that execs
pub fn unblock_signals() -> Result<()> {
    nsproc::unblock_all_signals()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_blocked(sig: i32) -> bool {
        // safe: cur is a valid sigset_t; we only query the current mask
        let mut cur: libc::sigset_t = unsafe { std::mem::zeroed() };
        unsafe { libc::sigprocmask(libc::SIG_BLOCK, std::ptr::null(), &mut cur) };
        unsafe { libc::sigismember(&cur, sig) == 1 }
    }

    // runs in a forked child so it does not leave the test process with a
    // mangled signal mask. the child encodes its findings in the exit code
    #[test]
    fn block_keeps_sigchld_and_masks_the_rest() {
        // safe: child only does signal syscalls then _exits
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            let ok = block_signals().is_ok()
                && is_blocked(libc::SIGTERM)
                && is_blocked(libc::SIGINT)
                && is_blocked(libc::SIGHUP)
                && !is_blocked(libc::SIGCHLD)
                // unblock restores a clean mask
                && unblock_signals().is_ok()
                && !is_blocked(libc::SIGTERM)
                && !is_blocked(libc::SIGCHLD);
            unsafe { libc::_exit(if ok { 0 } else { 1 }) };
        }
        let mut status = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0);
    }
}
