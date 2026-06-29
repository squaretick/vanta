//! Link strategies for composing environment views (`docs/09-store.md`).
//!
//! Order of preference: hardlink → symlink → copy. (Reflink/CoW is a
//! platform-specific syscall added later; `docs/17-cross-platform.md`.) On
//! Windows, symlinks may require privilege, so copy is the reliable fallback.

use std::fs;
use std::path::Path;
use vanta_core::{Area, VtaError, VtaResult};

/// Link `src` to `dst` using the cheapest mechanism that succeeds. Returns the
/// name of the strategy used.
pub fn link_best(src: &Path, dst: &Path) -> VtaResult<&'static str> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| err(dst, e))?;
    }
    let _ = fs::remove_file(dst); // replace an existing link/file

    if fs::hard_link(src, dst).is_ok() {
        return Ok("hardlink");
    }
    if symlink(src, dst).is_ok() {
        return Ok("symlink");
    }
    fs::copy(src, dst).map_err(|e| err(dst, e))?;
    Ok("copy")
}

#[cfg(unix)]
fn symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

#[cfg(windows)]
fn symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(src, dst)
}

fn err(path: &Path, e: std::io::Error) -> VtaError {
    VtaError::new(Area::Store, 3, format!("linking {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_a_file() {
        let dir = std::env::temp_dir().join(format!("vanta-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.bin");
        let dst = dir.join("bin/tool");
        fs::write(&src, b"#!/bin/true").unwrap();
        let how = link_best(&src, &dst).unwrap();
        assert!(["hardlink", "symlink", "copy"].contains(&how));
        assert_eq!(fs::read(&dst).unwrap(), b"#!/bin/true");
        let _ = fs::remove_dir_all(&dir);
    }
}
