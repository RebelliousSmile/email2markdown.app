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
use std::sync::{LazyLock, Mutex};

use crate::config::Account;
use crate::email_export::{probe_account, FolderRole, ListedFolder, PROBE_TIMEOUT};

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
        env::var("HOME").ok().map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Thunderbird")
                .join("Profiles")
        })
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

/// Profile an import reads when none is given: the marked default when it has
/// a `prefs.js` (it may be an empty placeholder), else the first that has one.
pub fn default_profile() -> Result<ThunderbirdProfile> {
    let profiles = list_profiles().context("Could not find Thunderbird profiles")?;
    let has_prefs = |p: &ThunderbirdProfile| p.path.join("prefs.js").exists();
    profiles
        .iter()
        .find(|p| p.is_default && has_prefs(p))
        .or_else(|| profiles.iter().find(|p| has_prefs(p)))
        .cloned()
        .context("No usable Thunderbird profiles found (no prefs.js)")
}

/// List available Thunderbird profiles
pub fn list_profiles() -> Result<Vec<ThunderbirdProfile>> {
    let profiles_dir = get_thunderbird_profiles_dir()
        .context("Could not determine Thunderbird profiles directory")?;

    let profiles_ini = profiles_dir
        .parent()
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
    let content = fs::read_to_string(ini_path).context("Failed to read profiles.ini")?;

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

fn build_profile_from_map(
    map: &HashMap<String, String>,
    base_dir: &Path,
) -> Option<ThunderbirdProfile> {
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

    let content = fs::read_to_string(&prefs_file).context("Failed to read prefs.js")?;

    parse_prefs_js(&content)
}

static SERVER_PREF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"user_pref\("mail\.server\.([^.]+)\.([^"]+)",\s*"?([^")]+)"?\);"#)
        .expect("static regex")
});

/// Parse prefs.js and extract IMAP account configurations
fn parse_prefs_js(content: &str) -> Result<Vec<Account>> {
    Ok(parse_prefs_servers(content)
        .into_iter()
        .map(|server| server.account)
        .collect())
}

/// How Thunderbird signs in to an IMAP server (`mail.server.<id>.authMethod`).
///
/// Values from `nsMsgAuthMethod` in Thunderbird's `MailNewsTypes2.idl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    /// Login with a password (`old` 2, `passwordCleartext` 3, `passwordEncrypted` 4, or unset).
    Password,
    /// `OAuth2` (10): Thunderbird keeps a token, not a password.
    OAuth2,
    /// Any other method (GSSAPI, NTLM, client certificate…).
    Other(u32),
}

impl AuthKind {
    fn from_pref(value: Option<&String>) -> Self {
        match value.and_then(|v| v.trim().parse::<u32>().ok()) {
            None | Some(2..=4) => AuthKind::Password,
            Some(10) => AuthKind::OAuth2,
            Some(other) => AuthKind::Other(other),
        }
    }
}

/// One IMAP server of `prefs.js`.
struct PrefsServer {
    server_id: String,
    account: Account,
    auth: AuthKind,
}

/// IMAP servers of `prefs.js`, sorted by account name.
fn parse_prefs_servers(content: &str) -> Vec<PrefsServer> {
    let mut servers: HashMap<String, HashMap<String, String>> = HashMap::new();

    // Pattern: user_pref("mail.server.server1.property", "value");

    for cap in SERVER_PREF_RE.captures_iter(content) {
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

        let auth = AuthKind::from_pref(props.get("authMethod"));
        let account = Account {
            name: name.clone(),
            server: hostname,
            port,
            username,
            password: None, // Passwords are stored separately in Thunderbird
            export_directory: format!("./exports/{}", safe_name),
            included_folders: None,
            ignored_folders: default_ignored_folders(&name),
            quote_depth: 1,
            skip_existing: true,
            collect_contacts: false,
            skip_signature_images: true,
            batch_size: 0,
            delete_after_export: false,
            cleanup_empty_dirs: true,
        };
        accounts.push(PrefsServer {
            server_id,
            account,
            auth,
        });
    }

    accounts.sort_by(|a, b| {
        a.account
            .name
            .to_lowercase()
            .cmp(&b.account.name.to_lowercase())
            .then_with(|| a.server_id.cmp(&b.server_id))
    });
    accounts
}

static UNSAFE_NAME_CHARS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-zA-Z0-9_-]").expect("static regex"));

/// Sanitize account name for use as directory name
fn sanitize_name(name: &str) -> String {
    UNSAFE_NAME_CHARS_RE.replace_all(name, "_").to_string()
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
            "[Gmail]/Important".to_string(),
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

/// What [`write_accounts_merged`] did to `accounts.yaml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSummary {
    /// Imported accounts that were not in the file yet.
    pub added: usize,
    /// Accounts already in the file, left exactly as they were.
    pub kept: usize,
    /// Copy of the previous file, when there was one.
    pub backup: Option<PathBuf>,
    /// Names of the accounts counted in `added`: only these may receive a
    /// password in `.env`, an account skipped by the merge never does.
    pub added_names: Vec<String>,
}

impl ImportSummary {
    /// The accounts of `accounts` this merge added: the only ones whose
    /// password may be written to `.env`.
    pub fn added_accounts(&self, accounts: &[Account]) -> Vec<Account> {
        accounts
            .iter()
            .filter(|account| self.added_names.contains(&account.name))
            .cloned()
            .collect()
    }

    /// Report shown by the CLI and the tray after an import.
    pub fn describe(&self, path: &Path) -> String {
        let mut message = format!(
            "{} compte(s) importé(s), {} compte(s) existant(s) conservé(s) tel(s) quel(s) dans {}",
            self.added,
            self.kept,
            path.display()
        );
        if let Some(backup) = &self.backup {
            message.push_str(&format!(
                "\nÉtat précédent sauvegardé dans {}",
                backup.display()
            ));
        }
        message
    }
}

/// Add the imported `accounts` to `path` without touching the accounts it
/// already lists (matched by name), after copying it to `accounts.yaml.bak`.
pub fn write_accounts_merged(accounts: &[Account], path: &Path) -> Result<ImportSummary> {
    let mut merged = crate::config::load_raw_accounts(path)
        .with_context(|| format!("read {}", path.display()))?;
    let kept = merged.len();

    let backup = if path.exists() {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(".bak");
        let backup = path.with_file_name(name);
        fs::copy(path, &backup).with_context(|| format!("back up to {}", backup.display()))?;
        Some(backup)
    } else {
        None
    };

    let mut added_names = Vec::new();
    for account in accounts {
        if !is_configured(&merged, account) {
            added_names.push(account.name.clone());
            merged.push(crate::config::RawAccount {
                name: account.name.clone(),
                server: account.server.clone(),
                port: account.port,
                username: account.username.clone(),
                included_folders: account.included_folders.clone(),
                ignored_folders: account.ignored_folders.clone(),
            });
        }
    }
    crate::config::save_accounts(&merged, path)
        .with_context(|| format!("write {}", path.display()))?;

    Ok(ImportSummary {
        added: merged.len() - kept,
        kept,
        backup,
        added_names,
    })
}

/// Whether `account` is already in `configured`: same name, or the same
/// mailbox under another name ("Pro" vs "pro@example.com"), which importing
/// would export twice.
fn is_configured(configured: &[crate::config::RawAccount], account: &Account) -> bool {
    configured.iter().any(|raw| {
        raw.name.eq_ignore_ascii_case(&account.name)
            || (raw.server.eq_ignore_ascii_case(&account.server)
                && raw.username.eq_ignore_ascii_case(&account.username))
    })
}

/// An IMAP account of a Thunderbird profile, listed without decrypting anything.
#[derive(Debug, Clone)]
pub struct ThunderbirdAccount {
    /// Key of the account in `prefs.js` (`server2` in `mail.server.server2.*`).
    pub server_id: String,
    pub account: Account,
    pub auth: AuthKind,
    /// `logins.json` holds a password for this IMAP server.
    pub has_saved_password: bool,
    /// Already in `accounts.yaml` (same name, or same server and username).
    pub already_configured: bool,
}

/// List the IMAP accounts of `profile` for the import picker.
///
/// Reads `prefs.js`, the plaintext `hostname` of `logins.json` and
/// `accounts_path`: NSS is not loaded, so Thunderbird may stay open.
pub fn scan_accounts(
    profile: &ThunderbirdProfile,
    accounts_path: &Path,
) -> Result<Vec<ThunderbirdAccount>> {
    let prefs_file = profile.path.join("prefs.js");
    let content = fs::read_to_string(&prefs_file)
        .with_context(|| format!("read {}", prefs_file.display()))?;
    let saved_servers: Vec<String> = read_logins(profile)?
        .map(|logins| {
            logins
                .logins
                .iter()
                .filter_map(|entry| imap_login_server(&entry.hostname))
                .collect()
        })
        .unwrap_or_default();
    let configured = crate::config::load_raw_accounts(accounts_path)
        .with_context(|| format!("read {}", accounts_path.display()))?;

    Ok(parse_prefs_servers(&content)
        .into_iter()
        .map(|server| ThunderbirdAccount {
            has_saved_password: saved_servers
                .iter()
                .any(|saved| saved.eq_ignore_ascii_case(&server.account.server)),
            already_configured: is_configured(&configured, &server.account),
            server_id: server.server_id,
            account: server.account,
            auth: server.auth,
        })
        .collect())
}

/// Server of an `imap://host[:port]` login, `None` for any other login
/// (an OAuth token is stored under `oauth://accounts.google.com`).
fn imap_login_server(hostname: &str) -> Option<String> {
    let rest = hostname.strip_prefix("imap://")?;
    let server = rest.split([':', '/']).next().unwrap_or_default();
    (!server.is_empty()).then(|| server.to_string())
}

/// `logins.json` of the profile, `None` when the profile has none.
fn read_logins(profile: &ThunderbirdProfile) -> Result<Option<LoginsJson>> {
    let logins_path = profile.path.join("logins.json");
    if !logins_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&logins_path)
        .with_context(|| format!("read {}", logins_path.display()))?;
    let logins = serde_json::from_str(&content)
        .with_context(|| format!("parse {}", logins_path.display()))?;
    Ok(Some(logins))
}

/// Generate .env template from extracted accounts
pub fn generate_env_template(accounts: &[Account]) -> String {
    let mut env = String::from("# Email passwords\n");
    env.push_str("# Replace 'your_password' with actual passwords\n");
    env.push_str("# For Gmail with 2FA, use App Password\n\n");

    for account in accounts {
        let env_var = crate::config::env_var_name(&account.name);
        env.push_str(&format!("{}_PASSWORD=your_password\n", env_var));
        // Also add APPLICATION_PASSWORD variant for Gmail-like accounts
        if account.server.contains("gmail") {
            env.push_str(&format!(
                "{}_APPLICATION_PASSWORD=your_app_password\n",
                env_var
            ));
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
    #[serde(default)]
    logins: Vec<LoginEntry>,
}

/// One entry in logins.json
#[derive(serde::Deserialize)]
struct LoginEntry {
    hostname: String,
    #[serde(rename = "encryptedUsername", default)]
    encrypted_username: String,
    #[serde(rename = "encryptedPassword", default)]
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
        let p = PathBuf::from("/Applications/Thunderbird.app/Contents/MacOS/libnss3.dylib");
        if p.exists() {
            Some(p)
        } else {
            None
        }
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
        let bytes = unsafe { std::slice::from_raw_parts(output.data, output.len as usize) };
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

/// The profile is protected by a Thunderbird primary password and none was given.
///
/// Callers detect it with `err.downcast_ref::<PrimaryPasswordRequired>()` to
/// ask for the password instead of showing a failure.
#[derive(Debug, thiserror::Error)]
#[error("le profil Thunderbird est protégé par un mot de passe principal : saisissez-le")]
pub struct PrimaryPasswordRequired;

/// NSS keeps process-wide state: one profile open at a time.
static NSS_LOCK: Mutex<()> = Mutex::new(());

/// Calls `NSS_Shutdown` when dropped, on every exit path after `NSS_Init`.
struct NssSession<'a>(&'a Library);

impl Drop for NssSession<'_> {
    fn drop(&mut self) {
        // Best effort: nothing to do if shutdown fails.
        let _ = unsafe {
            self.0
                .get::<unsafe extern "C" fn() -> i32>(b"NSS_Shutdown\0")
                .map(|f| f())
        };
    }
}

const WRONG_PRIMARY_PASSWORD: &str = "mot de passe principal Thunderbird incorrect";

/// Unlock the internal key slot: [`PrimaryPasswordRequired`] when it needs a
/// password and none was given, an error when the given one is refused.
fn unlock_key_slot(nss: &Library, primary_password: Option<&str>) -> Result<()> {
    unsafe {
        // PK11_GetInternalKeySlot() → *mut PK11SlotInfo (opaque pointer)
        let get_slot: libloading::Symbol<unsafe extern "C" fn() -> *mut c_void> = nss
            .get(b"PK11_GetInternalKeySlot\0")
            .context("PK11_GetInternalKeySlot not found")?;
        let need_login: libloading::Symbol<unsafe extern "C" fn(*mut c_void) -> i32> = nss
            .get(b"PK11_NeedLogin\0")
            .context("PK11_NeedLogin not found")?;
        let check_pw: libloading::Symbol<unsafe extern "C" fn(*mut c_void, *const c_char) -> i32> =
            nss.get(b"PK11_CheckUserPassword\0")
                .context("PK11_CheckUserPassword not found")?;
        let free_slot: libloading::Symbol<unsafe extern "C" fn(*mut c_void)> = nss
            .get(b"PK11_FreeSlot\0")
            .context("PK11_FreeSlot not found")?;

        let slot = get_slot();
        if slot.is_null() {
            anyhow::bail!("PK11_GetInternalKeySlot returned null");
        }
        let result = if need_login(slot) == 0 {
            Ok(())
        } else {
            match primary_password.map(CString::new) {
                None => Err(PrimaryPasswordRequired.into()),
                Some(Ok(pw_c)) if check_pw(slot, pw_c.as_ptr()) == 0 => Ok(()),
                Some(_) => Err(anyhow::anyhow!(WRONG_PRIMARY_PASSWORD)),
            }
        };
        free_slot(slot);
        result
    }
}

/// Extract decrypted IMAP passwords from a Thunderbird profile.
///
/// `primary_password` — the Thunderbird primary password, when the profile
/// has one. Without it, such a profile gives [`PrimaryPasswordRequired`].
pub fn extract_passwords(
    profile: &ThunderbirdProfile,
    primary_password: Option<&str>,
) -> Result<Vec<ThunderbirdPassword>> {
    let Some(logins) = read_logins(profile)? else {
        return Ok(vec![]);
    };

    let imap_entries: Vec<(String, &LoginEntry)> = logins
        .logins
        .iter()
        .filter_map(|e| imap_login_server(&e.hostname).map(|server| (server, e)))
        .collect();

    if imap_entries.is_empty() {
        return Ok(vec![]);
    }

    // Find NSS library
    let nss_path = find_nss_library_path(profile)
        .context("NSS library (nss3) not found. Please verify that Thunderbird is installed.")?;

    // On Windows, Thunderbird's install directory must be in PATH so that
    // nss3.dll can locate its own dependencies (mozglue.dll etc.). Added once:
    // the tray may import many times in the same process.
    #[cfg(target_os = "windows")]
    if let Some(dir) = nss_path.parent() {
        let current_path = env::var_os("PATH").unwrap_or_default();
        if !env::split_paths(&current_path).any(|p| p == dir) {
            let mut paths = vec![dir.to_path_buf()];
            paths.extend(env::split_paths(&current_path));
            if let Ok(joined) = env::join_paths(paths) {
                env::set_var("PATH", joined);
            }
        }
    }

    let _guard = NSS_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // Load NSS
    let nss = unsafe { Library::new(&nss_path) }
        .with_context(|| format!("Failed to load NSS library: {}", nss_path.display()))?;

    // NSS_Init requires the profile path (forward slashes on all platforms)
    let profile_path_str = profile.path.to_string_lossy().replace('\\', "/");
    let profile_c = CString::new(profile_path_str).context("Profile path contains null bytes")?;

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
    let _session = NssSession(&nss);

    unlock_key_slot(&nss, primary_password)?;

    // Decrypt all IMAP entries
    let mut passwords = Vec::new();
    for (imap_server, entry) in imap_entries {
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

    Ok(passwords)
}

/// The password of `username` on `server`.
///
/// Server and username must both match; the server alone is enough only
/// when it has a single saved password (Thunderbird may store `moi` for the
/// account `moi@example.com`). Two accounts on `imap.gmail.com` therefore
/// never share a password.
pub fn find_password<'a>(
    passwords: &'a [ThunderbirdPassword],
    server: &str,
    username: &str,
) -> Option<&'a ThunderbirdPassword> {
    find_exact_password(passwords, server, username).or_else(|| {
        let mut on_server = passwords
            .iter()
            .filter(|pw| pw.imap_server.eq_ignore_ascii_case(server));
        match (on_server.next(), on_server.next()) {
            (Some(only), None) => Some(only),
            _ => None,
        }
    })
}

fn find_exact_password<'a>(
    passwords: &'a [ThunderbirdPassword],
    server: &str,
    username: &str,
) -> Option<&'a ThunderbirdPassword> {
    passwords.iter().find(|pw| {
        pw.imap_server.eq_ignore_ascii_case(server) && pw.username.eq_ignore_ascii_case(username)
    })
}

/// [`find_password`], restricted to an exact username match when another
/// account of `accounts` uses the same server: the single saved password of
/// that server may belong to the other account.
fn password_for<'a>(
    accounts: &[&Account],
    passwords: &'a [ThunderbirdPassword],
    account: &Account,
) -> Option<&'a ThunderbirdPassword> {
    let shared_server = accounts
        .iter()
        .filter(|other| other.server.eq_ignore_ascii_case(&account.server))
        .count()
        > 1;
    if shared_server {
        find_exact_password(passwords, &account.server, &account.username)
    } else {
        find_password(passwords, &account.server, &account.username)
    }
}

/// Write extracted passwords to a `.env` file.
///
/// Pass only the accounts the import added (`ImportSummary::added_names`):
/// an account skipped by the merge may share its name with an existing one,
/// whose password must stay. Returns the number of passwords written.
pub fn write_passwords_to_env(
    accounts: &[crate::config::Account],
    passwords: &[ThunderbirdPassword],
    env_path: &Path,
) -> Result<usize> {
    let all: Vec<&Account> = accounts.iter().collect();
    let mut entries = Vec::new();
    for account in accounts {
        match password_for(&all, passwords, account) {
            Some(pw) => entries.push((account.name.as_str(), pw.password.as_str())),
            None => eprintln!(
                "Warning: no Thunderbird password found for account '{}' (server: {})",
                account.name, account.server
            ),
        }
    }
    write_env_passwords(&entries, env_path)
}

/// Set `{PREFIX}_PASSWORD` for each `(account name, password)` in `env_path`.
fn write_env_passwords(entries: &[(&str, &str)], env_path: &Path) -> Result<usize> {
    if entries.is_empty() {
        return Ok(0);
    }
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

    for (name, password) in entries {
        if !crate::config::env_value_is_valid(password) {
            eprintln!(
                "Warning: Thunderbird password for account '{}' contains a line break; skipped",
                name
            );
            continue;
        }
        let env_key = format!("{}_PASSWORD", crate::config::env_var_name(name));
        let escaped = password.replace('\'', "'\\''");
        let env_line = format!("{}='{}'", env_key, escaped);
        let key_prefix = format!("{}=", env_key);

        if let Some(pos) = lines.iter().position(|l| l.starts_with(&key_prefix)) {
            lines[pos] = env_line;
        } else {
            lines.push(env_line);
        }

        written += 1;
    }

    crate::config::write_env_file(env_path, &lines).context("Failed to write .env")?;

    Ok(written)
}

/// Folders exported by an imported account whose connection test succeeded:
/// the inbox and the sent folder.
pub fn default_included_folders(folders: &[ListedFolder]) -> Vec<String> {
    let mut included: Vec<String> = folders
        .iter()
        .filter(|f| matches!(f.role, FolderRole::Inbox | FolderRole::Sent))
        .map(|f| f.name.display.clone())
        .collect();
    if !folders.iter().any(|f| f.role == FolderRole::Inbox) {
        included.insert(0, "INBOX".to_string());
    }
    included
}

/// Outcome of one account of [`import_selected`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ImportReport {
    pub name: String,
    /// Added, and its connection test succeeded.
    pub ok: bool,
    /// Folders exported on success, cause of the failure otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Import the accounts `server_ids` of `profile`, each checked by a
/// connection test.
///
/// An account already in `accounts_path` (read again here) is never imported.
/// A tested account exports its inbox and sent folder, any other only its
/// inbox. Only the accounts actually added get their password in `env_path`.
/// [`PrimaryPasswordRequired`], a failed `NSS_Init` (Thunderbird open) or an
/// unreadable profile stop the import before anything is written.
pub fn import_selected(
    profile: &ThunderbirdProfile,
    server_ids: &[String],
    primary_password: Option<&str>,
    accounts_path: &Path,
    env_path: &Path,
) -> Result<Vec<ImportReport>> {
    import_selected_with(
        profile,
        server_ids,
        accounts_path,
        env_path,
        || extract_passwords(profile, primary_password),
        |account, password| {
            probe_account(
                &account.server,
                account.port,
                &account.username,
                password,
                PROBE_TIMEOUT,
            )
        },
    )
}

/// Outcome of the connection test of one account: its folders, or the cause
/// of the failure in French.
type ProbeOutcome = std::result::Result<Vec<ListedFolder>, String>;

/// [`import_selected`] with the password source and the connection test
/// injected, so that the import logic is tested without NSS or a server.
fn import_selected_with(
    profile: &ThunderbirdProfile,
    server_ids: &[String],
    accounts_path: &Path,
    env_path: &Path,
    passwords: impl FnOnce() -> Result<Vec<ThunderbirdPassword>>,
    probe: impl Fn(&Account, &str) -> Result<Vec<ListedFolder>> + Sync,
) -> Result<Vec<ImportReport>> {
    let scanned = scan_accounts(profile, accounts_path)?;
    let mut reports = Vec::new();
    let mut candidates = Vec::new();
    for id in server_ids {
        match scanned.iter().find(|a| &a.server_id == id) {
            None => reports.push(failure(id, "compte introuvable dans le profil Thunderbird")),
            Some(found) if found.already_configured => {
                reports.push(failure(&found.account.name, "déjà configuré"))
            }
            Some(found) => candidates.push(found),
        }
    }
    if candidates.is_empty() {
        return Ok(reports);
    }

    let passwords = if candidates.iter().any(|a| a.has_saved_password) {
        passwords()?
    } else {
        Vec::new()
    };
    let profile_accounts: Vec<&Account> = scanned.iter().map(|a| &a.account).collect();
    let matched: Vec<Option<&ThunderbirdPassword>> = candidates
        .iter()
        .map(|candidate| {
            if candidate.auth == AuthKind::OAuth2 {
                // Thunderbird holds a token, not a password: only an exact
                // match can be theirs.
                find_exact_password(
                    &passwords,
                    &candidate.account.server,
                    &candidate.account.username,
                )
            } else {
                password_for(&profile_accounts, &passwords, &candidate.account)
            }
        })
        .collect();

    // One thread per account: the whole import waits one timeout at most.
    let outcomes: Vec<ProbeOutcome> = std::thread::scope(|scope| {
        let probe = &probe;
        let handles: Vec<_> = candidates
            .iter()
            .zip(&matched)
            .map(|(candidate, password)| {
                scope.spawn(move || match password {
                    Some(pw) => probe(&candidate.account, &pw.password).map_err(|err| {
                        crate::email_export::describe_connection_error(
                            &candidate.account.server,
                            &err,
                        )
                    }),
                    None => Err(missing_password_message(candidate)),
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err("le test de connexion a échoué".to_string()))
            })
            .collect()
    });

    let to_add: Vec<Account> = candidates
        .iter()
        .zip(&outcomes)
        .map(|(candidate, outcome)| {
            let mut account = candidate.account.clone();
            account.included_folders = Some(match outcome {
                Ok(folders) => default_included_folders(folders),
                Err(_) => vec!["INBOX".to_string()],
            });
            account
        })
        .collect();
    let summary = write_accounts_merged(&to_add, accounts_path)?;

    let mut entries = Vec::new();
    for ((account, password), outcome) in to_add.iter().zip(&matched).zip(&outcomes) {
        if !summary.added_names.contains(&account.name) {
            // Two selected accounts with the same name: the first one won.
            reports.push(failure(&account.name, "déjà configuré"));
            continue;
        }
        if let Some(pw) = password {
            entries.push((account.name.as_str(), pw.password.as_str()));
        }
        reports.push(match outcome {
            Ok(_) => ImportReport {
                name: account.name.clone(),
                ok: true,
                message: account
                    .included_folders
                    .as_ref()
                    .map(|folders| format!("dossiers exportés : {}", folders.join(", "))),
            },
            Err(cause) => failure(&account.name, cause),
        });
    }
    write_env_passwords(&entries, env_path)?;

    Ok(reports)
}

fn failure(name: &str, cause: &str) -> ImportReport {
    ImportReport {
        name: name.to_string(),
        ok: false,
        message: Some(cause.to_string()),
    }
}

/// Why `candidate` cannot be tested: Thunderbird holds no password for it.
fn missing_password_message(candidate: &ThunderbirdAccount) -> String {
    let gmail = candidate.account.server.to_lowercase().contains("gmail");
    match (candidate.auth, gmail) {
        (AuthKind::OAuth2, true) => "compte Gmail connecté par OAuth dans Thunderbird : créez un \
             mot de passe d'application sur https://myaccount.google.com/apppasswords puis \
             saisissez-le dans le formulaire du compte"
            .to_string(),
        (AuthKind::OAuth2, false) => "compte connecté par OAuth dans Thunderbird, sans mot de \
             passe : saisissez un mot de passe (d'application si le fournisseur l'exige) dans le \
             formulaire du compte"
            .to_string(),
        _ => "mot de passe absent de Thunderbird : saisissez-le dans le formulaire du compte"
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imported(name: &str, server: &str) -> Account {
        Account {
            name: name.to_string(),
            server: server.to_string(),
            port: 993,
            username: format!("{name}@example.com"),
            password: None,
            included_folders: None,
            ignored_folders: vec!["Spam".to_string()],
            export_directory: String::new(),
            quote_depth: 1,
            skip_existing: true,
            collect_contacts: false,
            skip_signature_images: false,
            delete_after_export: false,
            cleanup_empty_dirs: true,
            batch_size: 0,
        }
    }

    #[test]
    fn test_write_accounts_merged_keeps_existing_account() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("accounts.yaml");
        let before = "accounts:\n- name: Perso\n  server: imap.custom.example\n  port: 143\n  username: moi\n  ignored_folders:\n  - Archives\n";
        fs::write(&path, before).unwrap();

        let summary = write_accounts_merged(
            &[
                imported("Perso", "imap.thunderbird.example"),
                imported("Pro", "imap.pro.example"),
            ],
            &path,
        )
        .unwrap();

        let backup = temp.path().join("accounts.yaml.bak");
        assert_eq!(
            summary,
            ImportSummary {
                added: 1,
                kept: 1,
                backup: Some(backup.clone()),
                added_names: vec!["Pro".to_string()],
            }
        );
        assert_eq!(fs::read_to_string(&backup).unwrap(), before);
        let accounts = crate::config::load_raw_accounts(&path).unwrap();
        let names: Vec<&str> = accounts.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Perso", "Pro"]);
        assert_eq!(accounts[0].server, "imap.custom.example");
        assert_eq!(accounts[0].port, 143);
        assert_eq!(accounts[0].ignored_folders, vec!["Archives".to_string()]);
    }

    #[test]
    fn test_parse_profiles_ini_with_several_profiles() {
        let temp = tempfile::TempDir::new().unwrap();
        let ini = temp.path().join("profiles.ini");
        let absolute = temp.path().join("ailleurs");
        fs::write(
            &ini,
            format!(
                "[General]\nStartWithLastProfile=1\n\n\
                 [Profile0]\nName=default\nIsRelative=1\nPath=Profiles/abc.default\n\n\
                 [Install123]\nDefault=Profiles/abc.default\n\n\
                 [Profile1]\nName=travail\nIsRelative=0\nPath={}\nDefault=1\n",
                absolute.display()
            ),
        )
        .unwrap();

        let profiles = parse_profiles_ini(&ini, temp.path()).unwrap();

        let summary: Vec<(&str, PathBuf, bool)> = profiles
            .iter()
            .map(|p| (p.name.as_str(), p.path.clone(), p.is_default))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("default", temp.path().join("Profiles/abc.default"), false),
                ("travail", absolute, true),
            ]
        );
    }

    #[test]
    fn test_write_accounts_merged_with_same_mailbox_under_another_name() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("accounts.yaml");
        let before = "accounts:\n- name: Pro\n  server: IMAP.example\n  port: 993\n  username: Pro@example.com\n";
        fs::write(&path, before).unwrap();
        let mut same_mailbox = imported("pro@example.com", "imap.example");
        same_mailbox.username = "pro@example.com".to_string();

        let summary = write_accounts_merged(&[same_mailbox], &path).unwrap();

        assert_eq!(summary.added, 0);
    }

    #[test]
    fn test_write_accounts_merged_without_file_writes_no_backup() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("config").join("accounts.yaml");

        let summary = write_accounts_merged(&[imported("Perso", "imap.example")], &path).unwrap();

        assert_eq!(
            summary,
            ImportSummary {
                added: 1,
                kept: 0,
                backup: None,
                added_names: vec!["Perso".to_string()],
            }
        );
        assert_eq!(crate::config::load_raw_accounts(&path).unwrap().len(), 1);
    }

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

    const FAKE_PREFS: &str = r#"
user_pref("mail.server.server1.type", "imap");
user_pref("mail.server.server1.hostname", "imap.free.fr");
user_pref("mail.server.server1.userName", "moi@free.fr");
user_pref("mail.server.server1.name", "Perso");
user_pref("mail.server.server1.authMethod", 3);
user_pref("mail.server.server2.type", "imap");
user_pref("mail.server.server2.hostname", "imap.gmail.com");
user_pref("mail.server.server2.userName", "moi@gmail.com");
user_pref("mail.server.server2.name", "Gmail");
user_pref("mail.server.server2.authMethod", 10);
user_pref("mail.server.server3.type", "imap");
user_pref("mail.server.server3.hostname", "mail.pro.fr");
user_pref("mail.server.server3.userName", "moi@pro.fr");
user_pref("mail.server.server3.name", "Pro");
user_pref("mail.server.server4.type", "none");
user_pref("mail.server.server4.hostname", "Local Folders");
"#;

    const FAKE_LOGINS: &str = r#"{"logins":[
  {"hostname":"imap://imap.free.fr","encryptedUsername":"u","encryptedPassword":"p"},
  {"hostname":"oauth://accounts.google.com","encryptedUsername":"u","encryptedPassword":"p"}
]}"#;

    /// Profile of `FAKE_PREFS` and `logins`, with `Pro` already in accounts.yaml.
    fn fake_profile(
        temp: &tempfile::TempDir,
        logins: Option<&str>,
    ) -> (ThunderbirdProfile, PathBuf) {
        let profile_dir = temp.path().join("profile");
        fs::create_dir_all(&profile_dir).unwrap();
        fs::write(profile_dir.join("prefs.js"), FAKE_PREFS).unwrap();
        if let Some(logins) = logins {
            fs::write(profile_dir.join("logins.json"), logins).unwrap();
        }
        let accounts_path = temp.path().join("accounts.yaml");
        fs::write(
            &accounts_path,
            "accounts:\n- name: Bureau\n  server: MAIL.pro.fr\n  port: 993\n  username: moi@pro.fr\n",
        )
        .unwrap();
        let profile = ThunderbirdProfile {
            name: "default".to_string(),
            path: profile_dir,
            is_default: true,
        };
        (profile, accounts_path)
    }

    fn tb_password(server: &str, username: &str, password: &str) -> ThunderbirdPassword {
        ThunderbirdPassword {
            imap_server: server.to_string(),
            username: username.to_string(),
            password: password.to_string(),
        }
    }

    fn folder(name: &str, role: FolderRole) -> ListedFolder {
        ListedFolder {
            name: crate::email_export::FolderName {
                raw: name.to_string(),
                display: name.to_string(),
            },
            role,
            depth: 0,
        }
    }

    #[test]
    fn test_scan_accounts_lists_imap_servers() {
        let temp = tempfile::TempDir::new().unwrap();
        let (profile, accounts_path) = fake_profile(&temp, Some(FAKE_LOGINS));

        let scanned = scan_accounts(&profile, &accounts_path).unwrap();

        let ids: Vec<(&str, &str)> = scanned
            .iter()
            .map(|a| (a.server_id.as_str(), a.account.name.as_str()))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("server2", "Gmail"),
                ("server1", "Perso"),
                ("server3", "Pro")
            ]
        );
    }

    #[test]
    fn test_scan_accounts_with_oauth2() {
        let temp = tempfile::TempDir::new().unwrap();
        let (profile, accounts_path) = fake_profile(&temp, Some(FAKE_LOGINS));

        let scanned = scan_accounts(&profile, &accounts_path).unwrap();

        let auths: Vec<AuthKind> = scanned.iter().map(|a| a.auth).collect();
        assert_eq!(
            auths,
            vec![AuthKind::OAuth2, AuthKind::Password, AuthKind::Password]
        );
    }

    #[test]
    fn test_scan_accounts_has_saved_password_ignores_oauth_token() {
        let temp = tempfile::TempDir::new().unwrap();
        let (profile, accounts_path) = fake_profile(&temp, Some(FAKE_LOGINS));

        let scanned = scan_accounts(&profile, &accounts_path).unwrap();

        let saved: Vec<bool> = scanned.iter().map(|a| a.has_saved_password).collect();
        assert_eq!(saved, vec![false, true, false]);
    }

    #[test]
    fn test_scan_accounts_already_configured_by_mailbox() {
        let temp = tempfile::TempDir::new().unwrap();
        let (profile, accounts_path) = fake_profile(&temp, Some(FAKE_LOGINS));

        let scanned = scan_accounts(&profile, &accounts_path).unwrap();

        let configured: Vec<bool> = scanned.iter().map(|a| a.already_configured).collect();
        assert_eq!(configured, vec![false, false, true]);
    }

    #[test]
    fn test_scan_accounts_without_logins_json() {
        let temp = tempfile::TempDir::new().unwrap();
        let (profile, accounts_path) = fake_profile(&temp, None);

        let scanned = scan_accounts(&profile, &accounts_path).unwrap();

        assert_eq!(scanned.len(), 3);
        assert!(scanned.iter().all(|a| !a.has_saved_password));
    }

    #[test]
    fn test_auth_kind_from_pref() {
        let kinds: Vec<AuthKind> = [None, Some("3"), Some("4"), Some("10"), Some("6")]
            .iter()
            .map(|v| AuthKind::from_pref(v.map(str::to_string).as_ref()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                AuthKind::Password,
                AuthKind::Password,
                AuthKind::Password,
                AuthKind::OAuth2,
                AuthKind::Other(6)
            ]
        );
    }

    #[test]
    fn test_imap_login_server_strips_scheme_and_port() {
        let servers: Vec<Option<String>> = [
            "imap://imap.free.fr:993",
            "imap://IMAP.gmail.com",
            "oauth://accounts.google.com",
            "smtp://smtp.free.fr",
        ]
        .iter()
        .map(|h| imap_login_server(h))
        .collect();
        assert_eq!(
            servers,
            vec![
                Some("imap.free.fr".to_string()),
                Some("IMAP.gmail.com".to_string()),
                None,
                None
            ]
        );
    }

    #[test]
    fn test_default_included_folders_inbox_and_sent() {
        let folders = [
            folder("INBOX", FolderRole::Inbox),
            folder("Sent", FolderRole::Sent),
            folder("Trash", FolderRole::Special),
            folder("Projets", FolderRole::Regular),
        ];

        assert_eq!(default_included_folders(&folders), vec!["INBOX", "Sent"]);
    }

    #[test]
    fn test_find_password_with_two_accounts_on_same_server() {
        let passwords = [
            tb_password("imap.gmail.com", "a@gmail.com", "pw-a"),
            tb_password("imap.gmail.com", "b@gmail.com", "pw-b"),
        ];

        let found: Vec<Option<&str>> = ["A@gmail.com", "b@gmail.com", "c@gmail.com"]
            .iter()
            .map(|u| find_password(&passwords, "IMAP.gmail.com", u).map(|pw| pw.password.as_str()))
            .collect();
        assert_eq!(found, vec![Some("pw-a"), Some("pw-b"), None]);
    }

    #[test]
    fn test_find_password_with_single_entry_on_server() {
        let passwords = [tb_password("imap.free.fr", "moi", "pw")];

        let found = find_password(&passwords, "imap.free.fr", "moi@free.fr");

        assert_eq!(found.map(|pw| pw.password.as_str()), Some("pw"));
    }

    #[test]
    fn test_write_passwords_to_env_with_two_gmail_accounts() {
        let temp = tempfile::TempDir::new().unwrap();
        let env_path = temp.path().join(".env");
        let mut a = imported("GmailA", "imap.gmail.com");
        a.username = "a@gmail.com".to_string();
        let mut b = imported("GmailB", "imap.gmail.com");
        b.username = "b@gmail.com".to_string();
        let passwords = [
            tb_password("imap.gmail.com", "b@gmail.com", "pw-b"),
            tb_password("imap.gmail.com", "a@gmail.com", "pw-a"),
        ];

        write_passwords_to_env(&[a, b], &passwords, &env_path).unwrap();

        assert_eq!(
            fs::read_to_string(&env_path).unwrap().trim_end(),
            "GMAILA_PASSWORD='pw-a'\nGMAILB_PASSWORD='pw-b'"
        );
    }

    #[test]
    fn test_write_passwords_to_env_without_fallback_on_shared_server() {
        let temp = tempfile::TempDir::new().unwrap();
        let env_path = temp.path().join(".env");
        let mut a = imported("GmailA", "imap.gmail.com");
        a.username = "a@gmail.com".to_string();
        let mut b = imported("GmailB", "imap.gmail.com");
        b.username = "b@gmail.com".to_string();
        let passwords = [tb_password("imap.gmail.com", "b@gmail.com", "pw-b")];

        write_passwords_to_env(&[a, b], &passwords, &env_path).unwrap();

        assert_eq!(
            fs::read_to_string(&env_path).unwrap().trim_end(),
            "GMAILB_PASSWORD='pw-b'"
        );
    }

    #[test]
    fn test_write_accounts_merged_added_names_exclude_skipped() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("accounts.yaml");
        fs::write(
            &path,
            "accounts:\n- name: Perso\n  server: imap.ancien.example\n  port: 993\n  username: moi\n",
        )
        .unwrap();
        let accounts = [
            imported("Perso", "imap.free.fr"),
            imported("Pro", "imap.pro.example"),
        ];

        let summary = write_accounts_merged(&accounts, &path).unwrap();

        let added: Vec<String> = summary
            .added_accounts(&accounts)
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(added, vec!["Pro"]);
    }

    #[test]
    fn test_import_selected_with_name_collision_keeps_env() {
        let temp = tempfile::TempDir::new().unwrap();
        let (profile, accounts_path) = fake_profile(&temp, Some(FAKE_LOGINS));
        fs::write(
            &accounts_path,
            "accounts:\n- name: Perso\n  server: imap.ancien.example\n  port: 993\n  username: moi\n",
        )
        .unwrap();
        let env_path = temp.path().join(".env");
        fs::write(&env_path, "PERSO_PASSWORD='ancien'\n").unwrap();

        let reports = import_selected_with(
            &profile,
            &["server1".to_string()],
            &accounts_path,
            &env_path,
            || Ok(vec![tb_password("imap.free.fr", "moi@free.fr", "nouveau")]),
            |_, _| Ok(vec![folder("INBOX", FolderRole::Inbox)]),
        )
        .unwrap();

        assert_eq!(reports, vec![failure("Perso", "déjà configuré")]);
        assert_eq!(
            fs::read_to_string(&env_path).unwrap(),
            "PERSO_PASSWORD='ancien'\n"
        );
    }

    #[test]
    fn test_import_selected_with_tested_accounts() {
        let temp = tempfile::TempDir::new().unwrap();
        let (profile, accounts_path) = fake_profile(&temp, Some(FAKE_LOGINS));
        let env_path = temp.path().join(".env");

        let reports = import_selected_with(
            &profile,
            &["server1".to_string(), "server2".to_string()],
            &accounts_path,
            &env_path,
            || Ok(vec![tb_password("imap.free.fr", "moi@free.fr", "secret")]),
            |account, password| {
                assert_eq!((account.name.as_str(), password), ("Perso", "secret"));
                Ok(vec![
                    folder("INBOX", FolderRole::Inbox),
                    folder("Envoyés", FolderRole::Sent),
                    folder("Corbeille", FolderRole::Special),
                ])
            },
        )
        .unwrap();

        let outcome: Vec<(&str, bool)> = reports.iter().map(|r| (r.name.as_str(), r.ok)).collect();
        assert_eq!(outcome, vec![("Perso", true), ("Gmail", false)]);
        let saved = crate::config::load_raw_accounts(&accounts_path).unwrap();
        let folders: Vec<(&str, Option<Vec<String>>)> = saved
            .iter()
            .map(|a| (a.name.as_str(), a.included_folders.clone()))
            .collect();
        assert_eq!(
            folders,
            vec![
                ("Bureau", None),
                (
                    "Perso",
                    Some(vec!["INBOX".to_string(), "Envoyés".to_string()])
                ),
                ("Gmail", Some(vec!["INBOX".to_string()])),
            ]
        );
        assert_eq!(
            fs::read_to_string(&env_path).unwrap().trim_end(),
            "PERSO_PASSWORD='secret'"
        );
    }

    #[test]
    fn test_import_selected_with_oauth_gmail_explains_app_password() {
        let temp = tempfile::TempDir::new().unwrap();
        let (profile, accounts_path) = fake_profile(&temp, Some(FAKE_LOGINS));
        let env_path = temp.path().join(".env");

        let reports = import_selected_with(
            &profile,
            &["server2".to_string()],
            &accounts_path,
            &env_path,
            || panic!("no saved password to decrypt"),
            |_, _| panic!("an OAuth account without password is not tested"),
        )
        .unwrap();

        let message = reports[0].message.clone().unwrap_or_default();
        assert!(message.contains("OAuth") && message.contains("apppasswords"));
        assert!(!env_path.exists());
    }

    #[test]
    fn test_import_selected_with_primary_password_required_writes_nothing() {
        let temp = tempfile::TempDir::new().unwrap();
        let (profile, accounts_path) = fake_profile(&temp, Some(FAKE_LOGINS));
        let before = fs::read_to_string(&accounts_path).unwrap();
        let env_path = temp.path().join(".env");

        let err = import_selected_with(
            &profile,
            &["server1".to_string()],
            &accounts_path,
            &env_path,
            || Err(PrimaryPasswordRequired.into()),
            |_, _| panic!("no test before the passwords are read"),
        )
        .unwrap_err();

        assert!(err.downcast_ref::<PrimaryPasswordRequired>().is_some());
        assert_eq!(fs::read_to_string(&accounts_path).unwrap(), before);
        assert!(!env_path.exists());
    }

    #[test]
    fn test_import_selected_with_failed_probe_keeps_inbox_and_cause() {
        let temp = tempfile::TempDir::new().unwrap();
        let (profile, accounts_path) = fake_profile(&temp, Some(FAKE_LOGINS));
        let env_path = temp.path().join(".env");

        let reports = import_selected_with(
            &profile,
            &["server1".to_string()],
            &accounts_path,
            &env_path,
            || Ok(vec![tb_password("imap.free.fr", "moi@free.fr", "périmé")]),
            |_, _| {
                Err(anyhow::anyhow!(
                    "NO [AUTHENTICATIONFAILED] Invalid credentials"
                ))
            },
        )
        .unwrap();

        assert!(!reports[0].ok);
        let saved = crate::config::load_raw_accounts(&accounts_path).unwrap();
        assert_eq!(saved[1].included_folders, Some(vec!["INBOX".to_string()]));
    }
}
