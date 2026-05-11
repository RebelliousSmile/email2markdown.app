use anyhow::{Context, Result};
use regex::Regex;
use serde_yaml::Value;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Statistics for the fix operation.
#[derive(Debug, Default)]
pub struct FixStats {
    pub total_scanned: usize,
    pub files_fixed: usize,
    pub files_rewritten: usize,
    pub errors: usize,
}

/// Fix complex YAML tags in email frontmatter.
pub fn fix_complex_yaml_tags(content: &str) -> String {
    let mut fixed = content.to_string();

    // Remove Python object tags
    let re_python = Regex::new(r"!!python/object:\w+\.").unwrap();
    fixed = re_python.replace_all(&fixed, "").to_string();

    // Remove YAML anchors and aliases
    let re_anchor = Regex::new(r"&\w+").unwrap();
    fixed = re_anchor.replace_all(&fixed, "").to_string();

    let re_alias = Regex::new(r"\*\w+").unwrap();
    fixed = re_alias.replace_all(&fixed, "").to_string();

    // Remove complex tuple structures
    let re_tuple = Regex::new(r"(?s)!!python/tuple\s*\[.*?\]").unwrap();
    fixed = re_tuple.replace_all(&fixed, "").to_string();

    // Clean up subject field specifically
    let re_subject = Regex::new(r"(?s)subject:\s*!!python/object:.*?_chunks:\s*\[(.*?)\]").unwrap();
    if let Some(caps) = re_subject.captures(&fixed) {
        let chunks = caps.get(1).map_or("", |m| m.as_str());
        // Extract the actual subject text from chunks
        let re_text = Regex::new(r#"-\s*(['"])(.*?)\1"#).unwrap();
        if let Some(text_match) = re_text.captures(chunks) {
            let subject_text = text_match.get(2).map_or("Unknown", |m| m.as_str());
            fixed = re_subject
                .replace_all(&fixed, format!("subject: \"{}\"", subject_text))
                .to_string();
        } else {
            fixed = re_subject
                .replace_all(&fixed, "subject: \"Unknown\"")
                .to_string();
        }
    }

    // Remove any remaining charset objects
    let re_charset = Regex::new(
        r"(?s)!!python/object:email\.charset\.Charset.*?input_charset:.*?\n\s*header_encoding:.*?\n\s*body_encoding:.*?\n\s*output_charset:.*?\n\s*input_codec:.*?\n\s*output_codec:.*?"
    ).unwrap();
    fixed = re_charset.replace_all(&fixed, "").to_string();

    fixed
}

/// Extract frontmatter and body from markdown content.
pub fn extract_frontmatter(content: &str) -> Option<(String, String)> {
    if !content.starts_with("---") {
        return None;
    }

    let lines: Vec<&str> = content.lines().collect();
    let mut frontmatter_lines = Vec::new();
    let mut body_start = 0;
    let mut found_end = false;

    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            continue;
        }
        if line.trim() == "---" {
            body_start = i + 1;
            found_end = true;
            break;
        }
        frontmatter_lines.push(*line);
    }

    if !found_end {
        return None;
    }

    let frontmatter = frontmatter_lines.join("\n");
    let body = lines[body_start..].join("\n");

    Some((frontmatter, body))
}

/// Fix a single email markdown file.
pub fn fix_email_file(file_path: &Path, dry_run: bool) -> Result<bool> {
    let content = fs::read_to_string(file_path)
        .context("Failed to read file")?;

    // Check if file needs fixing
    if !content.contains("!!python/object:") {
        return Ok(false);
    }

    println!("Fixing: {}", file_path.display());

    // Try the regex approach first
    let fixed_content = fix_complex_yaml_tags(&content);

    if dry_run {
        return Ok(true);
    }

    // Try to parse the fixed YAML
    if let Some((frontmatter, body)) = extract_frontmatter(&fixed_content) {
        match serde_yaml::from_str::<Value>(&frontmatter) {
            Ok(_) => {
                // YAML parses successfully, save the fixed file
                fs::write(file_path, &fixed_content)?;
                println!("  Fixed: {}", file_path.display());
                Ok(true)
            }
            Err(_) => {
                // YAML parsing failed, try to rewrite frontmatter
                println!("  Complex YAML structure, attempting rewrite...");

                let simple_frontmatter = create_simple_frontmatter(&content);
                let new_content = format!(
                    "---\n{}---\n\n{}",
                    serde_yaml::to_string(&simple_frontmatter)?,
                    body
                );

                fs::write(file_path, &new_content)?;
                println!("  Rewritten: {}", file_path.display());
                Ok(true)
            }
        }
    } else {
        println!("  No frontmatter in: {}", file_path.display());
        Ok(false)
    }
}

/// Create a simple frontmatter structure from complex content.
fn create_simple_frontmatter(content: &str) -> serde_yaml::Value {
    use serde_yaml::Mapping;

    let mut frontmatter = Mapping::new();

    // Try to extract simple fields
    let fields = ["from", "to", "date"];
    for field in &fields {
        let pattern = format!(r"{}:\s*([^\n]+)", field);
        if let Ok(re) = Regex::new(&pattern) {
            if let Some(caps) = re.captures(content) {
                let value = caps.get(1).map_or("Unknown", |m| m.as_str().trim());
                frontmatter.insert(
                    serde_yaml::Value::String(field.to_string()),
                    serde_yaml::Value::String(value.to_string()),
                );
            } else {
                frontmatter.insert(
                    serde_yaml::Value::String(field.to_string()),
                    serde_yaml::Value::String("Unknown".to_string()),
                );
            }
        }
    }

    // Try to extract subject
    let re_subject = Regex::new(r#"subject:.*?(['"])(.*?)\1"#).ok();
    let subject = if let Some(re) = re_subject {
        re.captures(content)
            .and_then(|caps| caps.get(2))
            .map_or("Unknown", |m| m.as_str())
    } else {
        "Unknown"
    };
    frontmatter.insert(
        serde_yaml::Value::String("subject".to_string()),
        serde_yaml::Value::String(subject.to_string()),
    );

    // Add empty tags and attachments
    frontmatter.insert(
        serde_yaml::Value::String("tags".to_string()),
        serde_yaml::Value::Sequence(Vec::new()),
    );
    frontmatter.insert(
        serde_yaml::Value::String("attachments".to_string()),
        serde_yaml::Value::Sequence(Vec::new()),
    );

    serde_yaml::Value::Mapping(frontmatter)
}

/// Scan and fix directory for malformed email files.
pub fn scan_and_fix_directory(
    directory: &Path,
    dry_run: bool,
    on_progress: Option<&(dyn Fn(usize, usize, &str) + Send + Sync)>,
) -> Result<FixStats> {
    let mut stats = FixStats::default();

    let entries: Vec<PathBuf> = if directory.is_file() {
        vec![directory.to_path_buf()]
    } else {
        WalkDir::new(directory)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().extension().map_or(false, |ext| ext == "md")
                    && !e.path().to_string_lossy().contains("attachments")
            })
            .map(|e| e.path().to_path_buf())
            .collect()
    };

    let total = entries.len();

    for (i, file_path) in entries.into_iter().enumerate() {
        if let Some(cb) = on_progress {
            cb(i + 1, total, "Fix YAML");
        }
        stats.total_scanned += 1;

        match fix_email_file(&file_path, dry_run) {
            Ok(true) => stats.files_fixed += 1,
            Ok(false) => {} // No fixing needed
            Err(e) => {
                println!("  Error processing {}: {}", file_path.display(), e);
                stats.errors += 1;
            }
        }
    }

    Ok(stats)
}

/// Print summary of fix operation.
pub fn print_summary(stats: &FixStats, dry_run: bool) {
    println!("\nSummary:");
    println!("   Total email files scanned: {}", stats.total_scanned);
    println!("   Files needing fixes: {}", stats.files_fixed);

    if dry_run {
        println!("   Use --apply to fix these files");
    } else {
        println!("   Files successfully fixed: {}", stats.files_fixed);
    }

    if stats.errors > 0 {
        println!("   Errors encountered: {}", stats.errors);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fix_complex_yaml_tags() {
        let content = "subject: !!python/object:email.header.Header test";
        let fixed = fix_complex_yaml_tags(content);
        assert!(!fixed.contains("!!python/object:"));
    }

    #[test]
    fn test_extract_frontmatter() {
        let content = "---\nfrom: test@example.com\n---\n\nBody content";
        let result = extract_frontmatter(content);
        assert!(result.is_some());

        let (frontmatter, body) = result.unwrap();
        assert!(frontmatter.contains("from:"));
        assert!(body.contains("Body content"));
    }

    #[test]
    fn test_extract_frontmatter_no_closing() {
        let content = "---\nfrom: test@example.com\n\nBody content";
        let result = extract_frontmatter(content);
        assert!(result.is_none());
    }
}
