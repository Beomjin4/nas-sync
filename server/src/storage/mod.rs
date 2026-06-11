use crate::error::{AppError, AppResult};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Storage {
    pub vault: PathBuf,
    pub trash: PathBuf,
    pub conflicts: PathBuf,
}

impl Storage {
    pub fn new(vault: PathBuf, trash: PathBuf, conflicts: PathBuf) -> Self {
        Self {
            vault,
            trash,
            conflicts,
        }
    }

    pub async fn ensure_dirs(&self) -> std::io::Result<()> {
        tokio::fs::create_dir_all(&self.vault).await?;
        tokio::fs::create_dir_all(&self.trash).await?;
        tokio::fs::create_dir_all(&self.conflicts).await?;
        Ok(())
    }

    /// Resolve a request path inside the vault, rejecting traversal and absolute paths.
    /// Returns the absolute on-disk path and the canonical relative path (forward slashes).
    pub fn resolve_vault(&self, rel: &str) -> AppResult<(PathBuf, String)> {
        let (abs, canon) = safe_join(&self.vault, rel).ok_or(AppError::InvalidPath)?;
        Ok((abs, canon))
    }

    pub fn trash_target(&self, sub: &str) -> AppResult<PathBuf> {
        let (abs, _) = safe_join(&self.trash, sub).ok_or(AppError::InvalidPath)?;
        Ok(abs)
    }

    pub fn conflicts_target(&self, sub: &str) -> AppResult<PathBuf> {
        let (abs, _) = safe_join(&self.conflicts, sub).ok_or(AppError::InvalidPath)?;
        Ok(abs)
    }
}

/// Joins `rel` onto `base`, allowing only `Normal` components.
/// Rejects absolute paths, `..`, NUL bytes, and empty/`.` paths.
/// Returns `(absolute_path, canonical_relative_with_forward_slashes)`.
fn safe_join(base: &Path, rel: &str) -> Option<(PathBuf, String)> {
    if rel.is_empty() || rel.contains('\0') {
        return None;
    }
    let mut out = base.to_path_buf();
    let mut canon_parts = Vec::new();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(c) => {
                let s = c.to_str()?;
                if s.is_empty() {
                    return None;
                }
                out.push(c);
                canon_parts.push(s.to_string());
            }
            _ => return None,
        }
    }
    if canon_parts.is_empty() {
        return None;
    }
    Some((out, canon_parts.join("/")))
}

/// blake3 hex digest of the given bytes — used as ETag.
pub fn etag_of(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_and_absolute() {
        let base = Path::new("/vault");
        assert!(safe_join(base, "../etc/passwd").is_none());
        assert!(safe_join(base, "/etc/passwd").is_none());
        assert!(safe_join(base, "a/../b").is_none());
        assert!(safe_join(base, "").is_none());
        assert!(safe_join(base, ".").is_none());
        assert!(safe_join(base, "with\0null").is_none());
    }

    #[test]
    fn accepts_normal_paths_and_canonicalizes() {
        let base = Path::new("/vault");
        let (abs, canon) = safe_join(base, "notes/foo.md").unwrap();
        assert_eq!(abs, Path::new("/vault/notes/foo.md"));
        assert_eq!(canon, "notes/foo.md");
    }
}
