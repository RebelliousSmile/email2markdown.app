use anyhow::Context;
use regex::Regex;
use std::fs;
use std::path::Path;

/// Limit the depth of quoted messages to reduce redundancy.
pub fn limit_quote_depth(text: &str, max_depth: usize) -> String {
    text.lines()
        .filter(|line| {
            let quote_level = line.chars().take_while(|&c| c == '>').count();
            quote_level <= max_depth
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract short name (initials) from email address.
pub fn get_short_name(email_str: Option<&str>) -> String {
    let email = match email_str {
        Some(s) if !s.is_empty() => s,
        _ => return "UNK".to_string(),
    };

    // Handle "Name <email@domain>" format: extract the display name part
    let name_str;
    let name_part = if let Some(angle_pos) = email.find('<') {
        let name = email[..angle_pos].trim();
        if name.is_empty() {
            // No display name, use local part of email
            name_str = email[angle_pos + 1..].trim_end_matches('>').to_string();
            name_str.split('@').next().unwrap_or(&name_str)
        } else {
            name
        }
    } else if email.contains('@') {
        // Plain email address: use local part
        email.split('@').next().unwrap_or(email)
    } else {
        // Plain name
        email
    };

    // Get initials or short name
    let words: Vec<&str> = name_part.split_whitespace().collect();
    let short_name = if words.len() == 1 {
        // Single word: use first 3 letters
        words[0].chars().take(3).collect::<String>().to_uppercase()
    } else {
        // Multiple words: use first letter of each word (max 3 words)
        words
            .iter()
            .take(3)
            .filter_map(|w| w.chars().next())
            .collect::<String>()
            .to_uppercase()
    };

    // Clean up any remaining special characters
    let re = Regex::new(r"[^A-Z]").unwrap();
    let result = re.replace_all(&short_name, "").to_string();

    if result.is_empty() {
        "UNK".to_string()
    } else {
        result
    }
}

/// Extract email addresses from a text field.
pub fn extract_emails(text: Option<&str>) -> Vec<String> {
    let text = match text {
        Some(s) => s,
        None => return Vec::new(),
    };

    let re = Regex::new(r"[\w\.-]+@[\w\.-]+\.\w+").unwrap();
    re.find_iter(text)
        .map(|m| m.as_str().to_lowercase())
        .collect()
}

/// Normalize line breaks to max 2 consecutive newlines.
pub fn normalize_line_breaks(text: &str) -> String {
    let re = Regex::new(r"\n{3,}").unwrap();
    re.replace_all(text, "\n\n").to_string()
}

/// Decode MIME encoded filenames (format: =?utf-8?q?filename?=).
pub fn decode_mime_filename(encoded_filename: &str) -> String {
    if encoded_filename.starts_with("=?") && encoded_filename.contains("?=") {
        let re = Regex::new(r"=\?(.*?)\?(.*?)\?(.*?)\?=").unwrap();
        if let Some(caps) = re.captures(encoded_filename) {
            let charset = caps.get(1).map_or("", |m| m.as_str());
            let encoding = caps.get(2).map_or("", |m| m.as_str());
            let encoded_text = caps.get(3).map_or("", |m| m.as_str());

            match encoding.to_lowercase().as_str() {
                "q" => {
                    // Quoted-printable encoding
                    if let Ok(decoded) = quoted_printable_decode(encoded_text, charset) {
                        return decoded;
                    }
                }
                "b" => {
                    // Base64 encoding
                    if let Ok(decoded) = base64_decode(encoded_text, charset) {
                        return decoded;
                    }
                }
                _ => {}
            }
        }
    }
    encoded_filename.to_string()
}

fn quoted_printable_decode(text: &str, charset: &str) -> Result<String, ()> {
    let text = text.replace('_', " ");
    let mut result = Vec::new();
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '=' {
            if let (Some(h1), Some(h2)) = (chars.next(), chars.next()) {
                let hex = format!("{}{}", h1, h2);
                if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                    result.push(byte);
                    continue;
                }
            }
        } else {
            result.push(c as u8);
        }
    }

    decode_bytes(&result, charset)
}

fn base64_decode(text: &str, charset: &str) -> Result<String, ()> {
    use std::collections::HashMap;

    let base64_table: HashMap<char, u8> = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        .chars()
        .enumerate()
        .map(|(i, c)| (c, i as u8))
        .collect();

    let chars: Vec<u8> = text
        .chars()
        .filter(|&c| c != '=')
        .filter_map(|c| base64_table.get(&c).copied())
        .collect();

    let mut bytes = Vec::new();
    for chunk in chars.chunks(4) {
        if chunk.is_empty() {
            continue;
        }
        let n = chunk.len();
        let mut val: u32 = 0;
        for (i, &b) in chunk.iter().enumerate() {
            val |= (b as u32) << (6 * (3 - i));
        }

        bytes.push((val >> 16) as u8);
        if n > 2 {
            bytes.push((val >> 8) as u8);
        }
        if n > 3 {
            bytes.push(val as u8);
        }
    }

    decode_bytes(&bytes, charset)
}

fn decode_bytes(bytes: &[u8], charset: &str) -> Result<String, ()> {
    use encoding_rs::*;

    let encoding = match charset.to_uppercase().as_str() {
        "UTF-8" | "UTF8" => UTF_8,
        "ISO-8859-1" | "LATIN1" | "LATIN-1" => WINDOWS_1252,
        "ISO-8859-15" => ISO_8859_15,
        "WINDOWS-1252" | "CP1252" => WINDOWS_1252,
        _ => UTF_8,
    };

    let (decoded, _, _) = encoding.decode(bytes);
    Ok(decoded.to_string())
}

/// [2] Decode IMAP modified UTF-7 encoding for folder names (complet).
pub fn decode_imap_utf7(encoded_str: &str) -> String {
    if !encoded_str.contains('&') {
        return encoded_str.to_string();
    }

    let mut result = String::new();
    let mut chars = encoded_str.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '&' {
            // Check for &- which is literal &
            if chars.peek() == Some(&'-') {
                chars.next();
                result.push('&');
                continue;
            }

            // Collect base64 encoded part until -
            let mut encoded = String::new();
            while let Some(&next) = chars.peek() {
                if next == '-' {
                    chars.next();
                    break;
                }
                encoded.push(chars.next().unwrap());
            }

            if encoded.is_empty() {
                result.push('&');
                continue;
            }

            // Decode modified base64 to UTF-16BE
            match decode_modified_base64(&encoded) {
                Some(decoded) => result.push_str(&decoded),
                None => {
                    // Fallback: keep original
                    result.push('&');
                    result.push_str(&encoded);
                    result.push('-');
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Decode modified base64 (IMAP UTF-7) to string.
fn decode_modified_base64(encoded: &str) -> Option<String> {
    // IMAP modified base64 uses , instead of /
    let standard = encoded.replace(',', "/");

    // Add padding if needed
    let padded = match standard.len() % 4 {
        2 => format!("{}==", standard),
        3 => format!("{}=", standard),
        _ => standard,
    };

    // Decode base64
    let bytes = base64_decode_simple(&padded)?;

    // Decode as UTF-16BE
    if bytes.len() % 2 != 0 {
        return None;
    }

    let utf16: Vec<u16> = bytes
        .chunks(2)
        .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
        .collect();

    String::from_utf16(&utf16).ok()
}

/// Simple base64 decoder.
fn base64_decode_simple(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut output = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits = 0;

    for c in input.chars() {
        if c == '=' {
            break;
        }

        let value = TABLE.iter().position(|&x| x == c as u8)? as u32;
        buffer = (buffer << 6) | value;
        bits += 6;

        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }

    Some(output)
}

/// Check if a filename represents a signature image.
pub fn is_signature_image(
    attachment_filename: Option<&str>,
    content_type: &str,
    payload_size: usize,
    content_disposition: Option<&str>,
) -> bool {
    let filename_lower = attachment_filename
        .unwrap_or("")
        .to_lowercase();

    // Common signature image patterns
    let signature_patterns = [
        "signature", "logo", "banner", "footer", "header",
        "company", "corporate", "brand", "societe", "entreprise",
    ];

    // Check 1: Common signature filenames (only if small)
    for pattern in &signature_patterns {
        if filename_lower.contains(pattern) {
            let size_limit = if filename_lower.contains("signature") {
                50 * 1024 // Signature images are typically small (< 50KB)
            } else if filename_lower.contains("logo") {
                60 * 1024 // Logos can be a bit larger (< 60KB)
            } else {
                80 * 1024 // Other signature-related images (< 80KB)
            };

            if payload_size < size_limit {
                return true;
            }
        }
    }

    // Check 2: Very small image files (likely logos/signatures)
    if content_type.starts_with("image/") && payload_size < 50 * 1024 {
        return true;
    }

    // Check 3: Inline disposition (embedded images)
    if let Some(disposition) = content_disposition {
        let disposition_lower = disposition.to_lowercase();
        if disposition_lower.contains("inline") {
            return true;
        }
    }

    // Check 4: Common image extensions with generic names
    let common_image_extensions = [".png", ".jpg", ".jpeg", ".gif", ".bmp", ".svg"];
    let generic_names = ["image", "img", "picture", "pic", "photo"];

    if common_image_extensions.iter().any(|ext| filename_lower.ends_with(ext))
        && payload_size < 100 * 1024
    {
        if generic_names.iter().any(|name| filename_lower.starts_with(name)) {
            return true;
        }
    }

    false
}

/// Generate MD5 hash prefix for uniqueness.
pub fn hash_md5_prefix(text: &str, length: usize) -> String {
    let digest = md5::compute(text.as_bytes());
    format!("{:x}", digest)
        .chars()
        .take(length)
        .collect()
}

/// Sanitize filename for filesystem.
pub fn sanitize_filename(filename: &str) -> String {
    let re = Regex::new(r#"[<>:"/\\|?*]"#).unwrap();
    re.replace_all(filename, "_").to_string()
}

/// Get relative path between two paths.
pub fn get_relative_path(from: &Path, to: &Path) -> String {
    if let Ok(rel) = to.strip_prefix(from) {
        rel.to_string_lossy().to_string()
    } else {
        to.to_string_lossy().to_string()
    }
}

/// Check whether a file name is considered OS junk (case-insensitive).
fn is_junk_file_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("Thumbs.db")
        || name.eq_ignore_ascii_case(".DS_Store")
        || name.eq_ignore_ascii_case("desktop.ini")
}

/// Bottom-up prune of empty directories under `root`.
///
/// A directory is considered empty when it contains nothing but OS junk files
/// (`Thumbs.db`, `.DS_Store`, `desktop.ini`, case-insensitive) and/or
/// subdirectories that were themselves pruned by the recursive pass. Junk
/// files are never deleted on their own — they are only removed together with
/// their parent directory.
///
/// Symlinks (both file and directory) are always treated as content: they are
/// never followed and never deleted, even if the target looks empty.
pub fn cleanup_empty_dirs(root: &Path) -> anyhow::Result<()> {
    // Defensive early-returns: empty path, missing target, or non-directory.
    if root.as_os_str().is_empty() {
        return Ok(());
    }
    if !root.exists() {
        return Ok(());
    }
    if !root.is_dir() {
        return Ok(());
    }

    // First pass: recurse into every real subdirectory, bottom-up.
    let entries = fs::read_dir(root).context("failed to read directory")?;
    for entry in entries {
        let entry = entry.context("failed to read directory entry")?;
        let file_type = entry
            .file_type()
            .context("failed to read entry file type")?;
        if file_type.is_dir() && !file_type.is_symlink() {
            cleanup_empty_dirs(&entry.path())?;
        }
    }

    // Second pass: classify what remains after the recursion.
    let mut junk_files: Vec<std::path::PathBuf> = Vec::new();
    let mut has_content = false;
    let remaining = fs::read_dir(root).context("failed to re-read directory")?;
    for entry in remaining {
        let entry = entry.context("failed to read directory entry")?;
        let file_type = entry
            .file_type()
            .context("failed to read entry file type")?;
        let path = entry.path();

        if file_type.is_symlink() {
            // Symlinks always count as content — never follow, never delete.
            has_content = true;
            continue;
        }
        if file_type.is_dir() {
            // Subdir survived recursion → real content.
            has_content = true;
            continue;
        }
        if file_type.is_file() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if is_junk_file_name(&name_str) {
                junk_files.push(path);
            } else {
                has_content = true;
            }
            continue;
        }
        // Unknown entry kind (e.g. block device): treat as content to stay safe.
        has_content = true;
    }

    if has_content {
        return Ok(());
    }

    // Only junk (or nothing) remains: delete junk, then the directory itself.
    for junk in junk_files {
        fs::remove_file(&junk).context("failed to remove junk file")?;
    }
    fs::remove_dir(root).context("failed to remove empty directory")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_limit_quote_depth() {
        let text = "Hello\n> First quote\n>> Second quote\n>>> Third quote\n> Back to first";
        let result = limit_quote_depth(text, 1);
        let expected = "Hello\n> First quote\n> Back to first";
        assert_eq!(result, expected);
    }

    #[test]
    fn test_limit_quote_depth_no_quotes() {
        let text = "Hello\nWorld";
        let result = limit_quote_depth(text, 1);
        assert_eq!(result, text);
    }

    #[test]
    fn test_get_short_name() {
        assert_eq!(get_short_name(Some("sender@example.com")), "SEN");
        assert_eq!(get_short_name(Some("John Doe <john@example.com>")), "JD");
        assert_eq!(get_short_name(Some("John Michael Doe")), "JMD");
        assert_eq!(get_short_name(None), "UNK");
        assert_eq!(get_short_name(Some("")), "UNK");
    }

    #[test]
    fn test_extract_emails() {
        let result = extract_emails(Some("Name <email@domain.com>"));
        assert_eq!(result, vec!["email@domain.com"]);

        let result = extract_emails(Some("a@b.com, c@d.com"));
        assert_eq!(result, vec!["a@b.com", "c@d.com"]);

        let result = extract_emails(None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_normalize_line_breaks() {
        let text = "Hello\n\n\n\nWorld";
        let result = normalize_line_breaks(text);
        assert_eq!(result, "Hello\n\nWorld");
    }

    #[test]
    fn test_is_signature_image() {
        assert!(is_signature_image(Some("signature.png"), "image/png", 1024, Some("inline")));
        assert!(is_signature_image(Some("logo.jpg"), "image/jpeg", 5120, Some("attachment")));
        assert!(!is_signature_image(Some("contract.pdf"), "application/pdf", 102400, Some("attachment")));
        assert!(!is_signature_image(Some("photo_vacation.jpg"), "image/jpeg", 2048000, Some("attachment")));
    }

    #[test]
    fn test_hash_md5_prefix() {
        let hash = hash_md5_prefix("Test Subject", 6);
        assert_eq!(hash.len(), 6);
    }

    // [2] Tests ameliores pour UTF-7 IMAP
    #[test]
    fn test_decode_imap_utf7_no_encoding() {
        let result = decode_imap_utf7("INBOX");
        assert_eq!(result, "INBOX");
    }

    #[test]
    fn test_decode_imap_utf7_literal_ampersand() {
        // &- should decode to &
        let result = decode_imap_utf7("Tom &- Jerry");
        assert_eq!(result, "Tom & Jerry");
    }

    #[test]
    fn test_decode_imap_utf7_french_e_acute() {
        // &AOk- = e with acute accent (UTF-16BE: 00E9)
        let result = decode_imap_utf7("&AOk-");
        assert_eq!(result, "é");
    }

    #[test]
    fn test_decode_imap_utf7_complex_folder() {
        // Test folder name with accented characters
        let result = decode_imap_utf7("INBOX.Envoy&AOk-s");
        assert_eq!(result, "INBOX.Envoyés");
    }

    #[test]
    fn test_cleanup_empty_dirs_removes_leaf() {
        let temp = tempfile::TempDir::new().unwrap();
        let leaf = temp.path().join("leaf");
        std::fs::create_dir(&leaf).unwrap();

        cleanup_empty_dirs(temp.path()).unwrap();

        assert!(!leaf.exists());
    }

    #[test]
    fn test_cleanup_empty_dirs_nested_empty_tree() {
        let temp = tempfile::TempDir::new().unwrap();
        let a = temp.path().join("a");
        let c = a.join("b").join("c");
        std::fs::create_dir_all(&c).unwrap();

        cleanup_empty_dirs(temp.path()).unwrap();

        assert!(!a.exists());
    }

    #[test]
    fn test_cleanup_empty_dirs_directory_with_junk_only() {
        let temp = tempfile::TempDir::new().unwrap();
        let dir = temp.path().join("junk_only");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("Thumbs.db"), b"junk").unwrap();

        cleanup_empty_dirs(temp.path()).unwrap();

        assert!(!dir.exists());
    }

    #[test]
    fn test_cleanup_empty_dirs_directory_with_real_content() {
        let temp = tempfile::TempDir::new().unwrap();
        let dir = temp.path().join("real");
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("note.md");
        std::fs::write(&file, b"content").unwrap();

        cleanup_empty_dirs(temp.path()).unwrap();

        assert!(dir.exists());
        assert!(file.exists());
    }

    #[test]
    fn test_cleanup_empty_dirs_mixed_real_and_junk() {
        let temp = tempfile::TempDir::new().unwrap();
        let dir = temp.path().join("mixed");
        std::fs::create_dir(&dir).unwrap();
        let real = dir.join("note.md");
        let junk = dir.join("Thumbs.db");
        std::fs::write(&real, b"content").unwrap();
        std::fs::write(&junk, b"junk").unwrap();

        cleanup_empty_dirs(temp.path()).unwrap();

        assert!(dir.exists());
        assert!(real.exists());
        assert!(junk.exists());
    }

    #[test]
    fn test_cleanup_empty_dirs_partial_tree() {
        let temp = tempfile::TempDir::new().unwrap();
        let populated = temp.path().join("populated");
        let empty_branch = temp.path().join("empty_branch");
        std::fs::create_dir(&populated).unwrap();
        std::fs::create_dir_all(empty_branch.join("child")).unwrap();
        let file = populated.join("note.md");
        std::fs::write(&file, b"content").unwrap();

        cleanup_empty_dirs(temp.path()).unwrap();

        assert!(populated.exists());
        assert!(file.exists());
        assert!(!empty_branch.exists());
    }

    #[test]
    fn test_cleanup_empty_dirs_nonexistent_root() {
        let temp = tempfile::TempDir::new().unwrap();
        let missing = temp.path().join("does_not_exist");

        let result = cleanup_empty_dirs(&missing);

        assert!(result.is_ok());
        assert!(!missing.exists());
    }

    #[test]
    fn test_cleanup_empty_dirs_junk_case_insensitive() {
        let temp = tempfile::TempDir::new().unwrap();
        let dir = temp.path().join("lowercase_junk");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("thumbs.db"), b"junk").unwrap();
        std::fs::write(dir.join(".ds_store"), b"junk").unwrap();

        cleanup_empty_dirs(temp.path()).unwrap();

        assert!(!dir.exists());
    }

    #[test]
    fn test_cleanup_empty_dirs_real_content_with_empty_subdir() {
        let temp = tempfile::TempDir::new().unwrap();
        let parent = temp.path().join("parent");
        let empty_child = parent.join("empty_child");
        std::fs::create_dir_all(&empty_child).unwrap();
        let real = parent.join("note.md");
        std::fs::write(&real, b"content").unwrap();

        cleanup_empty_dirs(temp.path()).unwrap();

        assert!(parent.exists());
        assert!(real.exists());
        assert!(!empty_child.exists());
    }

    #[test]
    fn test_cleanup_empty_dirs_empty_root_arg() {
        let result = cleanup_empty_dirs(Path::new(""));
        assert!(result.is_ok());
    }
}
