# errors

every fallible operation that crosses the core boundary returns
`Result<T> = std::result::Result<T, Error>`. the error variant is the stable tag
a policy matches on; a failed syscall additionally keeps its raw errno so log
and anyhow adapters see the real os error. the type lives in `api/err.rs`.

## the Error enum

- `Sys { op: &'static str, errno: i32, source: Arc<io::Error> }` - a failed
  syscall, tagged with the op that failed.
- `UnknownLayer(String)` - the named layer is not in the loaded descriptors.
- `NoHealthyLayer` - every candidate is unhealthy; nothing can be selected to
  boot.
- `Corrupt { hash: String }` - a store object failed its blake3 integrity
  check.
- `Config(String)` - toml parse error, invalid value, or unknown token in
  state.
- `Io(String)` - a filesystem op (mkdir, open, write) that is not a syscall.
- `Init(String)` - a boot-path failure: handoff, early_mounts, reaper, exec.
- `Libc(String)` - the libc loader was not found, or the wrong libc for a
  layer.

## why Sys keeps the errno

`Sys` carries the errno as a plain `i32` for cheap matching, and wraps the
`std::io::Error` in an `Arc` so the variant stays `Clone`. it exposes the real
os error through `std::error::Error::source`, so adapters that walk the source
chain (anyhow, log) see the underlying errno. the string variants carry no
structured cause and return no source.

`Error::sys(op, errno)` is the internal constructor that tags a failed syscall
with the static op name, for example `Error::sys("setns", e)`. the op name is
what shows up in a log line, so it names the exact syscall that failed.

## matching

because the variant is the contract, policy can branch on it without parsing
strings:

```rust
match core.boot("/sbin/init") {
    Ok(()) => unreachable!("boot replaces the process on success"),
    Err(nexus::Error::NoHealthyLayer) => fall_back_to_rescue(),
    Err(nexus::Error::Sys { op, errno, .. }) => log_syscall(op, errno),
    Err(e) => report(e),
}
```

`NoHealthyLayer` and `Corrupt` are the two variants a meta-distro is most likely
to recover from: the first by booting a rescue layer, the second by
re-importing or rolling back. `Init` always means the boot path could not
complete and the system did not come up.
