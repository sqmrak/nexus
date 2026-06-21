// zero-alloc tracing for the syscall hot path, off by default (enable with
// --features trace). trace_step! logs milestones, trace_field! logs values

#[cfg(feature = "trace")]
macro_rules! trace_step {
    ($msg:expr $(,)?) => {{
        eprintln!("nexus: {}", $msg);
    }};
}

#[cfg(not(feature = "trace"))]
macro_rules! trace_step {
    ($msg:expr $(,)?) => {{}};
}

#[cfg(feature = "trace")]
macro_rules! trace_field {
    ($fmt:expr, $($arg:expr),+ $(,)?) => {{
        eprintln!(concat!("nexus: ", $fmt), $($arg),+);
    }};
}

#[cfg(not(feature = "trace"))]
macro_rules! trace_field {
    ($fmt:expr, $($arg:expr),+ $(,)?) => {{
        $(let _ = &$arg;)+
    }};
}
