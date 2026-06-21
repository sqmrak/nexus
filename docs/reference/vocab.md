# vocabulary

every string the on-disk format and the core agree on lives in one place,
`vocab.rs`, so a fork can retune a token without hunting through the code. this
chapter lists them. nothing here is a distro name; these are structural tokens
(types, flags, filesystem and cgroup interface names). the form below is
`token` (`CONSTANT`) - meaning.

## layer type tokens

- `native` (`TYPE_NATIVE`) - independently installed layer.
- `shadowed` (`TYPE_SHADOWED`) - the former root the meta-distro installed onto.
- `rescue` (`TYPE_RESCUE`) - guaranteed-healthy fallback.

## layer flag tokens

- `meta` (`FLAG_META`) - an imported meta-distro stratum.
- `non-fhs` (`FLAG_NON_FHS`) - non-standard layout (nixos, guix, gobolinux).
- `atomic` (`FLAG_ATOMIC`) - immutable, atomically updated.
- `ephemeral` (`FLAG_EPHEMERAL`) - lives until reboot.
- `pinned` (`FLAG_PINNED`) - version frozen (policy, core ignores).
- `hidden` (`FLAG_HIDDEN`) - not shown in launcher or PATH (policy).

## seccomp posture tokens

- `off` (`SECCOMP_OFF`).
- `baseline` (`SECCOMP_BASELINE`).

## backend tokens

- `overlay` (`BACKEND_OVERLAY`) - the overlay backend.
- `erofs` (`BACKEND_EROFS`) - flat erofs image plus a writable overlay.

## filesystem type names

- `overlay` (`FS_OVERLAY`), `erofs` (`FS_EROFS`), `tmpfs` (`FS_TMPFS`).

## overlay mount option keys

- `lowerdir` (`OPT_LOWERDIR`), `upperdir` (`OPT_UPPERDIR`),
  `workdir` (`OPT_WORKDIR`), `source` (`OPT_SOURCE`).

## cgroup v2 tokens

the nexus subtree and the interface files it reads and writes.

- `nexus` (`CG_SUBTREE`) - the nexus subtree under the cgroup v2 root; leaves
  live beneath it, the subtree itself holds no processes.
- controller names: `memory` (`CG_CTRL_MEMORY`), `pids` (`CG_CTRL_PIDS`),
  `cpu` (`CG_CTRL_CPU`).
- `cgroup.controllers` (`CG_CONTROLLERS`) - the controllers available here
  (read).
- `cgroup.subtree_control` (`CG_SUBTREE_CONTROL`) - enable controllers down the
  subtree.
- `cgroup.procs` (`CG_PROCS`) - move a process into a cgroup.
- `memory.max` (`CG_MEMORY_MAX`) - memory ceiling, bytes.
- `pids.max` (`CG_PIDS_MAX`) - task ceiling.
- `cpu.weight` (`CG_CPU_WEIGHT`) - relative cpu share, 1..=10000.
- `cpu.max` (`CG_CPU_MAX`) - absolute cpu quota per period.
