//! Shared loop-guard: Construct-managed files must never trigger processing.
use std::path::Path;

/// True if `path` (absolute) is a Construct-managed file that must NOT trigger
/// any pipeline. `vault_root` is the vault's absolute path; `journal_folder` and
/// `managed_folder` are vault-relative folder names (e.g. "journal", "Construct").
pub fn is_excluded(
    path: &Path,
    vault_root: &Path,
    journal_folder: &str,
    managed_folder: Option<&str>,
) -> bool {
    // _index notes (any directory) are managed indices.
    if path.file_stem().and_then(|s| s.to_str()) == Some("_index") {
        return true;
    }
    let Ok(rel) = path.strip_prefix(vault_root) else {
        // Outside the vault entirely → exclude (not ours to touch).
        return true;
    };
    let first = rel.components().next().and_then(|c| c.as_os_str().to_str());
    if first == Some(journal_folder) {
        return true;
    }
    if let (Some(mf), Some(f)) = (managed_folder, first) {
        if f == mf {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn vault() -> PathBuf {
        PathBuf::from("/v")
    }

    #[test]
    fn excludes_index_notes_anywhere() {
        assert!(is_excluded(
            &PathBuf::from("/v/Inbox/_index.md"),
            &vault(),
            "journal",
            None
        ));
        assert!(is_excluded(
            &PathBuf::from("/v/_index.md"),
            &vault(),
            "journal",
            None
        ));
    }

    #[test]
    fn excludes_journal_tree() {
        assert!(is_excluded(
            &PathBuf::from("/v/journal/2026/06/02.md"),
            &vault(),
            "journal",
            None
        ));
    }

    #[test]
    fn excludes_managed_folder() {
        assert!(is_excluded(
            &PathBuf::from("/v/Construct/x.md"),
            &vault(),
            "journal",
            Some("Construct")
        ));
    }

    #[test]
    fn allows_normal_notes() {
        assert!(!is_excluded(
            &PathBuf::from("/v/Inbox/idea.md"),
            &vault(),
            "journal",
            Some("Construct")
        ));
        assert!(!is_excluded(
            &PathBuf::from("/v/Projects/x.md"),
            &vault(),
            "journal",
            None
        ));
    }

    #[test]
    fn excludes_outside_vault() {
        assert!(is_excluded(
            &PathBuf::from("/other/x.md"),
            &vault(),
            "journal",
            None
        ));
    }
}
