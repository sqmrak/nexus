# nexus

core for meta-distributions. pure mechanism: no fuse, no path rewriting, 
no per-distro branching. layers are composed from a content-addressed
store with kernel primitives alone (mount namespaces, overlay, erofs, idmapped
mounts, landlock, seccomp, cgroup v2), then run or booted as pid 1

## docs

https://sqmrak.github.io/nexus

## thanks

bedrock linux showed that one machine can run many distributions at once, and
that the idea is worth doing well. nexus owes it the whole premise. where
bedrock composes in userspace through fuse, nexus composes before launch so the
kernel handles every path natively; that is a different trade, not a criticism
of the project that charted the territory first

## license

GPL-2.0.
