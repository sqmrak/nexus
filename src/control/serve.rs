// management daemon: build/evict/list/status over a unix socket
// supervision is external; the daemon only accepts and dispatches

use crate::api::Result;
use crate::control::proto::{Reply, Request};
use crate::core::Core;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

// stop flag for serve(). set by signal handler or another thread
#[derive(Clone, Default)]
pub struct Shutdown(Arc<AtomicBool>);

impl Shutdown {
    pub fn new() -> Self {
        Shutdown(Arc::new(AtomicBool::new(false)))
    }

    // ask the serve loop to stop after its next wakeup
    pub fn trigger(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_triggered(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

// process-wide stop flag for the signal handler. async-signal-safe: only
// an atomic store. the serve loop reads it through Shutdown
static SIGNAL_STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_term(_sig: i32) {
    SIGNAL_STOP.store(true, Ordering::SeqCst);
}

// install SIGINT/SIGTERM via sigaction. returns a Shutdown for serve()
// best effort: if sigaction fails the daemon still serves
pub fn install_signal_stop() -> Shutdown {
    let flag = Shutdown::new();
    for sig in [libc::SIGINT, libc::SIGTERM] {
        // no SA_RESTART so poll() is interrupted (EINTR) and the loop rechecks
        // the shutdown flag; the handler is async-signal-safe (one atomic store)
        let mut sa: libc::sigaction = unsafe { std::mem::zeroed() };
        sa.sa_sigaction = on_term as *const () as usize;
        // safe: sa is valid, oldact is null
        unsafe { libc::sigaction(sig, &sa, std::ptr::null_mut()) };
    }
    flag
}

// take a listener from the supervisor (LISTEN_FDS) or bind path ourselves
pub fn listener(path: &str) -> Result<UnixListener> {
    if let Some(l) = activated() {
        return Ok(l);
    }
    let _ = std::fs::remove_file(path);
    UnixListener::bind(path).map_err(|e| crate::api::Error::Io(format!("bind {path}: {e}")))
}

// the first socket-activation fd is 3 (after stdio). present when
// LISTEN_FDS is set by the supervisor
fn activated() -> Option<UnixListener> {
    let n: i32 = std::env::var("LISTEN_FDS").ok()?.parse().ok()?;
    if n < 1 {
        return None;
    }
    // safe: the supervisor guarantees fd 3 is a listening unix socket
    let fd = unsafe { OwnedFd::from_raw_fd(3) };
    Some(UnixListener::from(fd))
}

// accept-until-stop. non-blocking listener + poll() timeout lets the loop
// check the shutdown flag even while idle
pub fn serve(listener: &UnixListener, core: &mut Core, stop: &Shutdown) -> Result<()> {
    listener
        .set_nonblocking(true)
        .map_err(|e| crate::api::Error::Io(format!("nonblocking: {e}")))?;

    let stopping = || stop.is_triggered() || SIGNAL_STOP.load(Ordering::SeqCst);

    while !stopping() {
        // wait up to a beat for a connection; wake to recheck the flag
        if !wait_readable(listener, Duration::from_millis(200)) {
            continue;
        }
        match listener.accept() {
            Ok((conn, _)) => {
                conn.set_nonblocking(false).ok();
                // a bad connection must not take the daemon down
                let _ = handle(conn, core);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(crate::api::Error::Io(format!("accept: {e}"))),
        }
    }
    Ok(())
}

// poll the listener fd for POLLIN. true if readable, false on timeout
fn wait_readable(listener: &UnixListener, timeout: Duration) -> bool {
    let mut pfd = libc::pollfd { fd: listener.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    // safe: pfd points at one valid pollfd for the duration of the call
    let r = unsafe { libc::poll(&mut pfd, 1, ms) };
    r > 0 && (pfd.revents & libc::POLLIN) != 0
}

fn handle(stream: UnixStream, core: &mut Core) -> Result<()> {
    // a client that connects and then stalls must not pin the single-threaded
    // accept loop. bound both directions so a silent peer times out
    let t = Some(Duration::from_secs(5));
    stream.set_read_timeout(t).map_err(io_err)?;
    stream.set_write_timeout(t).map_err(io_err)?;
    let mut reader = BufReader::new(stream.try_clone().map_err(io_err)?);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(io_err)?;

    let reply = match Request::parse(line.trim()) {
        Ok(req) => dispatch(req, core),
        Err(e) => Reply::Err(e.to_string()),
    };
    let mut stream = stream;
    stream.write_all(reply.encode().as_bytes()).map_err(io_err)?;
    Ok(())
}

fn dispatch(req: Request, core: &mut Core) -> Reply {
    match req {
        Request::Build(id) => match core.build(&id) {
            Ok(()) => Reply::Ok,
            Err(e) => Reply::Err(e.to_string()),
        },
        Request::Evict(id) => {
            core.evict(&id);
            Reply::Ok
        }
        Request::List => Reply::Lines(core.built_layers()),
        Request::Status => Reply::Lines(vec![format!("built {}", core.built_count())]),
    }
}

fn io_err(e: std::io::Error) -> crate::api::Error {
    crate::api::Error::Io(format!("io: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Layout;
    use crate::state::System;
    use std::time::Instant;

    fn empty_core(dir: &std::path::Path) -> Core {
        Core::open(Layout::new(dir), Vec::new(), System::default())
    }

    #[test]
    fn shutdown_flag_round_trips() {
        let s = Shutdown::new();
        assert!(!s.is_triggered());
        s.trigger();
        assert!(s.is_triggered());
        // a clone shares the flag
        assert!(s.clone().is_triggered());
    }

    #[test]
    fn serve_returns_when_pre_triggered() {
        let tmp = crate::tmp::TmpDir::new("serve-stop");
        let sock = tmp.join("ctl.sock");
        let listener = listener(&sock.display().to_string()).unwrap();
        let mut core = empty_core(tmp.path());

        // flag already set: serve must notice and return almost immediately
        // rather than blocking on accept forever
        let stop = Shutdown::new();
        stop.trigger();

        let start = Instant::now();
        serve(&listener, &mut core, &stop).unwrap();
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn serve_stops_when_triggered_while_idle() {
        let tmp = crate::tmp::TmpDir::new("serve-idle");
        let sock = tmp.join("ctl.sock");
        let listener = listener(&sock.display().to_string()).unwrap();
        let stop = Shutdown::new();
        let stop2 = stop.clone();

        // the daemon idles with no clients; trigger from another thread
        let dir = tmp.path().to_path_buf();
        let h = std::thread::spawn(move || {
            let mut core = empty_core(&dir);
            serve(&listener, &mut core, &stop2)
        });
        std::thread::sleep(std::time::Duration::from_millis(300));
        stop.trigger();
        // the loop should observe the flag within a poll beat and return
        let joined = h.join().unwrap();
        assert!(joined.is_ok());
    }
}
