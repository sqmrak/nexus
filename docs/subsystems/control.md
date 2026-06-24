# the control daemon

the control daemon is a management socket over the core. the hot path never
touches it: a launch opens the pinned namespace file and `setns` directly. the
daemon exists so tooling can warm, evict and list layers out of band. it is
socket-activation friendly. the mechanism lives in `control/proto.rs` and
`control/serve.rs`.

## the protocol

a human-readable text protocol, one request line in, one reply terminated by a
blank line out. it carries no file descriptors, because the hot path does not go
through the socket; there is nothing to pass.

requests:

```
build <layer>   build and pin the layer's namespace now (build it)
evict <layer>   drop the layer's namespace, unmounting its pin
list            list the built layers
status          daemon liveness and the built count
```

replies, each terminated by a blank line:

```
ok\n\n                       success, no data
line1\nline2\n\n             a list of lines
err <message>\n\n            failure with a message
```

`Request::parse` splits on whitespace, takes the verb and an optional argument,
and rejects a missing argument or an unknown verb with a config error.
`Request::encode` and `Reply::encode` produce the wire forms above.

## dispatch

each request maps onto one `Core` operation:

```
build <id>  >  core.build(&id)       >  ok | err
evict <id>  >  core.evict(&id)      >  ok        (always; an unknown id is a no-op)
list        >  core.built_layers()  >  lines
status      >  "built <n>"          >  lines
```

`build` builds and pins the namespace ahead of use, so the first user-visible
launch of that layer is already hot (it skips the cold compose described in
[run](../paths/run.md)). `evict` drops the namespace and its resource scope; a
later run rebuilds it.

## the serve loop

```
listener nonblocking
loop until stop:
  poll(listener, 200ms)
  on timeout            >  recheck the stop flag, loop
  on WOULDBLOCK / EINTR >  continue
  on a connection       >  read one line, parse, dispatch, write the reply
```

the non-blocking listener with a 200 ms `poll` timeout lets the loop notice the
shutdown flag even while idle. a bad connection is handled and dropped; it never
takes the daemon down.

shutdown is async-signal-safe. `install_signal_stop` installs `SIGINT` and
`SIGTERM` handlers that do nothing but an atomic store, with no `SA_RESTART` so
the in-flight `poll` is interrupted and the loop rechecks the flag promptly.
installing the handlers is best-effort: if `sigaction` fails the daemon still
serves, it just relies on the cooperative `Shutdown` trigger.

## socket activation

`listener(path)` first tries `activated()`, which reads `LISTEN_FDS` and takes
fd 3 (the first socket-activation fd after stdio) as an already-bound listening
socket. if there is no activation it removes a stale socket file and binds a new
one at `path`. this lets a supervisor (systemd or equivalent) pass the socket in
and start the daemon lazily on first connection.
