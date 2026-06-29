//! Version requests and tool requests (the `name@version` surface).
//!
//! See `docs/06-resolution.md` for the request grammar. Parsing lives here;
//! version ordering and constraint satisfaction live in `vanta-resolve`.

use crate::error::{Area, VtaError, VtaResult};
use crate::types::ToolName;
use std::fmt;

/// A version request as written in `vanta.toml` or on the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum VersionReq {
    /// A fully-specified version, e.g. `24.3.0`.
    Exact(String),
    /// A prefix match, e.g. `24` or `24.3`.
    Prefix(String),
    /// The newest stable version.
    Latest,
    /// The newest long-term-support version (provider-defined).
    Lts,
    /// A named channel, e.g. `stable` or `nightly`.
    Channel(String),
    /// A SemVer range, e.g. `^24` or `>=20 <24`.
    Range(String),
    /// Use a system-provided tool.
    System,
}

impl VersionReq {
    /// Parse the version portion of a request. This never fails; an unrecognized
    /// token is treated as a channel name (resolution decides if it exists).
    pub fn parse(s: &str) -> VersionReq {
        match s {
            "latest" | "" => VersionReq::Latest,
            "lts" => VersionReq::Lts,
            "system" => VersionReq::System,
            _ if s.starts_with(['^', '~', '>', '<', '=', '*']) => VersionReq::Range(s.to_string()),
            _ if s.chars().next().is_some_and(|c| c.is_ascii_digit()) => {
                if s.matches('.').count() >= 2 {
                    VersionReq::Exact(s.to_string())
                } else {
                    VersionReq::Prefix(s.to_string())
                }
            }
            _ => VersionReq::Channel(s.to_string()),
        }
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionReq::Exact(s)
            | VersionReq::Prefix(s)
            | VersionReq::Channel(s)
            | VersionReq::Range(s) => f.write_str(s),
            VersionReq::Latest => f.write_str("latest"),
            VersionReq::Lts => f.write_str("lts"),
            VersionReq::System => f.write_str("system"),
        }
    }
}

/// A tool request: a tool name plus a version request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub tool: ToolName,
    pub version: VersionReq,
}

impl Request {
    /// Parse a `name[@version]` request. A missing version means [`VersionReq::Latest`].
    pub fn parse(s: &str) -> VtaResult<Request> {
        let (tool, version) = match s.split_once('@') {
            Some((t, v)) => (t, VersionReq::parse(v)),
            None => (s, VersionReq::Latest),
        };
        if tool.is_empty() {
            return Err(VtaError::new(
                Area::Cfg,
                4,
                format!("empty tool name in request `{s}`"),
            ));
        }
        Ok(Request {
            tool: tool.to_string(),
            version,
        })
    }
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.tool, self.version)
    }
}

#[cfg(test)]
mod fuzz {
    use super::*;
    proptest::proptest! {
        #[test]
        fn version_req_parse_never_panics(s in ".*") { let _ = VersionReq::parse(&s); }
        #[test]
        fn request_parse_never_panics(s in ".*") { let _ = Request::parse(&s); }
    }
}
