//! `vanta-platform` — OS/arch detection and path/executable/shell helpers
//! (`docs/17-cross-platform.md`). A thin, dependency-light utility layer the rest
//! of the workspace builds on.
#![forbid(unsafe_code)]

use std::path::PathBuf;
pub use vanta_core::Platform;

/// The platform the running binary targets.
pub fn detect() -> Platform {
    Platform::current()
}

/// The user's home directory (`HOME`, or `USERPROFILE` on Windows).
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// `$VANTA_HOME`, else `<home>/.vanta`.
pub fn vanta_home() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os("VANTA_HOME") {
        return Some(PathBuf::from(h));
    }
    home_dir().map(|h| h.join(".vanta"))
}

/// The PATH list separator (`;` on Windows, `:` elsewhere).
pub fn path_list_sep() -> char {
    if cfg!(windows) {
        ';'
    } else {
        ':'
    }
}

/// The executable file suffix (`.exe` on Windows, empty elsewhere).
pub fn exe_suffix() -> &'static str {
    if cfg!(windows) {
        ".exe"
    } else {
        ""
    }
}

/// Append the platform's executable suffix to `name` if not already present.
pub fn with_exe(name: &str) -> String {
    let suffix = exe_suffix();
    if suffix.is_empty() || name.ends_with(suffix) {
        name.to_string()
    } else {
        format!("{name}{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_helpers_are_consistent() {
        let n = with_exe("node");
        if cfg!(windows) {
            assert_eq!(n, "node.exe");
            assert_eq!(with_exe("node.exe"), "node.exe"); // idempotent
        } else {
            assert_eq!(n, "node");
        }
    }

    #[test]
    fn detect_returns_a_platform() {
        assert!(detect().token().contains('/'));
    }
}
