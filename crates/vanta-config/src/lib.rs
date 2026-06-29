//! `vanta-config` — load, validate, and merge Vanta manifests.
//!
//! Parses `vanta.toml`/`config.toml` into the typed [`model::Manifest`], reports
//! span-accurate `VTA-CFG-*` diagnostics on failure, and implements the
//! precedence merge (global < project). See `docs/05-configuration.md`.
#![forbid(unsafe_code)]

pub mod model;

pub use model::Manifest;

use std::fs;
use std::path::Path;
use vanta_core::{Area, VtaError, VtaResult};

/// Parse a manifest from TOML source. `origin` is used in diagnostics (a path or
/// `<string>`); errors carry the line and column of the offending token.
pub fn parse_str(src: &str, origin: &str) -> VtaResult<Manifest> {
    toml::from_str::<Manifest>(src).map_err(|e| cfg_error(src, origin, &e))
}

/// Load and parse a manifest file.
pub fn load_file(path: &Path) -> VtaResult<Manifest> {
    let src = fs::read_to_string(path).map_err(|e| {
        VtaError::new(
            Area::Cfg,
            1,
            format!("cannot read manifest {}: {e}", path.display()),
        )
    })?;
    parse_str(&src, &path.display().to_string())
}

/// Merge a project manifest over a global one. The project
/// layer wins on every per-key conflict.
pub fn merge(global: &Manifest, project: &Manifest) -> Manifest {
    let mut out = global.clone();
    out.tools.extend(project.tools.clone());
    out.env.extend(project.env.clone());
    out.tasks.extend(project.tasks.clone());
    out.registries.extend(project.registries.clone());
    out.settings = merge_settings(&global.settings, &project.settings);
    if project.workspace.is_some() {
        out.workspace = project.workspace.clone();
    }
    if project.version.is_some() {
        out.version = project.version;
    }
    out
}

fn merge_settings(g: &model::Settings, p: &model::Settings) -> model::Settings {
    macro_rules! pick {
        ($field:ident) => {
            p.$field.clone().or_else(|| g.$field.clone())
        };
    }
    model::Settings {
        auto_install: pick!(auto_install),
        verify: pick!(verify),
        jobs: pick!(jobs),
        link_strategy: pick!(link_strategy),
        shims: pick!(shims),
        offline: pick!(offline),
        mirror: pick!(mirror),
        targets: pick!(targets),
        retain_generations: pick!(retain_generations),
        gc_keep_days: pick!(gc_keep_days),
        color: pick!(color),
        telemetry: pick!(telemetry),
        read_foreign_versions: pick!(read_foreign_versions),
    }
}

fn cfg_error(src: &str, origin: &str, e: &toml::de::Error) -> VtaError {
    let (line, col) = e.span().map(|s| line_col(src, s.start)).unwrap_or((0, 0));
    VtaError::new(Area::Cfg, 1, format!("{e} ({origin}:{line}:{col})"))
}

/// Convert a byte offset into a 1-based line and column.
fn line_col(src: &str, byte: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut col = 1usize;
    for (i, ch) in src.char_indices() {
        if i >= byte {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::model::ToolSpec;
    use super::*;

    #[test]
    fn parses_minimal() {
        let m = parse_str("[tools]\nnode = \"24\"\npython = \"3.13\"\n", "<test>").unwrap();
        assert_eq!(m.tools["node"].version(), "24");
        assert_eq!(m.tools["python"].version(), "3.13");
    }

    #[test]
    fn parses_detailed_tool() {
        let m = parse_str(
            "[tools]\nripgrep = { version = \"14\", os = [\"macos\", \"linux\"] }\n",
            "<test>",
        )
        .unwrap();
        match &m.tools["ripgrep"] {
            ToolSpec::Detailed(d) => {
                assert_eq!(d.version, "14");
                assert_eq!(
                    d.os.as_deref(),
                    Some(&["macos".to_string(), "linux".to_string()][..])
                );
            }
            _ => panic!("expected detailed"),
        }
    }

    #[test]
    fn unknown_top_level_key_is_error() {
        let err = parse_str("[nonsense]\nx = 1\n", "<test>").unwrap_err();
        assert_eq!(err.area, Area::Cfg);
    }

    #[test]
    fn project_overrides_global() {
        let g = parse_str("[tools]\nnode = \"24\"\nripgrep = \"14\"\n", "<g>").unwrap();
        let p = parse_str("[tools]\nnode = \"20\"\n", "<p>").unwrap();
        let merged = merge(&g, &p);
        assert_eq!(merged.tools["node"].version(), "20"); // project wins
        assert_eq!(merged.tools["ripgrep"].version(), "14"); // global retained
    }

    #[test]
    fn settings_merge_is_field_wise() {
        let g = parse_str("[settings]\njobs = 8\nverify = \"require\"\n", "<g>").unwrap();
        let p = parse_str("[settings]\njobs = 12\n", "<p>").unwrap();
        let merged = merge(&g, &p);
        assert_eq!(merged.settings.jobs, Some(12)); // project overrides
        assert_eq!(merged.settings.verify.as_deref(), Some("require")); // global retained
    }
}

#[cfg(test)]
mod fuzz {
    use super::*;
    proptest::proptest! {
        #[test]
        fn manifest_parse_never_panics(s in ".*") { let _ = parse_str(&s, "<fuzz>"); }
    }
}
