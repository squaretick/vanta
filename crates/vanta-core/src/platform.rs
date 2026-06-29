//! Platform identifiers — the canonical `os/arch[/libc]` tokens.
//!
//! See `docs/17-cross-platform.md` and `docs/26-registry-and-metadata-reference.md`.

use crate::error::{Area, VtaError, VtaResult};
use std::fmt;

/// Operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Os {
    Linux,
    Macos,
    Windows,
}

/// CPU architecture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Arch {
    X86_64,
    Aarch64,
}

/// C library / ABI variant (only meaningful on Linux).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Libc {
    Gnu,
    Musl,
    /// Not applicable (macOS / Windows).
    None,
}

impl Os {
    pub fn as_str(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Macos => "macos",
            Os::Windows => "windows",
        }
    }
}

impl Arch {
    pub fn as_str(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "aarch64",
        }
    }
}

impl Libc {
    pub fn as_str(self) -> &'static str {
        match self {
            Libc::Gnu => "gnu",
            Libc::Musl => "musl",
            Libc::None => "",
        }
    }
}

/// A fully-qualified target platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Platform {
    pub os: Os,
    pub arch: Arch,
    pub libc: Libc,
}

impl Platform {
    /// The canonical token, e.g. `"linux/x86_64/gnu"` or `"macos/aarch64"`.
    pub fn token(&self) -> String {
        let base = format!("{}/{}", self.os.as_str(), self.arch.as_str());
        match self.libc {
            Libc::None => base,
            other => format!("{base}/{}", other.as_str()),
        }
    }

    /// Parse a canonical token. Linux requires a libc segment; others must omit it.
    pub fn parse(s: &str) -> VtaResult<Platform> {
        let mut parts = s.split('/');
        let os = match parts.next() {
            Some("linux") => Os::Linux,
            Some("macos") => Os::Macos,
            Some("windows") => Os::Windows,
            _ => return Err(bad(s)),
        };
        let arch = match parts.next() {
            Some("x86_64") => Arch::X86_64,
            Some("aarch64") => Arch::Aarch64,
            _ => return Err(bad(s)),
        };
        let libc = match (os, parts.next()) {
            (Os::Linux, Some("gnu")) => Libc::Gnu,
            (Os::Linux, Some("musl")) => Libc::Musl,
            (Os::Linux, _) => return Err(bad(s)),
            (_, None) => Libc::None,
            (_, Some(_)) => return Err(bad(s)),
        };
        if parts.next().is_some() {
            return Err(bad(s));
        }
        Ok(Platform { os, arch, libc })
    }

    /// The platform the running binary was built for.
    ///
    /// On Linux this reports the `gnu` libc; musl targets are distinguished at
    /// artifact-selection time (see `docs/17-cross-platform.md`).
    pub fn current() -> Platform {
        let os = if cfg!(target_os = "macos") {
            Os::Macos
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        };
        let arch = if cfg!(target_arch = "aarch64") {
            Arch::Aarch64
        } else {
            Arch::X86_64
        };
        let libc = match os {
            Os::Linux => Libc::Gnu,
            _ => Libc::None,
        };
        Platform { os, arch, libc }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.token())
    }
}

fn bad(s: &str) -> VtaError {
    VtaError::new(Area::Sys, 1, format!("unknown platform token `{s}`"))
}
