//! The error taxonomy: stable `VTA-<AREA>-<NNNN>` codes and process exit codes.
//!
//! See `docs/25-error-and-exit-code-catalog.md`. Every library function returns
//! [`VtaResult`]; every error carries an [`Area`], a number, and a message.

use std::error::Error;
use std::fmt;

/// Subsystem area of an error code. The string form (`CFG`, `RES`, …) is stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Area {
    /// Config / manifest (`vanta-config`).
    Cfg,
    /// Resolution / versioning (`vanta-resolve`).
    Res,
    /// Registry (`vanta-registry`).
    Reg,
    /// Provider / sandbox (`vanta-provider`).
    Prov,
    /// Network / download (`vanta-net`).
    Net,
    /// Verification / security (`vanta-security`).
    Vrf,
    /// Store / state / IO (`vanta-store`, `vanta-state`).
    Store,
    /// Install engine (`vanta-install`).
    Inst,
    /// Environment / activation (`vanta-env`).
    Env,
    /// Lockfile (`vanta-lock`).
    Lock,
    /// Platform (`vanta-platform`).
    Sys,
    /// Internal (a bug).
    Int,
}

impl Area {
    /// The stable string token used in a code (e.g. `"CFG"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Area::Cfg => "CFG",
            Area::Res => "RES",
            Area::Reg => "REG",
            Area::Prov => "PROV",
            Area::Net => "NET",
            Area::Vrf => "VRF",
            Area::Store => "STORE",
            Area::Inst => "INST",
            Area::Env => "ENV",
            Area::Lock => "LOCK",
            Area::Sys => "SYS",
            Area::Int => "INT",
        }
    }

    /// The default process exit code for this area (see `docs/25`).
    pub fn exit(self) -> ExitCode {
        match self {
            Area::Cfg => ExitCode::Config,
            Area::Res => ExitCode::Resolve,
            Area::Reg | Area::Net => ExitCode::Network,
            Area::Vrf => ExitCode::Verify,
            Area::Store | Area::Inst | Area::Lock => ExitCode::Store,
            Area::Prov | Area::Env | Area::Sys | Area::Int => ExitCode::Failure,
        }
    }
}

impl fmt::Display for Area {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stable process exit codes (`docs/25-error-and-exit-code-catalog.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExitCode {
    Ok = 0,
    Failure = 1,
    Usage = 2,
    Config = 3,
    Resolve = 4,
    Network = 5,
    Verify = 6,
    Store = 7,
    NotFound = 8,
    Trust = 9,
}

impl ExitCode {
    /// The numeric code, for `std::process::exit` / `process::ExitCode`.
    pub fn as_i32(self) -> i32 {
        self as i32
    }
}

/// The structured error type returned across the workspace.
#[derive(Debug)]
pub struct VtaError {
    /// The subsystem area.
    pub area: Area,
    /// The stable per-area number (rendered zero-padded to four digits).
    pub number: u16,
    /// A human-readable, actionable message.
    pub message: String,
    /// An optional underlying cause.
    pub source: Option<Box<dyn Error + Send + Sync>>,
}

impl VtaError {
    /// Construct an error with an area, a stable number, and a message.
    pub fn new(area: Area, number: u16, message: impl Into<String>) -> Self {
        VtaError {
            area,
            number,
            message: message.into(),
            source: None,
        }
    }

    /// Attach an underlying cause.
    pub fn with_source(mut self, source: impl Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// The stable code string, e.g. `"VTA-CFG-0007"`.
    pub fn code(&self) -> String {
        format!("VTA-{}-{:04}", self.area.as_str(), self.number)
    }

    /// The process exit code this error maps to.
    pub fn exit(&self) -> ExitCode {
        self.area.exit()
    }
}

impl fmt::Display for VtaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error[{}]: {}", self.code(), self.message)
    }
}

impl Error for VtaError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|s| s.as_ref() as &(dyn Error + 'static))
    }
}

/// The workspace result alias.
pub type VtaResult<T> = Result<T, VtaError>;
