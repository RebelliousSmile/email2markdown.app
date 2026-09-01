//! Per-user file-manager integration for the standalone contextual window.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub const REBUILD_HINT: &str = "cargo build --release --features tray";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactState {
    Installed,
    Missing,
    Stale { configured_binary: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactStatus {
    pub name: String,
    pub location: String,
    pub state: ArtifactState,
}

pub fn current_binary() -> Result<PathBuf> {
    let path = std::env::current_exe().context("cannot resolve current executable")?;
    let canonical = path.canonicalize().unwrap_or(path);
    Ok(shell_compatible_path(canonical))
}

#[cfg(target_os = "windows")]
fn shell_compatible_path(path: PathBuf) -> PathBuf {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = value.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

#[cfg(not(target_os = "windows"))]
fn shell_compatible_path(path: PathBuf) -> PathBuf { path }

pub fn install() -> Result<Vec<ArtifactStatus>> {
    if !cfg!(feature = "tray") {
        anyhow::bail!(
            "file-manager integration requires the GUI build; rebuild with: {REBUILD_HINT}"
        );
    }
    let binary = current_binary()?;
    platform_install(&binary)
}

pub fn status() -> Result<Vec<ArtifactStatus>> {
    platform_status(&current_binary()?)
}

pub fn uninstall() -> Result<Vec<ArtifactStatus>> {
    platform_uninstall()?;
    status()
}

/// Convert a file-manager URI into a native local path. Remote URIs and file
/// authorities other than localhost are deliberately rejected.
pub fn local_path_from_uri(value: &str) -> Result<PathBuf> {
    let parsed = url::Url::parse(value).context("invalid file-manager URI")?;
    if parsed.scheme() != "file" {
        anyhow::bail!("only local file:// URIs are accepted");
    }
    if parsed.host_str().is_some_and(|host| !host.is_empty() && host != "localhost") {
        anyhow::bail!("remote file URI authorities are not accepted");
    }
    parsed
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("URI cannot be represented as a local native path"))
}

pub fn format_status(statuses: &[ArtifactStatus]) -> String {
    statuses
        .iter()
        .map(|artifact| {
            let state = match &artifact.state {
                ArtifactState::Installed => "installé".to_string(),
                ArtifactState::Missing => "absent".to_string(),
                ArtifactState::Stale { configured_binary } => {
                    format!("chemin périmé ({configured_binary})")
                }
            };
            format!("{}: {} — {}", artifact.name, state, artifact.location)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Called after self-replacement. Missing integrations stay opt-in; installed
/// or stale managed artifacts are repaired to the current executable path.
#[cfg(feature = "tray")]
pub fn repair_if_installed() -> Result<()> {
    let states = status()?;
    if states.iter().any(|artifact| !matches!(artifact.state, ArtifactState::Missing)) {
        install()?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn platform_install(binary: &Path) -> Result<Vec<ArtifactStatus>> { windows::install(binary) }
#[cfg(target_os = "windows")]
fn platform_status(binary: &Path) -> Result<Vec<ArtifactStatus>> { windows::status(binary) }
#[cfg(target_os = "windows")]
fn platform_uninstall() -> Result<()> { windows::uninstall() }

#[cfg(target_os = "macos")]
fn platform_install(binary: &Path) -> Result<Vec<ArtifactStatus>> { macos::install(binary) }
#[cfg(target_os = "macos")]
fn platform_status(binary: &Path) -> Result<Vec<ArtifactStatus>> { macos::status(binary) }
#[cfg(target_os = "macos")]
fn platform_uninstall() -> Result<()> { macos::uninstall() }

#[cfg(target_os = "linux")]
fn platform_install(binary: &Path) -> Result<Vec<ArtifactStatus>> { linux::install(binary) }
#[cfg(target_os = "linux")]
fn platform_status(binary: &Path) -> Result<Vec<ArtifactStatus>> { linux::status(binary) }
#[cfg(target_os = "linux")]
fn platform_uninstall() -> Result<()> { linux::uninstall() }

#[cfg(test)]
mod tests {
    use super::local_path_from_uri;

    #[test]
    fn local_uri_is_decoded_and_remote_schemes_are_rejected() {
        #[cfg(target_os = "windows")]
        let uri = "file:///C:/Temp/Client%20%C3%89t%C3%A9";
        #[cfg(not(target_os = "windows"))]
        let uri = "file:///tmp/Client%20%C3%89t%C3%A9";
        let path = local_path_from_uri(uri).unwrap();
        assert!(path.to_string_lossy().contains("Client Été"));
        assert!(local_path_from_uri("smb://server/share/client").is_err());
        assert!(local_path_from_uri("file://server/share/client").is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn explorer_binary_path_removes_windows_verbatim_prefixes() {
        use super::shell_compatible_path;
        assert_eq!(
            shell_compatible_path(std::path::PathBuf::from(
                r"\\?\C:\Program Files\Email Été\email-to-markdown.exe"
            )),
            std::path::PathBuf::from(r"C:\Program Files\Email Été\email-to-markdown.exe")
        );
        assert_eq!(
            shell_compatible_path(std::path::PathBuf::from(
                r"\\?\UNC\server\share\email-to-markdown.exe"
            )),
            std::path::PathBuf::from(r"\\server\share\email-to-markdown.exe")
        );
    }
}
