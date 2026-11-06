use std::io::{Read, Write};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const GITHUB_API_LATEST: &str =
    "https://api.github.com/repos/RebelliousSmile/email2markdown.app/releases/latest";

const USER_AGENT: &str = "email-to-markdown-updater";

/// Release asset listing the SHA-256 of every published binary (`sha256sum` format).
const CHECKSUMS_ASSET: &str = "SHA256SUMS";

/// Whole-request bound for the small calls (release metadata, checksums).
const METADATA_TIMEOUT: Duration = Duration::from_secs(15);
/// Binary download: bound the connection and each read rather than the whole
/// transfer, so a silent server fails fast while a slow link still completes.
const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const DOWNLOAD_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Largest asset accepted: the release binary weighs about 10 MB, so a larger
/// download is not ours and must not fill the disk.
const MAX_ASSET_BYTES: u64 = 100 * 1024 * 1024;

static SEMVER_V_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^v").expect("static regex"));

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub tag_name: String,
    pub body: String,
    /// File name of the platform binary, as listed in `SHA256SUMS`.
    pub asset_name: String,
    pub asset_url: String,
    pub checksums_url: String,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    body: Option<String>,
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

pub fn check_update(current: &str) -> Result<Option<Release>> {
    // Bound the call: ureq has no default timeout, so an unreachable host would
    // otherwise hang the update window's spinner indefinitely.
    let response: GhRelease = ureq::get(GITHUB_API_LATEST)
        .set("User-Agent", USER_AGENT)
        .timeout(METADATA_TIMEOUT)
        .call()
        .map_err(|e| anyhow!("GitHub API error: {}", e))?
        .into_json()?;

    let Some(release) = select_release(response, current)? else {
        return Ok(None);
    };
    // Never offer an update that could not be verified afterwards.
    fetch_expected_checksum(&release)?;
    Ok(Some(release))
}

/// Pick the update to offer from a decoded release: `None` when `current` is
/// already up to date, an error when the release lacks the platform binary or
/// its checksums.
fn select_release(response: GhRelease, current: &str) -> Result<Option<Release>> {
    let remote_tag = SEMVER_V_PREFIX.replace(&response.tag_name, "").to_string();

    let remote_ver = semver::Version::parse(&remote_tag)
        .map_err(|e| anyhow!("Invalid remote version '{}': {}", remote_tag, e))?;
    let current_ver = semver::Version::parse(current)
        .map_err(|e| anyhow!("Invalid current version '{}': {}", current, e))?;
    if remote_ver <= current_ver {
        return Ok(None);
    }

    let checksums_url = response
        .assets
        .iter()
        .find(|asset| asset.name == CHECKSUMS_ASSET)
        .map(|asset| asset.browser_download_url.clone())
        .ok_or_else(|| {
            anyhow!(
                "No {} asset found in release {}",
                CHECKSUMS_ASSET,
                response.tag_name
            )
        })?;
    let asset = response
        .assets
        .into_iter()
        .find(|asset| release_asset_matches(&asset.name))
        .ok_or_else(|| anyhow!("No compatible asset found in release {}", response.tag_name))?;

    Ok(Some(Release {
        tag_name: remote_tag,
        body: response.body.unwrap_or_default(),
        asset_name: asset.name,
        asset_url: asset.browser_download_url,
        checksums_url,
    }))
}

fn release_asset_matches(name: &str) -> bool {
    #[cfg(target_os = "windows")]
    return name.ends_with("windows-x86_64.exe");
    #[cfg(target_os = "macos")]
    return name.ends_with("macos-native");
    #[cfg(target_os = "linux")]
    return name.ends_with("linux-x86_64");
    #[allow(unreachable_code)]
    false
}

/// Extract the SHA-256 of `file_name` from a `sha256sum`-formatted listing
/// (`<64 hex>  <name>`, or `<64 hex> *<name>` in binary mode).
fn checksum_for(sums: &str, file_name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (sum, name) = line.trim().split_once(char::is_whitespace)?;
        let name = name.trim_start().trim_start_matches('*');
        let is_sha256 = sum.len() == 64 && sum.bytes().all(|b| b.is_ascii_hexdigit());
        (is_sha256 && name == file_name).then(|| sum.to_ascii_lowercase())
    })
}

/// Compare the SHA-256 of the file at `path` with `expected` (case-insensitive).
fn verify_checksum(path: &Path, expected: &str) -> Result<()> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("failed to open {} for verification", path.display()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)
        .with_context(|| format!("failed to read {} for verification", path.display()))?;
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected.trim()) {
        return Err(anyhow!(
            "checksum mismatch: expected {}, got {}",
            expected.trim().to_ascii_lowercase(),
            actual
        ));
    }
    Ok(())
}

/// Download the release's `SHA256SUMS` and return the sum of its platform binary.
fn fetch_expected_checksum(release: &Release) -> Result<String> {
    let sums = ureq::get(&release.checksums_url)
        .set("User-Agent", USER_AGENT)
        .timeout(METADATA_TIMEOUT)
        .call()
        .map_err(|e| anyhow!("Checksums download error: {}", e))?
        .into_string()
        .context("failed to read checksums")?;
    checksum_for(&sums, &release.asset_name).ok_or_else(|| {
        anyhow!(
            "No checksum for {} in {} of release {}",
            release.asset_name,
            CHECKSUMS_ASSET,
            release.tag_name
        )
    })
}

pub fn download_and_apply(release: &Release, on_progress: impl Fn(&str)) -> Result<()> {
    on_progress("Téléchargement en cours…");

    let expected = fetch_expected_checksum(release)?;

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(DOWNLOAD_CONNECT_TIMEOUT)
        .timeout_read(DOWNLOAD_READ_TIMEOUT)
        .build();
    let response = agent
        .get(&release.asset_url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| anyhow!("Download error: {}", e))?;

    install_download(
        response.into_reader(),
        &expected,
        &std::env::temp_dir(),
        MAX_ASSET_BYTES,
        &on_progress,
        |path| Ok(self_replace::self_replace(path)?),
    )?;

    // The binary is already replaced: a stale "Programs and Features" entry is
    // cosmetic, so its failure is reported but never fails the update.
    #[cfg(target_os = "windows")]
    if let Err(e) = installer_registry::set_display_version(&release.tag_name) {
        eprintln!("Version du registre non mise à jour : {e:#}");
    }

    on_progress("Mise à jour terminée — fermez et relancez l'application.");
    Ok(())
}

/// Inno Setup uninstall entry, whose `DisplayVersion` the installer writes once:
/// an in-place update must rewrite it or "Programs and Features" shows the
/// version originally installed.
#[cfg(target_os = "windows")]
mod installer_registry {
    use anyhow::{anyhow, Result};
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ,
    };

    /// `AppId` of `packaging/windows/email-to-markdown.iss`, suffixed `_is1` by Inno Setup.
    const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{8BC2AB85-9C79-4D83-A0AE-237D321B5EF6}_is1";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Rewrite `DisplayVersion` of the per-user install; a no-op when the app
    /// was not installed by the installer (key absent), never creating the key.
    pub(super) fn set_display_version(version: &str) -> Result<()> {
        let key_path = wide(UNINSTALL_KEY);
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: `key_path` is NUL-terminated and outlives the call; `key` is a valid out-pointer.
        let opened = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                key_path.as_ptr(),
                0,
                KEY_SET_VALUE,
                &mut key,
            )
        };
        if opened != ERROR_SUCCESS {
            return Ok(());
        }
        let name = wide("DisplayVersion");
        let data = wide(version);
        let byte_len = u32::try_from(data.len() * 2)?;
        // SAFETY: `key` was opened above with KEY_SET_VALUE; `data` holds `byte_len` bytes.
        let status = unsafe {
            let status = RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                byte_len,
            );
            RegCloseKey(key);
            status
        };
        if status != ERROR_SUCCESS {
            return Err(anyhow!("RegSetValueExW a échoué (code {status})"));
        }
        Ok(())
    }
}

/// Copy at most `max_bytes` of `reader` into a temporary file of `temp_dir`,
/// check it against `expected`, then hand the kept file to `replace`. Any
/// failure before `replace` deletes the download and leaves the binary intact.
fn install_download(
    reader: impl Read,
    expected: &str,
    temp_dir: &Path,
    max_bytes: u64,
    on_progress: &impl Fn(&str),
    replace: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    let mut tmp = tempfile::NamedTempFile::new_in(temp_dir)?;
    let copied = std::io::copy(&mut reader.take(max_bytes + 1), &mut tmp)
        .context("failed to download update")?;
    if copied > max_bytes {
        anyhow::bail!("update download exceeds {max_bytes} bytes, refuse to install it");
    }
    tmp.flush()?;

    // Verify before `keep()`: on mismatch `tmp` is dropped, which deletes the
    // download, and the running binary is never touched.
    on_progress("Vérification de l'intégrité…");
    verify_checksum(tmp.path(), expected)?;

    on_progress("Application de la mise à jour…");
    let (_, tmp_path) = tmp
        .keep()
        .map_err(|e| anyhow!("failed to persist temp file: {}", e))?;
    replace(&tmp_path)
}

#[cfg(test)]
mod tests {
    use super::{
        checksum_for, install_download, select_release, verify_checksum, GhRelease, Release,
    };
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    #[cfg(target_os = "windows")]
    const PLATFORM_ASSET: &str = "email-to-markdown-v0.10.0-windows-x86_64.exe";
    #[cfg(target_os = "macos")]
    const PLATFORM_ASSET: &str = "email-to-markdown-v0.10.0-macos-native";
    #[cfg(target_os = "linux")]
    const PLATFORM_ASSET: &str = "email-to-markdown-v0.10.0-linux-x86_64";

    /// SHA-256 of the three bytes `abc`.
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const OTHER_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    /// Decoded `releases/latest` answer for `tag`, with the named assets.
    fn gh_release(tag: &str, asset_names: &[&str]) -> GhRelease {
        let assets: Vec<serde_json::Value> = asset_names
            .iter()
            .map(|name| {
                serde_json::json!({
                    "name": name,
                    "browser_download_url": format!("https://example.invalid/{name}"),
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "tag_name": tag,
            "body": "notes",
            "assets": assets,
        }))
        .unwrap()
    }

    fn sums_listing() -> String {
        format!(
            "{OTHER_SHA256}  email-to-markdown-v0.10.0-other-arch\n{ABC_SHA256}  {PLATFORM_ASSET}\n"
        )
    }

    #[test]
    fn test_select_release_older_current_returns_release() {
        let response = gh_release("v0.10.0", &["SHA256SUMS", PLATFORM_ASSET]);

        let release = select_release(response, "0.8.2").unwrap();

        assert_eq!(
            release,
            Some(Release {
                tag_name: "0.10.0".to_string(),
                body: "notes".to_string(),
                asset_name: PLATFORM_ASSET.to_string(),
                asset_url: format!("https://example.invalid/{PLATFORM_ASSET}"),
                checksums_url: "https://example.invalid/SHA256SUMS".to_string(),
            })
        );
    }

    #[test]
    fn test_select_release_equal_or_newer_current_returns_none() {
        let offers = [
            select_release(
                gh_release("v0.10.0", &["SHA256SUMS", PLATFORM_ASSET]),
                "0.10.0",
            )
            .unwrap(),
            select_release(
                gh_release("v0.10.0", &["SHA256SUMS", PLATFORM_ASSET]),
                "0.11.0",
            )
            .unwrap(),
        ];

        assert_eq!(offers, [None, None]);
    }

    #[test]
    fn test_select_release_semver_boundary_offers_update() {
        let response = gh_release("v0.10.0", &["SHA256SUMS", PLATFORM_ASSET]);

        let release = select_release(response, "0.9.0").unwrap();

        assert_eq!(release.map(|r| r.tag_name), Some("0.10.0".to_string()));
    }

    #[test]
    fn test_select_release_without_checksums_asset_errors() {
        let response = gh_release("v0.10.0", &[PLATFORM_ASSET]);

        let err = select_release(response, "0.9.0").unwrap_err();

        assert!(err.to_string().contains("SHA256SUMS"));
    }

    #[test]
    fn test_checksum_for_listed_file_returns_sum() {
        assert_eq!(
            checksum_for(&sums_listing(), PLATFORM_ASSET),
            Some(ABC_SHA256.to_string())
        );
    }

    #[test]
    fn test_checksum_for_binary_mode_marker_returns_sum() {
        let sums = format!("{}  *{PLATFORM_ASSET}\n", ABC_SHA256.to_ascii_uppercase());

        assert_eq!(
            checksum_for(&sums, PLATFORM_ASSET),
            Some(ABC_SHA256.to_string())
        );
    }

    #[test]
    fn test_checksum_for_unlisted_file_returns_none() {
        let sums = format!("{OTHER_SHA256}  email-to-markdown-v0.10.0-other-arch\n");

        assert_eq!(checksum_for(&sums, PLATFORM_ASSET), None);
    }

    #[test]
    fn test_verify_checksum_matching_sum_accepts() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("binary");
        std::fs::write(&path, b"abc").unwrap();

        let outcomes = [
            verify_checksum(&path, ABC_SHA256).is_ok(),
            verify_checksum(&path, &ABC_SHA256.to_ascii_uppercase()).is_ok(),
        ];

        assert_eq!(outcomes, [true, true]);
    }

    #[test]
    fn test_verify_checksum_altered_byte_rejects() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("binary");
        std::fs::write(&path, b"abd").unwrap();

        let message = verify_checksum(&path, ABC_SHA256).unwrap_err().to_string();

        // SHA-256 of `abd`: the error names both sums.
        assert!(message.contains(ABC_SHA256));
        assert!(
            message.contains("a52d159f262b2c6ddb724a61840befc36eb30c88877a4030b65cbe86298449c9")
        );
    }

    /// Run `install_download` on `content` in `dir`; returns the result and the
    /// path handed to the replacement, if it was called.
    fn install(
        content: &[u8],
        expected: &str,
        dir: &Path,
        max_bytes: u64,
    ) -> (anyhow::Result<()>, Option<PathBuf>) {
        let replaced = RefCell::new(None);
        let result = install_download(content, expected, dir, max_bytes, &|_| {}, |path| {
            *replaced.borrow_mut() = Some(path.to_path_buf());
            Ok(())
        });
        (result, replaced.into_inner())
    }

    fn dir_entries(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect()
    }

    #[test]
    fn test_install_download_matching_sum_replaces() {
        let temp = TempDir::new().unwrap();

        let (result, replaced) = install(b"abc", ABC_SHA256, temp.path(), 3);

        result.unwrap();
        let path = replaced.expect("replacement called");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"abc",
            "kept file handed over"
        );
    }

    #[test]
    fn test_install_download_wrong_sum_never_replaces() {
        let temp = TempDir::new().unwrap();

        let (result, replaced) = install(b"abc", OTHER_SHA256, temp.path(), 3);

        assert!(result.is_err());
        assert_eq!(replaced, None);
        assert!(dir_entries(temp.path()).is_empty(), "download deleted");
    }

    #[test]
    fn test_install_download_oversized_stream_never_replaces() {
        let temp = TempDir::new().unwrap();

        let (result, replaced) = install(b"abcd", ABC_SHA256, temp.path(), 3);

        let error = format!("{:#}", result.unwrap_err());
        assert!(error.contains("exceeds 3 bytes"), "{error}");
        assert_eq!(replaced, None);
        assert!(dir_entries(temp.path()).is_empty(), "download deleted");
    }
}
