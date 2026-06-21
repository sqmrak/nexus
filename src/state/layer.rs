// private deserialization structs so the file format can evolve without
// breaking the public api; libc is read as data, not branched on by name

use crate::api::{
    Cap, CpuMax, Error, IdMap, LayerDescriptor, LayerFlags, LayerId, LayerType, Libc, Limits,
    Result, Sandbox, Seccomp,
};
use crate::vocab;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct File {
    layer: BTreeMap<String, Entry>,
}

#[derive(Deserialize)]
struct Entry {
    r#type: String,
    priority: u32,
    libc: LibcTable,
    sandbox: Option<SandboxTable>,
    resources: Option<ResourcesTable>,
    // every other key is a flag. only true ones are written, so capturing
    // bools is enough; a stray false is dropped on convert
    #[serde(flatten)]
    flags: BTreeMap<String, bool>,
}

#[derive(Deserialize)]
struct LibcTable {
    name: String,
    loader: Option<String>,
}

#[derive(Deserialize)]
struct SandboxTable {
    #[serde(default)]
    read: Vec<PathBuf>,
    #[serde(default)]
    write: Vec<PathBuf>,
    // "off" or "baseline"
    seccomp: Option<String>,
    idmap: Option<IdMapTable>,
    // capability names to keep, e.g. ["net_bind_service"]. default: drop all
    #[serde(default)]
    keep_caps: Vec<String>,
}

#[derive(Deserialize)]
struct IdMapTable {
    outer_start: u32,
    count: u32,
}

#[derive(Deserialize)]
struct ResourcesTable {
    // memory.max in bytes, pids.max as a count
    memory_max: Option<u64>,
    pids_max: Option<u64>,
    // cpu.weight, 1..=10000
    cpu_weight: Option<u32>,
    cpu_max: Option<CpuMaxTable>,
}

#[derive(Deserialize)]
struct CpuMaxTable {
    quota_us: u64,
    period_us: u64,
}

pub fn load_layers(path: &Path) -> Result<Vec<LayerDescriptor>> {
    let text =
        std::fs::read_to_string(path).map_err(|e| Error::Config(format!("read {path:?}: {e}")))?;
    parse_layers(&text)
}

fn parse_layers(text: &str) -> Result<Vec<LayerDescriptor>> {
    let file: File = toml::from_str(text).map_err(|e| Error::Config(format!("parse: {e}")))?;
    file.layer.into_iter().map(|(id, e)| e.into_descriptor(id)).collect()
}

impl Entry {
    fn into_descriptor(self, id: String) -> Result<LayerDescriptor> {
        let r#type = match self.r#type.as_str() {
            vocab::TYPE_NATIVE => LayerType::Native,
            vocab::TYPE_SHADOWED => LayerType::Shadowed,
            vocab::TYPE_RESCUE => LayerType::Rescue,
            other => return Err(Error::Config(format!("unknown layer type: {other}"))),
        };
        let flags = into_flags(self.flags);
        let sandbox = self.sandbox.map(SandboxTable::into_sandbox).transpose()?;
        let resources = self.resources.map(ResourcesTable::into_limits).transpose()?;
        let id = LayerId::new(id).map_err(Error::Config)?;
        Ok(LayerDescriptor {
            id,
            r#type,
            priority: self.priority,
            libc: Libc { name: self.libc.name, loader: self.libc.loader },
            flags,
            sandbox: sandbox.unwrap_or_default(),
            resources: resources.unwrap_or_default(),
        })
    }
}

// map the true-valued flag keys onto the typed set; unknown keys ride in
// extra so a fork's own flags survive a round trip
fn into_flags(raw: BTreeMap<String, bool>) -> LayerFlags {
    let mut f = LayerFlags::default();
    for (k, on) in raw {
        if !on {
            continue;
        }
        match k.as_str() {
            vocab::FLAG_META => f.meta = true,
            vocab::FLAG_NON_FHS => f.non_fhs = true,
            vocab::FLAG_ATOMIC => f.atomic = true,
            vocab::FLAG_EPHEMERAL => f.ephemeral = true,
            vocab::FLAG_PINNED => f.pinned = true,
            vocab::FLAG_HIDDEN => f.hidden = true,
            _ => f.extra.push(k),
        }
    }
    f
}

impl SandboxTable {
    fn into_sandbox(self) -> Result<Sandbox> {
        let seccomp = match self.seccomp.as_deref() {
            None | Some(vocab::SECCOMP_OFF) => Seccomp::Off,
            Some(vocab::SECCOMP_BASELINE) => Seccomp::Baseline,
            Some(other) => return Err(Error::Config(format!("unknown seccomp: {other}"))),
        };
        let idmap = self
            .idmap
            .map(|m| IdMap::new(m.outer_start, m.count))
            .transpose()
            .map_err(Error::Config)?;
        let keep_caps = self
            .keep_caps
            .iter()
            .map(|n| {
                Cap::from_name(n).ok_or_else(|| Error::Config(format!("unknown capability: {n}")))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Sandbox { read: self.read, write: self.write, seccomp, idmap, keep_caps })
    }
}

impl ResourcesTable {
    fn into_limits(self) -> Result<Limits> {
        let cpu_max = self
            .cpu_max
            .map(|c| CpuMax::new(c.quota_us, c.period_us))
            .transpose()
            .map_err(Error::Config)?;
        Ok(Limits {
            memory_max: self.memory_max,
            pids_max: self.pids_max,
            cpu_weight: self.cpu_weight,
            cpu_max,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{HealthCheck, LayerSelector};
    use crate::init::{DefaultHealthCheck, DefaultSelector};

    const SAMPLE: &str = r#"
        [layer.debian]
        type = "shadowed"
        priority = 1
        [layer.debian.libc]
        name = "glibc"
        loader = "/lib64/ld-linux-x86-64.so.2"

        [layer.void]
        type = "native"
        priority = 3
        pinned = true
        [layer.void.libc]
        name = "musl"
        loader = "/lib/ld-musl-x86_64.so.1"

        [layer.tools]
        type = "native"
        priority = 5
        [layer.tools.libc]
        name = "static"
    "#;

    #[test]
    fn parses_descriptors() {
        let mut layers = parse_layers(SAMPLE).unwrap();
        layers.sort_by_key(|l| l.priority);
        assert_eq!(layers.len(), 3);

        let debian = &layers[0];
        assert_eq!(debian.id.as_str(), "debian");
        assert_eq!(debian.r#type, LayerType::Shadowed);
        assert_eq!(debian.libc.name, "glibc");
        assert!(!debian.libc.is_static());

        let void = &layers[1];
        assert!(void.flags.pinned);

        // static layer has no loader
        assert!(layers[2].libc.is_static());
    }

    #[test]
    fn false_flags_dropped() {
        let t = r#"
            [layer.x]
            type = "native"
            priority = 2
            hidden = false
            ephemeral = true
            [layer.x.libc]
            name = "glibc"
            loader = "/lib/ld.so"
        "#;
        let layers = parse_layers(t).unwrap();
        assert!(layers[0].flags.ephemeral);
        assert!(!layers[0].flags.hidden);
    }

    #[test]
    fn unknown_flag_goes_to_extra() {
        let t = r#"
            [layer.x]
            type = "native"
            priority = 2
            my_fork_flag = true
            [layer.x.libc]
            name = "glibc"
            loader = "/lib/ld.so"
        "#;
        let f = &parse_layers(t).unwrap()[0].flags;
        assert_eq!(f.extra, ["my_fork_flag"]);
        assert!(!f.meta);
    }

    #[test]
    fn parses_sandbox() {
        let t = r#"
            [layer.x]
            type = "native"
            priority = 2
            [layer.x.libc]
            name = "glibc"
            loader = "/lib/ld.so"
            [layer.x.sandbox]
            read = ["/usr", "/etc"]
            write = ["/home"]
            seccomp = "baseline"
        "#;
        let l = &parse_layers(t).unwrap()[0];
        assert_eq!(l.sandbox.read.len(), 2);
        assert_eq!(l.sandbox.write, [std::path::PathBuf::from("/home")]);
        assert_eq!(l.sandbox.seccomp, crate::api::Seccomp::Baseline);
    }

    #[test]
    fn no_sandbox_is_empty() {
        let l = &parse_layers(SAMPLE).unwrap()[0];
        assert!(l.sandbox.is_empty());
    }

    #[test]
    fn parses_keep_caps() {
        let t = r#"
            [layer.x]
            type = "native"
            priority = 2
            [layer.x.libc]
            name = "glibc"
            loader = "/lib/ld.so"
            [layer.x.sandbox]
            keep_caps = ["net_bind_service", "sys_time"]
        "#;
        let l = &parse_layers(t).unwrap()[0];
        assert_eq!(l.sandbox.keep_caps.iter().map(|c| c.raw()).collect::<Vec<_>>(), [10, 25]);
        assert!(!l.sandbox.is_empty());
    }

    #[test]
    fn unknown_cap_is_rejected() {
        let t = r#"
            [layer.x]
            type = "native"
            priority = 2
            [layer.x.libc]
            name = "glibc"
            loader = "/lib/ld.so"
            [layer.x.sandbox]
            keep_caps = ["not_a_cap"]
        "#;
        assert!(parse_layers(t).is_err());
    }

    #[test]
    fn parses_idmap() {
        let t = r#"
            [layer.x]
            type = "native"
            priority = 2
            [layer.x.libc]
            name = "glibc"
            loader = "/lib/ld.so"
            [layer.x.sandbox.idmap]
            outer_start = 100000
            count = 65536
        "#;
        let m = parse_layers(t).unwrap()[0].sandbox.idmap.unwrap();
        assert_eq!(m.outer_start(), 100000);
        assert_eq!(m.count(), 65536);
    }

    // stub health check ;  never touches the real filesystem
    struct AllHealthy;
    impl HealthCheck for AllHealthy {
        fn is_healthy(&self, _l: &LayerDescriptor, _root: &Path) -> bool {
            true
        }
    }

    #[test]
    fn selects_lowest_priority() {
        let layers = parse_layers(SAMPLE).unwrap();
        let picked = DefaultSelector.select(&layers, &AllHealthy, Path::new("/store")).unwrap();
        assert_eq!(picked.as_str(), "debian");
    }

    #[test]
    fn skips_unhealthy() {
        // mark debian unhealthy by pointing its loader nowhere; void and
        // tools remain, void wins on priority
        let layers = parse_layers(SAMPLE).unwrap();
        struct OnlyStatic;
        impl HealthCheck for OnlyStatic {
            fn is_healthy(&self, l: &LayerDescriptor, _root: &Path) -> bool {
                // simulate: only musl and static layers pass this round
                l.libc.name != "glibc"
            }
        }
        let picked = DefaultSelector.select(&layers, &OnlyStatic, Path::new("/store")).unwrap();
        assert_eq!(picked.as_str(), "void");
    }

    #[test]
    fn default_health_static_is_healthy() {
        let layers = parse_layers(SAMPLE).unwrap();
        let tools = layers.iter().find(|l| l.id.as_str() == "tools").unwrap();
        // static layer has no loader, so the root is irrelevant
        assert!(DefaultHealthCheck.is_healthy(tools, Path::new("/nonexistent")));
    }

    #[test]
    fn parses_resources() {
        let t = r#"
            [layer.svc]
            type = "native"
            priority = 2
            [layer.svc.libc]
            name = "glibc"
            loader = "/lib/ld.so"
            [layer.svc.resources]
            memory_max = 104857600
            pids_max = 128
            cpu_weight = 200
            [layer.svc.resources.cpu_max]
            quota_us = 50000
            period_us = 100000
        "#;
        let layers = parse_layers(t).unwrap();
        let r = &layers[0].resources;
        assert_eq!(r.memory_max, Some(104_857_600));
        assert_eq!(r.pids_max, Some(128));
        assert_eq!(r.cpu_weight, Some(200));
        assert_eq!(r.cpu_max, Some(crate::api::CpuMax::new(50_000, 100_000).unwrap()));
    }

    #[test]
    fn no_resources_is_empty() {
        let layers = parse_layers(SAMPLE).unwrap();
        assert!(layers.iter().all(|l| l.resources.is_empty()));
    }

    #[test]
    fn invalid_cpu_max_is_rejected() {
        let t = r#"
            [layer.svc]
            type = "native"
            priority = 2
            [layer.svc.libc]
            name = "static"
            [layer.svc.resources.cpu_max]
            quota_us = 0
            period_us = 100000
        "#;
        assert!(parse_layers(t).is_err());
    }
}
