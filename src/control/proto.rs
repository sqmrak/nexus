// management-only text protocol; the hot path bypasses this socket entirely
// (nx opens the ns file and setns), so no fds, just human-readable lines

use crate::api::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    // build and pin a layer's namespace now (warm it)
    Build(String),
    // drop a layer's namespace, unmounting its pin
    Evict(String),
    // list built layers
    List,
    // daemon liveness and built count
    Status,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Ok,
    Lines(Vec<String>),
    Err(String),
}

impl Request {
    pub fn parse(line: &str) -> Result<Request> {
        let mut it = line.split_whitespace();
        let verb = it.next().unwrap_or("");
        let arg = it.next();
        let need = |a: Option<&str>| {
            a.map(|s| s.to_string()).ok_or_else(|| Error::Config(format!("{verb}: missing layer")))
        };
        match verb {
            "build" => Ok(Request::Build(need(arg)?)),
            "evict" => Ok(Request::Evict(need(arg)?)),
            "list" => Ok(Request::List),
            "status" => Ok(Request::Status),
            other => Err(Error::Config(format!("unknown verb: {other}"))),
        }
    }

    pub fn encode(&self) -> String {
        match self {
            Request::Build(l) => format!("build {l}"),
            Request::Evict(l) => format!("evict {l}"),
            Request::List => "list".into(),
            Request::Status => "status".into(),
        }
    }
}

impl Reply {
    // wire form: body lines then a blank terminator line
    pub fn encode(&self) -> String {
        let body = match self {
            Reply::Ok => "ok".to_string(),
            Reply::Lines(ls) => ls.join("\n"),
            Reply::Err(m) => format!("err {m}"),
        };
        format!("{body}\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        for r in [
            Request::Build("void".into()),
            Request::Evict("void".into()),
            Request::List,
            Request::Status,
        ] {
            assert_eq!(Request::parse(&r.encode()).unwrap(), r);
        }
    }

    #[test]
    fn missing_arg_errors() {
        assert!(Request::parse("build").is_err());
        assert!(Request::parse("bogus x").is_err());
    }

    #[test]
    fn reply_terminates_blank() {
        assert!(Reply::Ok.encode().ends_with("\n\n"));
    }
}
