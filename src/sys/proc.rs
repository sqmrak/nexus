// fork-safety: multi-threaded fork is UB (child may inherit locked mutexes),
// so assert single-threadedness before every fork via thread_count from /proc
pub(crate) fn thread_count() -> Option<usize> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // comm (field 2) is parenthesized and may contain spaces; split past it so
    // the rest is positional. num_threads is the 18th token after the paren
    stat.rsplit_once(')')?.1.split_whitespace().nth(17)?.parse().ok()
}

// panics if any thread beyond the caller exists. libtest runs multi-threaded,
// so this is skipped in test builds; in release builds a multi-threaded fork
// is UB and must be caught at the call site
pub(crate) fn assert_fork_safe() {
    #[cfg(not(test))]
    assert!(
        thread_count().is_none_or(|n| n <= 1),
        "fork() from a multi-threaded process: child may deadlock or hit UB"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_at_least_the_current_thread() {
        assert!(thread_count().is_some_and(|n| n >= 1));
    }

    #[test]
    fn sees_a_spawned_thread() {
        use std::sync::mpsc;
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let h = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            // park until released, so the thread is provably alive at the check
            let _ = release_rx.recv();
        });
        started_rx.recv().unwrap();
        assert!(thread_count().is_some_and(|n| n >= 2));
        drop(release_tx);
        h.join().unwrap();
    }
}
