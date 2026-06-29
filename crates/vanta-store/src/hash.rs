//! Canonical content hashing for the store (`docs/09-store.md`).
//!
//! Store keys are `blake3-<hex>` over the *canonicalized* materialized tree:
//! entries are visited in sorted order, the executable bit is normalized,
//! timestamps are excluded, and symlinks are hashed by target. This makes the
//! same content hash identically across machines and filesystems.

use blake3::Hasher;
use std::fs;
use std::path::{Path, PathBuf};
use vanta_core::{Area, VtaError, VtaResult};

/// Hash a byte slice, returning a `blake3-<hex>` key.
pub fn hash_bytes(bytes: &[u8]) -> String {
    format!("blake3-{}", blake3::hash(bytes).to_hex())
}

/// Hash a directory tree canonically, returning a `blake3-<hex>` store key.
pub fn hash_tree(root: &Path) -> VtaResult<String> {
    let mut files = Vec::new();
    collect(root, root, &mut files)?;
    files.sort();

    let mut hasher = Hasher::new();
    for rel in &files {
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update(&[0]);
        let full = root.join(rel);
        let meta = fs::symlink_metadata(&full).map_err(|e| io_err(&full, e))?;
        if meta.file_type().is_symlink() {
            let target = fs::read_link(&full).map_err(|e| io_err(&full, e))?;
            hasher.update(b"L");
            hasher.update(target.to_string_lossy().as_bytes());
        } else {
            hasher.update(if is_executable(&meta) { b"X" } else { b"F" });
            let contents = fs::read(&full).map_err(|e| io_err(&full, e))?;
            hasher.update(&(contents.len() as u64).to_le_bytes());
            hasher.update(&contents);
        }
        hasher.update(&[0]);
    }
    Ok(format!("blake3-{}", hasher.finalize().to_hex()))
}

/// Recursively collect file paths (relative to `root`) under `dir`.
fn collect(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> VtaResult<()> {
    for entry in fs::read_dir(dir).map_err(|e| io_err(dir, e))? {
        let entry = entry.map_err(|e| io_err(dir, e))?;
        let path = entry.path();
        let ty = entry.file_type().map_err(|e| io_err(&path, e))?;
        if ty.is_dir() {
            collect(root, &path, out)?;
        } else {
            // Files and symlinks are recorded; empty directories are not part of
            // content identity (they carry no bytes).
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            out.push(rel);
        }
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &fs::Metadata) -> bool {
    // On Windows, executability is by extension, not a mode bit. Normalize to
    // false so a given artifact hashes consistently on that platform.
    false
}

fn io_err(path: &Path, e: std::io::Error) -> VtaError {
    VtaError::new(Area::Store, 2, format!("hashing {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("vanta-hash-{}-{}", tag, std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn bytes_have_prefix() {
        assert!(hash_bytes(b"hello").starts_with("blake3-"));
        assert_ne!(hash_bytes(b"a"), hash_bytes(b"b"));
    }

    #[test]
    fn tree_is_deterministic_and_content_sensitive() {
        let d = tmp("tree");
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("a.txt"), b"alpha").unwrap();
        fs::write(d.join("sub/b.txt"), b"beta").unwrap();
        let h1 = hash_tree(&d).unwrap();
        let h2 = hash_tree(&d).unwrap();
        assert_eq!(h1, h2); // deterministic
        fs::write(d.join("a.txt"), b"ALPHA").unwrap();
        assert_ne!(hash_tree(&d).unwrap(), h1); // content-sensitive
        let _ = fs::remove_dir_all(&d);
    }
}
