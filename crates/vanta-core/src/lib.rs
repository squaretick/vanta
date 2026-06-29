//! `vanta-core` — the shared vocabulary every Vanta crate speaks.
//!
//! This crate defines the nouns of the system (see `docs/02-architecture.md`):
//! [`Request`], [`Resolution`], [`Artifact`], [`StoreKey`], [`Generation`],
//! [`Platform`], [`Scope`]; the trait seams that the engine is wired through
//! ([`Provider`], [`Backend`], [`CacheStore`], [`SignatureVerifier`],
//! [`LinkStrategy`]); and the [`VtaError`] taxonomy with stable `VTA-<AREA>-<NNNN>`
//! codes and process [`ExitCode`]s (see `docs/25-error-and-exit-code-catalog.md`).
//!
//! It is the leaf of the crate dependency graph and has no third-party
//! dependencies, so every other crate can depend on it freely.
#![forbid(unsafe_code)]

pub mod error;
pub mod platform;
pub mod traits;
pub mod types;
pub mod version;

pub use error::{Area, ExitCode, VtaError, VtaResult};
pub use platform::{Arch, Libc, Os, Platform};
pub use traits::{Backend, CacheStore, LinkStrategy, Provider, SignatureVerifier};
pub use types::{
    Artifact, Checksum, GenId, Generation, Reason, Resolution, Scope, StoreKey, ToolName,
};
pub use version::{Request, VersionReq};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_format() {
        let e = VtaError::new(Area::Cfg, 7, "bad value");
        assert_eq!(e.code(), "VTA-CFG-0007");
        assert_eq!(e.exit(), ExitCode::Config);
        assert_eq!(format!("{e}"), "error[VTA-CFG-0007]: bad value");
    }

    #[test]
    fn platform_token_roundtrip() {
        let p = Platform::parse("linux/x86_64/gnu").unwrap();
        assert_eq!(p.token(), "linux/x86_64/gnu");
        let m = Platform::parse("macos/aarch64").unwrap();
        assert_eq!(m.token(), "macos/aarch64");
        assert!(Platform::parse("plan9/sparc").is_err());
    }

    #[test]
    fn request_parsing() {
        let r = Request::parse("node@24").unwrap();
        assert_eq!(r.tool, "node");
        assert_eq!(r.version, VersionReq::Prefix("24".into()));
        assert_eq!(Request::parse("rust").unwrap().version, VersionReq::Latest);
        assert_eq!(
            Request::parse("python@3.13.4").unwrap().version,
            VersionReq::Exact("3.13.4".into())
        );
        assert_eq!(
            Request::parse("go@latest").unwrap().version,
            VersionReq::Latest
        );
        assert_eq!(
            Request::parse("node@^24").unwrap().version,
            VersionReq::Range("^24".into())
        );
    }

    #[test]
    fn store_key_validation() {
        assert!(StoreKey::new("blake3-aa3f").is_ok());
        assert!(StoreKey::new("sha1-deadbeef").is_err());
    }

    #[test]
    fn gen_id_display() {
        assert_eq!(GenId(8).to_string(), "0008");
        assert_eq!(GenId(1234).to_string(), "1234");
    }
}
