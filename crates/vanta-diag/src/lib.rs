//! `vanta-diag` — `vanta doctor` health checks and diagnostics.
//!
//! Runs a set of checks over a `$VANTA_HOME` and reports each with a fix where it
//! fails. See `docs/18-developer-experience.md` and `docs/25-error-and-exit-code-catalog.md`.
#![forbid(unsafe_code)]

use std::path::Path;
use vanta_state::State;
use vanta_store::Store;

/// One diagnostic result.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

fn check(name: &str, ok: bool, detail: impl Into<String>) -> Check {
    Check {
        name: name.to_string(),
        ok,
        detail: detail.into(),
    }
}

/// Run all checks against `home`.
pub fn run(home: &Path) -> Vec<Check> {
    let mut out = Vec::new();

    // Ensure the home directory exists so later checks (state db) can initialize.
    let created = std::fs::create_dir_all(home).is_ok();
    out.push(check("home directory", created, home.display().to_string()));

    // ~/.vanta/bin on PATH.
    let bin = home.join("bin");
    let bin_str = bin.to_string_lossy().into_owned();
    let sep = if cfg!(windows) { ';' } else { ':' };
    let on_path = std::env::var("PATH")
        .map(|p| p.split(sep).any(|seg| seg == bin_str))
        .unwrap_or(false);
    out.push(check(
        "~/.vanta/bin on PATH",
        on_path,
        if on_path {
            "present".to_string()
        } else {
            "add `eval \"$(vanta activate <shell>)\"` to your shell rc [VTA-ENV-0001]".to_string()
        },
    ));

    // State database.
    match State::open(&home.join("state.db")) {
        Ok(state) => match state.schema_version() {
            Ok(v) => out.push(check("state database", true, format!("schema v{v}"))),
            Err(e) => out.push(check("state database", false, e.to_string())),
        },
        Err(e) => out.push(check("state database", false, e.to_string())),
    }

    // Store integrity.
    match Store::open(home) {
        Ok(store) => {
            let (total, corrupt) = integrity(&store, home);
            out.push(check(
                "store integrity",
                corrupt == 0,
                format!("{total} entries, {corrupt} corrupt"),
            ));
        }
        Err(e) => out.push(check("store integrity", false, e.to_string())),
    }

    out
}

fn integrity(store: &Store, home: &Path) -> (usize, usize) {
    let dir = home.join("store");
    let (mut total, mut corrupt) = (0usize, 0usize);
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("blake3-") {
                if let Ok(key) = vanta_core::StoreKey::new(name) {
                    total += 1;
                    if !store.verify_entry(&key).unwrap_or(false) {
                        corrupt += 1;
                    }
                }
            }
        }
    }
    (total, corrupt)
}

/// Whether every check passed.
pub fn all_ok(checks: &[Check]) -> bool {
    checks.iter().all(|c| c.ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_on_fresh_home() {
        let home = std::env::temp_dir().join(format!("vanta-diag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let checks = run(&home);
        // home, PATH, state database, store integrity.
        assert_eq!(checks.len(), 4);
        // A fresh home has no store entries → integrity is clean (0 corrupt).
        let integ = checks.iter().find(|c| c.name == "store integrity").unwrap();
        assert!(integ.ok);
        let _ = std::fs::remove_dir_all(&home);
    }
}
