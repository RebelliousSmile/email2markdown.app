use anyhow::{Context, Result};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Safe path-segment shape — alphanumerics, space, dash, underscore, dot.
/// Used to validate user-typed notes destinations against path-traversal.
static SAFE_PATH_SEGMENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9À-ſ _.\-]+$").expect("static regex"));

/// Join `dest` (user-typed `Travail/Projets/X`) onto `root` safely.
///
/// Rejects segments that:
/// - contain a Windows separator `\`
/// - are `..` or `.` (path-traversal)
/// - contain any character outside `SAFE_PATH_SEGMENT_RE`
///
/// Returns the joined `PathBuf` on success, or an error naming the bad segment.
pub fn join_safe_segments(root: &Path, dest: &str) -> Result<PathBuf> {
    let mut out = root.to_path_buf();
    for raw in dest.split('/') {
        let seg = raw.trim();
        if seg.is_empty() {
            continue;
        }
        if seg == ".." || seg == "." || seg.contains('\\') {
            anyhow::bail!("invalid path segment in notes destination: {:?}", seg);
        }
        if !SAFE_PATH_SEGMENT_RE.is_match(seg) {
            anyhow::bail!(
                "notes destination segment contains forbidden characters: {:?}",
                seg
            );
        }
        out = out.join(seg);
    }
    Ok(out)
}

/// Compute a relative path from `from_dir` to `to`.
/// Both paths should be absolute for correct results.
fn relative_path_from(from_dir: &Path, to: &Path) -> PathBuf {
    let mut from_parts: Vec<_> = from_dir.components().collect();
    let mut to_parts: Vec<_> = to.components().collect();

    // Strip common prefix
    let common = from_parts
        .iter()
        .zip(to_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();
    from_parts.drain(..common);
    to_parts.drain(..common);

    let mut result = PathBuf::new();
    for _ in &from_parts {
        result.push("..");
    }
    for part in to_parts {
        result.push(part);
    }
    result
}

/// Rewrite attachment paths in a `.md` file's YAML frontmatter so they are
/// relative to `new_parent_dir` instead of `old_parent_dir`.
/// Both `old_parent_dir` and `new_parent_dir` must be absolute paths.
pub fn rewrite_attachment_paths(
    md_path: &Path,
    old_parent_dir: &Path,
    new_parent_dir: &Path,
) -> Result<()> {
    let content = fs::read_to_string(md_path)
        .with_context(|| format!("failed to read {}", md_path.display()))?;

    // Only process files that have a YAML frontmatter block.
    let Some(rest) = content.strip_prefix("---\n") else {
        return Ok(());
    };
    let Some(end) = rest.find("\n---") else {
        return Ok(());
    };
    let frontmatter = &rest[..end];
    let after_frontmatter = &rest[end + 4..]; // skip "\n---"

    // Rewrite each line that is an attachment list item: "  - <path>"
    // We look for lines inside an `attachments:` block.
    let mut in_attachments = false;
    let mut new_frontmatter = String::with_capacity(frontmatter.len());

    for line in frontmatter.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("attachments:") {
            in_attachments = true;
            new_frontmatter.push_str(line);
            new_frontmatter.push('\n');
            continue;
        }
        if in_attachments && trimmed.starts_with("- ") {
            // Extract the path after "- "
            let path_str = trimmed.trim_start_matches("- ");
            // Absolute attachment path from old_parent_dir
            let abs = old_parent_dir.join(path_str.replace('/', std::path::MAIN_SEPARATOR_STR));
            // Relative path from new_parent_dir to abs
            let rel = relative_path_from(new_parent_dir, &abs);
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            // Preserve original indentation
            let indent: String = line.chars().take_while(|c| *c == ' ').collect();
            new_frontmatter.push_str(&format!("{}- {}\n", indent, rel_str));
            continue;
        }
        // A non-list line ends the attachments block
        if in_attachments && !trimmed.starts_with('-') {
            in_attachments = false;
        }
        new_frontmatter.push_str(line);
        new_frontmatter.push('\n');
    }

    let new_content = format!("---\n{}---{}", new_frontmatter, after_frontmatter);
    fs::write(md_path, new_content)
        .with_context(|| format!("failed to write updated frontmatter to {}", md_path.display()))?;
    Ok(())
}

/// Move a `.md` file and its sibling `<stem>_attachments/` directory into `dest_dir`.
///
/// Steps:
/// 1. Reject symlinks: if `md_path` is a symlink, return `Err` immediately (no FS mutation).
/// 2. Move `<stem>_attachments/` (if it exists) alongside the `.md` into `dest_dir`.
/// 3. Move the `.md` itself into `dest_dir`.
/// 4. Rewrite attachment paths in the moved `.md` to point to the co-located directory.
///
/// The move is attempted with `fs::rename`; if that crosses device boundaries the
/// fallback is `fs::copy` + `fs::remove_file` / `fs::remove_dir_all`.
pub fn move_email(md_path: &Path, dest_dir: &Path) -> Result<()> {
    // --- Symlink guard (project rule 02-rust-filesystem-safety) ---
    let meta = md_path
        .symlink_metadata()
        .with_context(|| format!("failed to stat {}", md_path.display()))?;
    if meta.file_type().is_symlink() {
        anyhow::bail!(
            "refusing to move symlink: {}",
            md_path.display()
        );
    }

    let old_parent = md_path
        .parent()
        .with_context(|| format!("md_path has no parent: {}", md_path.display()))?;

    let stem = md_path
        .file_stem()
        .with_context(|| format!("md_path has no stem: {}", md_path.display()))?
        .to_string_lossy()
        .into_owned();

    // --- Move attachments directory if present ---
    let attachments_src = old_parent.join(format!("{}_attachments", stem));
    if attachments_src.exists() {
        let attachments_dest = dest_dir.join(format!("{}_attachments", stem));
        if fs::rename(&attachments_src, &attachments_dest).is_err() {
            // Cross-device fallback: copy then remove
            copy_dir_all(&attachments_src, &attachments_dest).with_context(|| {
                format!(
                    "failed to copy attachments dir from {} to {}",
                    attachments_src.display(),
                    attachments_dest.display()
                )
            })?;
            fs::remove_dir_all(&attachments_src).with_context(|| {
                format!(
                    "failed to remove original attachments dir {}",
                    attachments_src.display()
                )
            })?;
        }
    }

    // --- Move the .md file ---
    let md_dest = dest_dir.join(
        md_path
            .file_name()
            .with_context(|| format!("md_path has no file name: {}", md_path.display()))?,
    );
    if fs::rename(md_path, &md_dest).is_err() {
        fs::copy(md_path, &md_dest).with_context(|| {
            format!(
                "failed to copy {} to {}",
                md_path.display(),
                dest_dir.display()
            )
        })?;
        fs::remove_file(md_path).with_context(|| {
            format!("failed to remove {} after copy", md_path.display())
        })?;
    }

    // --- Rewrite attachment paths in the moved .md ---
    // Both the .md and _attachments/ dir are now co-located in dest_dir, so the
    // relative paths in the YAML are computed relative to dest_dir (both old and new
    // base are dest_dir). This keeps the relative path unchanged when both are moved.
    if let Err(e) = rewrite_attachment_paths(&md_dest, dest_dir, dest_dir) {
        eprintln!(
            "warning: could not update attachment paths in {}: {}",
            md_dest.display(),
            e
        );
    }

    Ok(())
}

/// Recursively copy a directory tree from `src` to `dst`.
fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)
        .with_context(|| format!("failed to create directory {}", dst.display()))?;
    for entry in fs::read_dir(src)
        .with_context(|| format!("failed to read directory {}", src.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read entry in {}", src.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to get file type for {}", entry.path().display()))?;
        // Never follow symlinks (project rule 02-rust-filesystem-safety)
        if file_type.is_symlink() {
            continue;
        }
        let dest_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &dest_path)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &dest_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    entry.path().display(),
                    dest_path.display()
                )
            })?;
        }
    }
    Ok(())
}
