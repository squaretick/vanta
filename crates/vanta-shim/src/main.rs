//! `vanta-shim` — the per-tool dispatcher (see `docs/10-environments.md`).
//!
//! Installed under each tool's name in `~/.vanta/bin`. When invoked it finds the
//! tool that its `argv[0]` names in the active generation, locates the real
//! binary in the store, and `exec`s it. (Per-directory switching via a
//! resolution cache is a later refinement; this cut dispatches from the active
//! generation.)
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use vanta_core::StoreKey;
use vanta_state::State;

fn main() -> ExitCode {
    let invoked = std::env::args()
        .next()
        .and_then(|p| {
            Path::new(&p)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    let name = invoked.strip_suffix(".exe").unwrap_or(&invoked).to_string();
    let args: Vec<String> = std::env::args().skip(1).collect();

    match dispatch(&name, &args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("vanta-shim: {msg}");
            ExitCode::from(1)
        }
    }
}

fn dispatch(name: &str, args: &[String]) -> Result<ExitCode, String> {
    let home = home().ok_or("cannot determine VANTA_HOME")?;
    let state = State::open(&home.join("state.db")).map_err(|e| e.to_string())?;
    let id = state
        .current()
        .map_err(|e| e.to_string())?
        .ok_or("no active generation")?;
    let generation = state
        .get_generation(id)
        .map_err(|e| e.to_string())?
        .ok_or("active generation is missing")?;
    let (_, key) = generation
        .tools
        .iter()
        .find(|(tool, _)| tool == name)
        .ok_or_else(|| format!("`{name}` is not managed by vanta"))?;
    // L12/M7: validate the key shape before joining it onto the store path so a
    // malformed generation record cannot traverse out of the store.
    let key = StoreKey::new(key.clone()).map_err(|e| e.to_string())?;
    let entry = home.join("store").join(key.as_str());
    let bin = find_bin(&entry, name)
        .ok_or_else(|| format!("executable for `{name}` not found in {}", entry.display()))?;
    exec(&bin, args)
}

fn find_bin(entry: &Path, name: &str) -> Option<PathBuf> {
    let candidates = [
        entry.join("bin").join(name),
        entry.join(name),
        entry.join("bin").join(format!("{name}.exe")),
        entry.join(format!("{name}.exe")),
    ];
    candidates.into_iter().find(|c| c.is_file())
}

#[cfg(unix)]
fn exec(bin: &Path, args: &[String]) -> Result<ExitCode, String> {
    use std::os::unix::process::CommandExt;
    // `exec` replaces this process and only returns on failure.
    let err = Command::new(bin).args(args).exec();
    Err(format!("exec {}: {err}", bin.display()))
}

#[cfg(not(unix))]
fn exec(bin: &Path, args: &[String]) -> Result<ExitCode, String> {
    let status = Command::new(bin)
        .args(args)
        .status()
        .map_err(|e| format!("running {}: {e}", bin.display()))?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn home() -> Option<PathBuf> {
    if let Ok(h) = std::env::var("VANTA_HOME") {
        return Some(PathBuf::from(h));
    }
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(|base| PathBuf::from(base).join(".vanta"))
}
