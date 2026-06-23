use crate::frontmatter::Note;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// List relative folder paths under `root`, skipping dotfolders and `exclude`.
pub fn list_folders(root: &Path, exclude: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    walk_dirs(root, root, exclude, &mut out);
    out.sort();
    out
}

fn walk_dirs(root: &Path, dir: &Path, exclude: &[String], out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name.starts_with('.') || exclude.iter().any(|e| e == name) {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().to_string());
        }
        walk_dirs(root, &path, exclude, out);
    }
}

/// Gather the set of tags already used across the vault (frontmatter + inline).
pub fn existing_tags(root: &Path) -> Vec<String> {
    existing_tags_excluding(root, &[])
}

/// Like [`existing_tags`], but skips any directory whose name appears in `exclude`.
pub fn existing_tags_excluding(root: &Path, exclude: &[String]) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    collect_tags(root, exclude, &mut set);
    set.into_iter().collect()
}

fn collect_tags(dir: &Path, exclude: &[String], set: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !name.starts_with('.') && !exclude.iter().any(|e| e == name) {
                collect_tags(&path, exclude, set);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                let note = Note::parse(&text);
                for t in note.tags() {
                    set.insert(t);
                }
                if let Some(serde_yaml::Value::Sequence(seq)) =
                    note.frontmatter.get(serde_yaml::Value::from("tags"))
                {
                    for v in seq {
                        if let Some(s) = v.as_str() {
                            set.insert(s.to_string());
                        }
                    }
                }
            }
        }
    }
}

/// Recursively list all `.md` files under `root` (absolute paths), skipping
/// dotfolders and any directory whose name appears in `exclude`.
pub fn walk_notes(root: &Path, exclude: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_notes(root, exclude, &mut out);
    out
}

fn collect_notes(dir: &Path, exclude: &[String], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !name.starts_with('.') && !exclude.iter().any(|e| e == name) {
                collect_notes(&path, exclude, out);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    #[test]
    fn lists_folders_skipping_dot_and_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("Projects/Active")).unwrap();
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        std::fs::create_dir_all(root.join("Archive")).unwrap();
        let folders = list_folders(root, &["Archive".to_string()]);
        assert!(folders.contains(&"Projects".to_string()));
        assert!(folders.contains(&"Projects/Active".to_string()));
        assert!(!folders.iter().any(|f| f.contains(".obsidian")));
        assert!(!folders.contains(&"Archive".to_string()));
    }

    #[test]
    fn gathers_existing_tags() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("a.md"),
            "---\ntags:\n- rust\n- cli\n---\nbody #project",
        );
        write(&root.join("sub/b.md"), "body #rust");
        let tags = existing_tags(root);
        assert!(tags.contains(&"rust".to_string()));
        assert!(tags.contains(&"cli".to_string()));
        assert!(tags.contains(&"project".to_string()));
    }

    #[test]
    fn walk_notes_lists_md_recursively_skipping_dot_and_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("a.md"), "x");
        write(&root.join("sub/b.md"), "y");
        write(&root.join(".obsidian/c.md"), "z");
        write(&root.join("Archive/old.md"), "w");
        write(&root.join("notes.txt"), "not md");
        let mut found: Vec<String> = walk_notes(root, &["Archive".to_string()])
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().to_string())
            .collect();
        found.sort();
        assert_eq!(found, vec!["a.md".to_string(), "sub/b.md".to_string()]);
    }

    #[test]
    fn existing_tags_skips_excluded_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("a.md"), "body #keep");
        write(&root.join("Archive/old.md"), "body #secret");
        let tags = existing_tags_excluding(root, &["Archive".to_string()]);
        assert!(tags.contains(&"keep".to_string()));
        assert!(!tags.contains(&"secret".to_string()));
    }
}
