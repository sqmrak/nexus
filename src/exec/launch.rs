use crate::api::{Error, Layer, LaunchStrategy, Result};
use crate::exec::enter;
use std::convert::Infallible;
use std::os::unix::process::CommandExt;
use std::process::Command;

// default strategy: setns then execve, replacing the caller. a zygote
// strategy can implement LaunchStrategy for lower launch latency
pub struct SetnsExec;

impl LaunchStrategy for SetnsExec {
    fn launch(
        &self,
        layer: &Layer,
        argv: &[String],
        env: &[(String, String)],
    ) -> Result<Infallible> {
        enter(layer)?;
        crate::sys::sandbox::apply(&layer.desc.sandbox)?;

        let (prog, args) = argv.split_first().ok_or_else(|| Error::Init("empty argv".into()))?;

        // clear first: without env_clear the child inherits pid 1's whole
        // environment, leaking caller state past the sandbox boundary
        // exec replaces this process and only returns on failure
        let err = Command::new(prog).args(args).env_clear().envs(env.iter().cloned()).exec();
        Err(Error::Init(format!("exec {prog}: {err}")))
    }
}
