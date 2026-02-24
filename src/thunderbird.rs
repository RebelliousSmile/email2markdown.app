// [1] Import automatique depuis Thunderbird
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use libloading::Library;
use regex::Regex;
use std::collections::HashMap;
use std::env;
use std::ffi::CString;
use std::fs;
use std::os::raw::{c_char, c_void};
use std::path::{Path, PathBuf};

use crate::config::Account;

/// Thunderbird profile information
#[derive(Debug, Clone)]
pub struct ThunderbirdProfile {
    pub name: String,
    pub path: PathBuf,
    pub is_default: bool,
}

/// Get Thunderbird profiles directory based on OS
pub fn get_thunderbird_profiles_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        env::var("APPDATA")
            .ok()
            .map(|appdata| PathBuf::from(appdata).join("Thunderbird").join("Profiles"))
    }

    #[cfg(target_os = "macos")]
    {
        env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join("Library").join("Thunderbird").join("Profiles"))
    }

    #[cfg(target_os = "linux")]
    {
        env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join(".thunderbird"))
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// List available Thunderbird profiles
pub fn list_profiles() -> Result<Vec<ThunderbirdProfile>> {
    let profiles_dir = get_thunderbird_profiles_dir()
        .context("Could not determine Thunderbird profiles directory")?;

    let profiles_ini = profiles_dir.parent()
        .unwrap_or(&profiles_dir)
        .join("profiles.ini");

    if !profiles_ini.exists() {
        // Fallback: scan directory for profile folders
        return scan_profile_directories(&profiles_dir);
    }

    parse_profiles_ini(&profiles_ini, &profiles_dir)
}

/// Parse profiles.ini file
fn parse_profiles_ini(ini_path: &Path, _base_dir: &Path) -> Result<Vec<ThunderbirdProfile>> {
    let content = fs::read_to_string(ini_path)
        .context("Failed to read profiles.ini")?;

    // The base directory for relative paths is the Thunderbird folder (parent of profiles.ini)
    let thunderbird_dir = ini_path.parent().unwrap_or(Path::new("."));

    let mut profiles = Vec::new();
    let mut current_profile: Option<HashMap<String, String>> = None;

    for line in content.lines() {
        let line = line.trim();

        if line.starts_with('[') && line.ends_with(']') {
            // Save previous profile
            if let Some(profile) = current_profile.take() {
                if let Some(p) = build_profile_from_map(&profile, thunderbird_dir) {
                    profiles.push(p);
                }
            }

            // Start new profile section
            if line.to_lowercase().starts_with("[profile") {
                current_profile = Some(HashMap::new());
            }
        } else if let Some(ref mut profile) = current_profile {
            if let Some((key, value)) = line.split_once('=') {
                profile.insert(key.trim().to_lowercase(), value.trim().to_string());
            }
        }
    }

    // Don't forget the last profile
    if let Some(profile) = current_profile {
        if let Some(p) = build_profile_from_map(&profile, thunderbird_dir) {
            profiles.push(p);
        }
    }

    Ok(profiles)
}

fn build_profile_from_map(map: &HashMap<String, String>, base_dir: &Path) -> Option<ThunderbirdProfile> {
    let name = map.get("name")?.clone();
    let path_str = map.get("path")?;
    let is_relative = map.get("isrelative").map(|s| s == "1").unwrap_or(true);
    let is_default = map.get("default").map(|s| s == "1").unwrap_or(false);

    let path = if is_relative {
        base_dir.join(path_str)
    } else {
        PathBuf::from(path_str)
    };

    Some(ThunderbirdProfile {
        name,
        path,
        is_default,
    })
}

/// Scan directory for profile folders (fallback)
fn scan_profile_directories(profiles_dir: &Path) -> Result<Vec<ThunderbirdProfile>> {
    let mut profiles = Vec::new();

    if profiles_dir.exists() {
        for entry in fs::read_dir(profiles_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                let prefs_file = path.join("prefs.js");
                if prefs_file.exists() {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    profiles.push(ThunderbirdProfile {
                        name: name.clone(),
                        path,
                        is_default: name.contains("default"),
                    });
                }
            }
        }
    }

    Ok(profiles)
}

/// Extract IMAP accounts from a Thunderbird profile
pub fn extract_accounts(profile: &ThunderbirdProfile) -> Result<Vec<Account>> {
    let prefs_file = profile.path.join("prefs.js");

    if !prefs_file.exists() {
        anyhow::bail!("prefs.js not found in profile: {}", profile.path.display());
    }

    let content = fs::read_to_string(&prefs_file)
        .context("Failed to read prefs.js")?;

    parse_prefs_js(&content)
}

/// Parse prefs.js and extract IMAP account configurations
fn parse_prefs_js(content: &str) -> Result<Vec<Account>> {
    let mut servers: HashMap<String, HashMap<String, String>> = HashMap::new();

    // Pattern: user_pref("mail.server.server1.property", "value");
    let re = Regex::new(r#"user_pref\("mail\.server\.([^.]+)\.([^"]+)",\s*"?([^")]+)"?\);"#)?;

    for cap in re.captures_iter(content) {
        let server_id = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let property = cap.get(2).map(|m| m.as_str()).unwrap_or("");
        let value = cap.get(3).map(|m| m.as_str()).unwrap_or("");

        servers
            .entry(server_id.to_string())
            .or_default()
            .insert(property.to_string(), value.to_string());
    }

    let mut accounts = Vec::new();

    for (server_id, props) in servers {
        // Only process IMAP accounts
        let server_type = props.get("type").map(|s| s.as_str()).unwrap_or("");
        if server_type != "imap" {
            continue;
        }

        let hostname = match props.get("hostname") {
            Some(h) => h.clone(),
            None => continue,
        };

        let username = props.get("userName").cloned().unwrap_or_default();
        let port = props
            .get("port")
            .and_then(|p| p.parse().ok())
            .unwrap_or(993);

        let name = props
            .get("name")
            .cloned()
            .unwrap_or_else(|| format!("Account_{}", server_id));

        // Clean the name for use as export directory
        let safe_name = sanitize_name(&name);

        accounts.push(Account {
            name: name.clone(),
            server: hostname,
            port,
            username,
            password: None, // Passwords are stored separately in Thunderbird
            export_directory: format!("./exports/{}", safe_name),
            ignored_folders: default_ignored_folders(&name),
            quote_depth: 1,
            skip_existing: true,
            collect_contacts: false,
            skip_signature_images: true,
            delete_after_export: false,
        });
    }

    Ok(accounts)
}

/// Sanitize account name for use as directory name
fn sanitize_name(name: &str) -> String {
    let re = Regex::new(r"[^a-zA-Z0-9_-]").unwrap();
    re.replace_all(name, "_").to_string()
}

/// Get default ignored folders based on account name
fn default_ignored_folders(name: &str) -> Vec<String> {
    let name_lower = name.to_lowercase();

    if name_lower.contains("gmail") {
        vec![
            "[Gmail]/Spam".to_string(),
            "[Gmail]/Trash".to_string(),
            "[Gmail]/All Mail".to_string(),
            "[Gmail]/Drafts".to_string(),
        ]
    } else if name_lower.contains("outlook") || name_lower.contains("hotmail") {
        vec![
            "Junk".to_string(),
            "Deleted Items".to_string(),
            "Drafts".to_string(),
        ]
    } else {
        vec![
            "Spam".to_string(),
            "Trash".to_string(),
            "Junk".to_string(),
            "Drafts".to_string(),
        ]
    }
}

/// Generate accounts.yaml content from extracted accounts
pub fn generate_accounts_yaml(accounts: &[Account]) -> String {
    let mut yaml = String::from("# Auto-generated from Thunderbird configuration\n");
    yaml.push_str("# Review and adjust settings as needed\n");
    yaml.push_str("# Passwords must be added to .env file\n\n");
    yaml.push_str("accounts:\n");

    for account in accounts {
        yaml.push_str(&format!("  - name: \"{}\"\n", account.name));
        yaml.push_str(&format!("    server: \"{}\"\n", account.server));
        yaml.push_str(&format!("    port: {}\n", account.port));
        yaml.push_str(&format!("    username: \"{}\"\n", account.username));
        yaml.push_str(&format!("    export_directory: \"{}\"\n", account.export_directory));
        yaml.push_str("    ignored_folders:\n");
        for folder in &account.ignored_folders {
            yaml.push_str(&format!("      - \"{}\"\n", folder));
        }
        yaml.push_str(&format!("    quote_depth: {}\n", account.quote_depth));
        yaml.push_str(&format!("    skip_existing: {}\n", account.skip_existing));
        yaml.push_str(&format!("    collect_contacts: {}\n", account.collect_contacts));
        yaml.push_str(&format!("    skip_signature_images: {}\n", account.skip_signature_images));
        yaml.push_str(&format!("    delete_after_export: {}\n", account.delete_after_export));
        yaml.push('\n');
    }

    // Add .env reminder
    yaml.push_str("# Add passwords to .env file:\n");
    for account in accounts {
        let env_var = account.name.to_uppercase().replace(' ', "_");
        yaml.push_str(&format!("# {}_PASSWORD=your_password\n", env_var));
    }

    yaml
}

/// Generate .env template from extracted accounts
pub fn generate_env_template(accounts: &[Account]) -> String {
    let mut env = String::from("# Email passwords\n");
    env.push_str("# Replace 'your_password' with actual passwords\n");
    env.push_str("# For Gmail with 2FA, use App Password\n\n");

    for account in accounts {
        let env_var = account.name.to_uppercase().replace(' ', "_").replace('-', "_");
        env.push_str(&format!("{}_PASSWORD=your_password\n", env_var));
        // Also add APPLICATION_PASSWORD variant for Gmail-like accounts
        if account.server.contains("gmail") {
            env.push_str(&format!("{}_APPLICATION_PASSWORD=your_app_password\n", env_var));
        }
    }

    env
}

// ---------------------------------------------------------------------------
// Password extraction via NSS (Thunderbird logins.json)
// ---------------------------------------------------------------------------

/// Decrypted IMAP credentials from Thunderbird
pub struct ThunderbirdPassword {
    pub imap_server: String,
    pub username: String,
    pub password: String,
}

/// NSS SECItem — must match Mozilla's C layout exactly
#[repr(C)]
struct SECItem {
    item_type: u32,
    data: *mut u8,
    len: u32,
}

/// logins.json top-level structure
#[derive(serde::Deserialize)]
struct LoginsJson {
    logins: Vec<LoginEntry>,
}

/// One entry in logins.json
#[derive(serde::Deserialize)]
struct LoginEntry {
    hostname: String,
    #[serde(rename = "encryptedUsername")]
    encrypted_username: String,
    #[serde(rename = "encryptedPassword")]
    encrypted_password: String,
}

/// Find the nss3 shared library for the current platform.
pub fn find_nss_library_path(_profile: &ThunderbirdProfile) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let candidates = [
            r"C:\Program Files\Mozilla Thunderbird\nss3.dll",
            r"C:\Program Files (x86)\Mozilla Thunderbird\nss3.dll",
        ];
        for path in &candidates {
            let p = PathBuf::from(path);
            if p.exists() {
                return Some(p);
            }
        }
        None
    }

    #[cfg(target_os = "macos")]
    {
        let p = PathBuf::from(
            "/Applications/Thunderbird.app/Contents/MacOS/libnss3.dylib",
        );
        if p.exists() { Some(p) } else { None }
    }

    #[cfg(target_os = "linux")]
    {
        Some(PathBuf::from("libnss3.so"))
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// Decrypt a single base64-encoded NSS string using PK11SDR_Decrypt.
fn decrypt_nss_string(nss: &Library, encrypted_b64: &str) -> Result<String> {
    let mut encrypted_bytes = STANDARD
        .decode(encrypted_b64)
        .context("Failed to decode base64")?;

    let mut input = SECItem {
        item_type: 0,
        data: encrypted_bytes.as_mut_ptr(),
        len: encrypted_bytes.len() as u32,
    };
    let mut output = SECItem {
        item_type: 0,
        data: std::ptr::null_mut(),
        len: 0,
    };

    let status = unsafe {
        let pk11sdr_decrypt: libloading::Symbol<
            unsafe extern "C" fn(*mut SECItem, *mut SECItem, *mut c_void) -> i32,
        > = nss
            .get(b"PK11SDR_Decrypt\0")
            .context("PK11SDR_Decrypt not found in NSS library")?;
        pk11sdr_decrypt(&mut input, &mut output, std::ptr::null_mut())
    };

    if status != 0 {
        anyhow::bail!("PK11SDR_Decrypt failed (status {})", status);
    }

    // Copy decrypted bytes before freeing
    let result = if output.data.is_null() || output.len == 0 {
        String::new()
    } else {
        let bytes =
            unsafe { std::slice::from_raw_parts(output.data, output.len as usize) };
        String::from_utf8_lossy(bytes).into_owned()
    };

    // Release NSS-allocated memory (best effort)
    unsafe {
        if let Ok(secitem_free) =
            nss.get::<unsafe extern "C" fn(*mut SECItem, i32)>(b"SECITEM_FreeItem\0")
        {
            secitem_free(&mut output, 0);
        }
    }

    Ok(result)
}

/// Extract decrypted IMAP passwords from a Thunderbird profile.
///
/// `master_password` — pass `Some("...")` if the user has a Thunderbird
/// Master Password configured; `None` otherwise.
pub fn extract_passwords(
    profile: &ThunderbirdProfile,
    master_password: Option<&str>,
) -> Result<Vec<ThunderbirdPassword>> {
    let logins_path = profile.path.join("logins.json");

    if !logins_path.exists() {
        return Ok(vec![]);
    }

    let content = fs::read_to_string(&logins_path).context("Failed to read logins.json")?;
    let logins: LoginsJson =
        serde_json::from_str(&content).context("Failed to parse logins.json")?;

    let imap_entries: Vec<&LoginEntry> = logins
        .logins
        .iter()
        .filter(|e| e.hostname.starts_with("imap://"))
        .collect();

    if imap_entries.is_empty() {
        return Ok(vec![]);
    }

    // Find NSS library
    let nss_path = find_nss_library_path(profile).context(
        "NSS library (nss3) not found. Please verify that Thunderbird is installed.",
    )?;

    // On Windows, prepend Thunderbird's install directory to PATH so that
    // nss3.dll can locate its own dependencies (mozglue.dll etc.)
    #[cfg(target_os = "windows")]
    if let Some(dir) = nss_path.parent() {
        let current_path = env::var("PATH").unwrap_or_default();
        env::set_var("PATH", format!("{};{}", dir.display(), current_path));
    }

    // Load NSS
    let nss = unsafe { Library::new(&nss_path) }
        .with_context(|| format!("Failed to load NSS library: {}", nss_path.display()))?;

    // NSS_Init requires the profile path (forward slashes on all platforms)
    let profile_path_str = profile.path.to_string_lossy().replace('\\', "/");
    let profile_c =
        CString::new(profile_path_str).context("Profile path contains null bytes")?;

    let init_status = unsafe {
        let nss_init: libloading::Symbol<unsafe extern "C" fn(*const c_char) -> i32> = nss
            .get(b"NSS_Init\0")
            .context("NSS_Init not found in NSS library")?;
        nss_init(profile_c.as_ptr())
    };

    if init_status != 0 {
        anyhow::bail!(
            "NSS_Init failed (status {}). \
             Thunderbird may still be running — close it first, then retry.",
            init_status
        );
    }

    // If the profile has a Master Password, authenticate before decrypting
    if let Some(mp) = master_password {
        let mp_c = CString::new(mp).context("Master password contains null bytes")?;
        let auth_status = unsafe {
            // PK11_GetInternalKeySlot() → *mut PK11SlotInfo (opaque pointer)
            let get_slot: libloading::Symbol<unsafe extern "C" fn() -> *mut c_void> = nss
                .get(b"PK11_GetInternalKeySlot\0")
                .context("PK11_GetInternalKeySlot not found")?;
            let slot = get_slot();
            if slot.is_null() {
                anyhow::bail!("PK11_GetInternalKeySlot returned null");
            }

            let check_pw: libloading::Symbol<
                unsafe extern "C" fn(*mut c_void, *const c_char) -> i32,
            > = nss
                .get(b"PK11_CheckUserPassword\0")
                .context("PK11_CheckUserPassword not found")?;
            let status = check_pw(slot, mp_c.as_ptr());

            // Free the slot reference
            if let Ok(free_slot) =
                nss.get::<unsafe extern "C" fn(*mut c_void)>(b"PK11_FreeSlot\0")
            {
                free_slot(slot);
            }

            status
        };

        if auth_status != 0 {
            anyhow::bail!(
                "Master Password authentication failed (status {}). \
                 Check that the Master Password is correct.",
                auth_status
            );
        }
    }

    // Decrypt all IMAP entries
    let mut passwords = Vec::new();
    for entry in &imap_entries {
        // Strip "imap://" prefix and optional port
        let imap_server = entry
            .hostname
            .strip_prefix("imap://")
            .unwrap_or(&entry.hostname)
            .split(':')
            .next()
            .unwrap_or("")
            .to_string();

        let username = match decrypt_nss_string(&nss, &entry.encrypted_username) {
            Ok(u) => u,
            Err(e) => {
                eprintln!(
                    "Warning: Could not decrypt username for {}: {}",
                    imap_server, e
                );
                continue;
            }
        };

        let password = match decrypt_nss_string(&nss, &entry.encrypted_password) {
            Ok(p) => p,
            Err(e) => {
                eprintln!(
                    "Warning: Could not decrypt password for {}: {}",
                    imap_server, e
                );
                continue;
            }
        };

        passwords.push(ThunderbirdPassword {
            imap_server,
            username,
            password,
        });
    }

    // Shutdown NSS (best effort)
    let _ = unsafe {
        nss.get::<unsafe extern "C" fn() -> i32>(b"NSS_Shutdown\0")
            .map(|f| f())
    };

    Ok(passwords)
}

/// Write extracted passwords to a `.env` file.
///
/// For each account in `accounts`, looks for a matching `ThunderbirdPassword`
/// by comparing `account.server` with `password.imap_server` (case-insensitive).
/// Returns the number of passwords written.
pub fn write_passwords_to_env(
    accounts: &[crate::config::Account],
    passwords: &[ThunderbirdPassword],
    env_path: &Path,
) -> Result<usize> {
    // Read existing .env lines (if the file already exists)
    let mut lines: Vec<String> = if env_path.exists() {
        fs::read_to_string(env_path)
            .context("Failed to read .env")?
            .lines()
            .map(|l| l.to_string())
            .collect()
    } else {
        Vec::new()
    };

    let mut written = 0;

    for account in accounts {
        let matching_pw = passwords
            .iter()
            .find(|pw| pw.imap_server.eq_ignore_ascii_case(&account.server));

        let pw = match matching_pw {
            Some(p) => p,
            None => {
                eprintln!(
                    "Warning: no Thunderbird password found for account '{}' (server: {})",
                    account.name, account.server
                );
                continue;
            }
        };

        let env_key = account
            .name
            .to_uppercase()
            .replace(' ', "_")
            .replace('-', "_");
        let env_line = format!("{}={}", env_key, pw.password);
        let key_prefix = format!("{}=", env_key);

        if let Some(pos) = lines.iter().position(|l| l.starts_with(&key_prefix)) {
            lines[pos] = env_line;
        } else {
            lines.push(env_line);
        }

        written += 1;
    }

    // Write back (join with newline, ensure trailing newline)
    let mut content = lines.join("\n");
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    fs::write(env_path, content).context("Failed to write .env")?;

    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_name() {
        assert_eq!(sanitize_name("My Email Account"), "My_Email_Account");
        assert_eq!(sanitize_name("test@gmail.com"), "test_gmail_com");
    }

    #[test]
    fn test_default_ignored_folders_gmail() {
        let folders = default_ignored_folders("Gmail");
        assert!(folders.iter().any(|f| f.contains("[Gmail]")));
    }

    #[test]
    fn test_default_ignored_folders_other() {
        let folders = default_ignored_folders("MyMail");
        assert!(folders.contains(&"Spam".to_string()));
        assert!(folders.contains(&"Trash".to_string()));
    }

    #[test]
    fn test_parse_prefs_js() {
        let prefs = r#"
user_pref("mail.server.server1.type", "imap");
user_pref("mail.server.server1.hostname", "imap.gmail.com");
user_pref("mail.server.server1.port", "993");
user_pref("mail.server.server1.userName", "test@gmail.com");
user_pref("mail.server.server1.name", "Gmail");
"#;
        let accounts = parse_prefs_js(prefs).unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].server, "imap.gmail.com");
        assert_eq!(accounts[0].username, "test@gmail.com");
    }
}
