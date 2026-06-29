//! `vanta-env` — compose environment views and generate shell activation hooks.
//!
//! An environment is a `envs/<env-id>/bin` directory of links into the store; the
//! `env-id` is a hash of the resolved tool set, so identical sets share a view.
//! Activation hooks (per shell) put that bin directory on `PATH` and switch it
//! per directory. See `docs/10-environments.md`.
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use vanta_core::{StoreKey, VtaResult};
use vanta_store::{hash_bytes, link_best, Store};

/// One tool's contribution to an environment: its store key and the executables
/// to expose (paths relative to the store entry).
#[derive(Debug, Clone, PartialEq)]
pub struct EnvTool {
    pub tool: String,
    pub key: StoreKey,
    pub bins: Vec<String>,
}

/// A stable identifier for a resolved tool set (the hash of its sorted members).
pub fn env_id(tools: &[EnvTool]) -> String {
    let mut members: Vec<String> = tools
        .iter()
        .map(|t| format!("{}={}", t.tool, t.key.as_str()))
        .collect();
    members.sort();
    let blob = members.join("\n");
    // `hash_bytes` returns `blake3-<hex>`; use the hex as a compact directory id.
    hash_bytes(blob.as_bytes())
        .strip_prefix("blake3-")
        .unwrap_or("env")
        .chars()
        .take(32)
        .collect()
}

/// Compose (or refresh) the environment view for `tools`, returning its `bin`
/// directory. Executables are linked from the store with the cheapest available
/// strategy (`docs/09-store.md`).
pub fn compose(store: &Store, home: &Path, tools: &[EnvTool]) -> VtaResult<PathBuf> {
    let id = env_id(tools);
    let bin_dir = home.join("envs").join(&id).join("bin");
    std::fs::create_dir_all(&bin_dir).map_err(|e| {
        vanta_core::VtaError::new(
            vanta_core::Area::Env,
            1,
            format!("creating env dir {}: {e}", bin_dir.display()),
        )
    })?;
    for tool in tools {
        let entry = store.entry_path(&tool.key);
        for bin in &tool.bins {
            let src = entry.join(bin);
            let dst = bin_dir.join(basename(bin));
            link_best(&src, &dst)?;
        }
    }
    Ok(bin_dir)
}

/// The activation hook for a shell, for `eval "$(vanta activate <shell>)"`.
/// Returns `None` for an unsupported shell.
pub fn activate_hook(shell: &str) -> Option<String> {
    let hook = match shell {
        "bash" => BASH,
        "zsh" => ZSH,
        "fish" => FISH,
        "pwsh" | "powershell" => PWSH,
        _ => return None,
    };
    Some(hook.to_string())
}

fn basename(p: &str) -> String {
    p.rsplit(['/', '\\']).next().unwrap_or(p).to_string()
}

// Activation hooks place `~/.vanta/bin` on PATH idempotently (installed tools are
// linked there). Per-directory switching is described in `docs/10-environments.md`.

const BASH: &str = r#"# vanta shell hook (bash)
case ":$PATH:" in *":$HOME/.vanta/bin:"*) ;; *) export PATH="$HOME/.vanta/bin:$PATH";; esac
"#;

const ZSH: &str = r#"# vanta shell hook (zsh)
case ":$PATH:" in *":$HOME/.vanta/bin:"*) ;; *) export PATH="$HOME/.vanta/bin:$PATH";; esac
"#;

const FISH: &str = r#"# vanta shell hook (fish)
if not contains "$HOME/.vanta/bin" $PATH
    set -gx PATH "$HOME/.vanta/bin" $PATH
end
"#;

const PWSH: &str = r#"# vanta shell hook (PowerShell)
if ($env:PATH -notlike "*$HOME\.vanta\bin*") { $env:PATH = "$HOME\.vanta\bin;$env:PATH" }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_id_is_order_independent_and_stable() {
        let a = EnvTool {
            tool: "node".into(),
            key: StoreKey::new("blake3-aa").unwrap(),
            bins: vec![],
        };
        let b = EnvTool {
            tool: "go".into(),
            key: StoreKey::new("blake3-bb").unwrap(),
            bins: vec![],
        };
        let id1 = env_id(&[a.clone(), b.clone()]);
        let id2 = env_id(&[b, a]);
        assert_eq!(id1, id2);
        assert!(!id1.is_empty());
    }

    #[test]
    fn activate_known_and_unknown() {
        assert!(activate_hook("zsh").unwrap().contains("vanta"));
        assert!(activate_hook("bash").is_some());
        assert!(activate_hook("tcsh").is_none());
    }

    #[test]
    fn compose_links_bins() {
        let home = std::env::temp_dir().join(format!("vanta-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let store = Store::open(&home).unwrap();
        let staged = store.new_staging().unwrap();
        std::fs::create_dir_all(staged.join("bin")).unwrap();
        std::fs::write(staged.join("bin/node"), b"#!/bin/true").unwrap();
        let key = store.publish_tree(&staged).unwrap();

        let tools = vec![EnvTool {
            tool: "node".into(),
            key,
            bins: vec!["bin/node".into()],
        }];
        let bin_dir = compose(&store, &home, &tools).unwrap();
        assert!(bin_dir.join("node").exists());
        let _ = std::fs::remove_dir_all(&home);
    }
}
