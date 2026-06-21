// reaps orphans during boot (early_mounts, compose). a dedicated thread
// polls waitpid(-1, WNOHANG); the real init takes over after handoff

use crate::api::{Error, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

const SWEEP: Duration = Duration::from_millis(50);

// a running reaper. dropping it (or calling stop) drains any remaining dead
// children and exits the thread. held for the boot window only
pub struct Reaper {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Reaper {
    // spawn before early_mounts so no child dies unreaped during boot. a
    // thread spawn failure here is fatal: orphans would zombie under pid 1
    pub fn spawn() -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let handle = std::thread::Builder::new()
            .name("nexus-reaper".into())
            // tiny stack: the thread only calls waitpid and sleeps
            .stack_size(16 * 1024)
            .spawn(move || reap_loop(&stop_thread))
            .map_err(|e| Error::Init(format!("spawn reaper thread: {e}")))?;
        Ok(Reaper { stop, handle: Some(handle) })
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
            // drain once more after the join: a child can die between the
            // last sweep and the thread's exit
            drain();
        }
    }
}

impl Drop for Reaper {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn reap_loop(stop: &AtomicBool) {
    loop {
        let stopping = stop.load(Ordering::SeqCst);
        drain();
        if stopping {
            return;
        }
        std::thread::sleep(SWEEP);
    }
}

// reap every already-exited child. non-blocking; returns when none are
// waiting (waitpid yields 0 with children alive, -1/ECHILD with none)
fn drain() {
    loop {
        let mut status = 0;
        // safe: waitpid on any child (-1), WNOHANG, valid out param
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if pid <= 0 {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // count zombie children of pid `me` via /proc
    fn zombie_children_of(me: i32) -> usize {
        let me = me.to_string();
        let mut n = 0;
        if let Ok(rd) = std::fs::read_dir("/proc") {
            for e in rd.flatten() {
                if let Ok(s) = std::fs::read_to_string(e.path().join("stat")) {
                    if let Some(rest) = s.rsplit(')').next() {
                        let f: Vec<&str> = rest.split_whitespace().collect();
                        if f.len() >= 2 && f[0] == "Z" && f[1] == me {
                            n += 1;
                        }
                    }
                }
            }
        }
        n
    }

    // waitpid(-1) steals from other tests → each scenario in its own fork
    fn in_subprocess(body: fn() -> bool) -> bool {
        // safe: child runs the closure then _exits; parent only waitpids it
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            let ok = body();
            unsafe { libc::_exit(if ok { 0 } else { 1 }) };
        }
        let mut status = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0
    }

    #[test]
    fn reaps_children_that_die_during_its_life() {
        assert!(in_subprocess(|| {
            let r = Reaper::spawn().unwrap();
            for _ in 0..16 {
                if unsafe { libc::fork() } == 0 {
                    unsafe { libc::_exit(0) };
                }
            }
            std::thread::sleep(Duration::from_millis(300));
            let z = zombie_children_of(std::process::id() as i32);
            r.stop();
            z == 0
        }));
    }

    #[test]
    fn final_drain_collects_a_child_that_died_before_stop() {
        assert!(in_subprocess(|| {
            let r = Reaper::spawn().unwrap();
            if unsafe { libc::fork() } == 0 {
                unsafe { libc::_exit(0) };
            }
            std::thread::sleep(Duration::from_millis(120));
            r.stop();
            zombie_children_of(std::process::id() as i32) == 0
        }));
    }
}
