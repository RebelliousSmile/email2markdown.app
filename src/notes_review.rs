//! Collect `.md` notes for the "Organiser les notes" window.
//!
//! Walks a notes directory (or takes an explicit file list), parses the YAML
//! frontmatter of each `.md`, and produces a flat list of [`NoteEntry`] for the
//! WebView to display, filter and bulk-edit.
//!
//! Recursion never follows symlinks (project rule 02-rust-filesystem-safety).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// One `.md` note as displayed in the organize window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteEntry {
    /// Absolute path on disk — opaque identifier for IPC round-trips.
    pub path: PathBuf,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cc: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_type: Option<String>,

    /// Folder of this note relative to the notes root, e.g. `Travail/Projets/ClientX`.
    /// `None` when the note sits at the root or when collection was per-file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_path: Option<String>,
}

/// Recursively walk `root`, return a [`NoteEntry`] per `.md` file (symlinks skipped).
///
/// Hidden directories (`.archive`, `_generated`, `_local`, anything starting with `.` or `_`)
/// are pruned to avoid surfacing internal/working state.
pub fn collect_notes(root: &Path) -> Result<Vec<NoteEntry>> {
    if root.as_os_str().is_empty() || !root.exists() || !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_pruned_dir(e));

    for entry in walker {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let ft = entry.file_type();
        if ft.is_symlink() || !ft.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("md")) {
            continue;
        }
        match parse_note(path, Some(root)) {
            Ok(note) => out.push(note),
            Err(e) => {
                eprintln!("notes_review: parse skipped {} ({:#})", path.display(), e);
                continue;
            }
        }
    }

    out.sort_by(|a, b| a.date.cmp(&b.date));
    Ok(out)
}

/// Parse a user-picked list of `.md` files. Missing files are silently skipped.
pub fn collect_files(files: &[PathBuf]) -> Result<Vec<NoteEntry>> {
    let mut out = Vec::with_capacity(files.len());
    for fp in files {
        if !fp.exists() || !fp.is_file() {
            continue;
        }
        if fp.extension().is_none_or(|e| !e.eq_ignore_ascii_case("md")) {
            continue;
        }
        match parse_note(fp, None) {
            Ok(note) => out.push(note),
            Err(e) => {
                eprintln!("notes_review: parse skipped {} ({:#})", fp.display(), e);
                continue;
            }
        }
    }
    out.sort_by(|a, b| a.date.cmp(&b.date));
    Ok(out)
}

/// Set of directory names hidden from the organize window — internal storage / VCS metadata
/// that should never be surfaced as user-facing notes.
///
/// Kept narrow on purpose: a user may legitimately create folders starting with `_` or `.`
/// for their own classification scheme (e.g. `_archives-2024`), and we should not silently
/// drop them.
const PRUNED_DIR_NAMES: &[&str] = &[
    ".archive",
    ".archives",
    ".git",
    ".github",
    "__pycache__",
    "_generated",
    "_local",
    "node_modules",
    "target",
];

fn is_pruned_dir(entry: &walkdir::DirEntry) -> bool {
    let ft = entry.file_type();
    if !ft.is_dir() || ft.is_symlink() {
        return false;
    }
    if entry.depth() == 0 {
        return false;
    }
    let Some(name) = entry.file_name().to_str() else {
        return true;
    };
    PRUNED_DIR_NAMES.iter().any(|p| p.eq_ignore_ascii_case(name))
}

fn parse_note(path: &Path, root: Option<&Path>) -> Result<NoteEntry> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read note {}", path.display()))?;
    let meta = extract_frontmatter_map(&content);
    let notes_path = root.and_then(|r| relative_folder(r, path));
    Ok(NoteEntry {
        path: path.to_path_buf(),
        subject: meta_str(&meta, "subject"),
        from: meta_str(&meta, "from").or_else(|| meta_str(&meta, "sender")),
        to: meta_list(&meta, "to"),
        cc: meta_list(&meta, "cc"),
        date: meta_str(&meta, "date"),
        email_type: meta_str(&meta, "email_type"),
        notes_path,
    })
}

fn relative_folder(root: &Path, note: &Path) -> Option<String> {
    let parent = note.parent()?;
    let rel = parent.strip_prefix(root).ok()?;
    let s = rel.to_string_lossy().replace('\\', "/");
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Strip the leading YAML frontmatter block and parse it as a generic map.
/// Returns `serde_yaml::Value::Null` if the file has no frontmatter or it is malformed.
fn extract_frontmatter_map(content: &str) -> serde_yaml::Value {
    let Some(rest) = content.strip_prefix("---\n").or_else(|| content.strip_prefix("---\r\n"))
    else {
        return serde_yaml::Value::Null;
    };
    let end = match rest.find("\n---") {
        Some(i) => i,
        None => return serde_yaml::Value::Null,
    };
    let yaml = &rest[..end];
    serde_yaml::from_str(yaml).unwrap_or(serde_yaml::Value::Null)
}

fn meta_str(meta: &serde_yaml::Value, key: &str) -> Option<String> {
    let v = meta.get(key)?;
    match v {
        serde_yaml::Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn meta_list(meta: &serde_yaml::Value, key: &str) -> Vec<String> {
    let Some(v) = meta.get(key) else {
        return Vec::new();
    };
    match v {
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .filter_map(|x| match x {
                serde_yaml::Value::String(s) => Some(s.trim().to_string()),
                _ => None,
            })
            .filter(|s| !s.is_empty())
            .collect(),
        serde_yaml::Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                Vec::new()
            } else {
                vec![t.to_string()]
            }
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(p: &Path, body: &str) {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn test_collect_notes_walks_recursively_skipping_pruned_dirs() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write(
            &root.join("Travail").join("a.md"),
            "---\nsubject: A\nfrom: x@y\ndate: 2026-01-01\n---\nBody",
        );
        write(
            &root.join("Travail").join("Projet").join("b.md"),
            "---\nsubject: B\nfrom: x@y\ndate: 2026-02-02\nto:\n  - c@d\n  - e@f\n---\nBody",
        );
        write(&root.join("_local").join("skip.md"), "---\nsubject: X\n---\n");
        write(&root.join(".archive").join("old.md"), "---\nsubject: Y\n---\n");

        let notes = collect_notes(root).unwrap();
        assert_eq!(notes.len(), 2);
        let subjects: Vec<_> = notes.iter().filter_map(|n| n.subject.clone()).collect();
        assert!(subjects.contains(&"A".to_string()));
        assert!(subjects.contains(&"B".to_string()));
        assert!(!subjects.contains(&"X".to_string()));
        assert!(!subjects.contains(&"Y".to_string()));
    }

    #[test]
    fn test_collect_notes_extracts_notes_path_relative_to_root() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        write(
            &root.join("Travail").join("Projet").join("a.md"),
            "---\nsubject: A\n---\nBody",
        );
        write(&root.join("at_root.md"), "---\nsubject: R\n---\nBody");

        let notes = collect_notes(root).unwrap();
        let by_subject: std::collections::HashMap<_, _> = notes
            .into_iter()
            .map(|n| (n.subject.clone().unwrap_or_default(), n))
            .collect();
        assert_eq!(
            by_subject.get("A").and_then(|n| n.notes_path.clone()),
            Some("Travail/Projet".to_string())
        );
        assert_eq!(by_subject.get("R").and_then(|n| n.notes_path.clone()), None);
    }

    #[test]
    fn test_collect_files_handles_to_as_scalar_or_list() {
        let temp = TempDir::new().unwrap();
        let a = temp.path().join("a.md");
        let b = temp.path().join("b.md");
        write(&a, "---\nsubject: A\nto: solo@x\n---\n");
        write(&b, "---\nsubject: B\nto:\n  - one@x\n  - two@x\n---\n");
        let notes = collect_files(&[a, b]).unwrap();
        let by_subject: std::collections::HashMap<_, _> = notes
            .into_iter()
            .map(|n| (n.subject.clone().unwrap_or_default(), n))
            .collect();
        assert_eq!(by_subject["A"].to, vec!["solo@x".to_string()]);
        assert_eq!(
            by_subject["B"].to,
            vec!["one@x".to_string(), "two@x".to_string()]
        );
    }

    #[test]
    fn test_collect_notes_empty_or_missing_root_returns_empty() {
        let notes = collect_notes(Path::new("")).unwrap();
        assert!(notes.is_empty());
        let notes = collect_notes(Path::new("Z:\\does\\not\\exist")).unwrap();
        assert!(notes.is_empty());
    }
}
