use std::io::Write;
use std::sync::LazyLock;

use anyhow::{anyhow, Result};
use regex::Regex;
use serde::Deserialize;

const GITHUB_API_LATEST: &str =
    "https://api.github.com/repos/RebelliousSmile/email2markdown.app/releases/latest";

static SEMVER_V_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^v").expect("static regex"));

#[derive(Debug, Clone)]
pub struct Release {
    pub tag_name: String,
    pub body: String,
    pub asset_url: String,
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
        .set("User-Agent", "email-to-markdown-updater")
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| anyhow!("GitHub API error: {}", e))?
        .into_json()?;

    let remote_tag = SEMVER_V_PREFIX.replace(&response.tag_name, "").to_string();

    let remote_ver = semver::Version::parse(&remote_tag)
        .map_err(|e| anyhow!("Invalid remote version '{}': {}", remote_tag, e))?;
    let current_ver = semver::Version::parse(current)
        .map_err(|e| anyhow!("Invalid current version '{}': {}", current, e))?;
    if remote_ver <= current_ver {
        return Ok(None);
    }

    let asset = response
        .assets
        .into_iter()
        .find(|a| a.name.ends_with(".exe"))
        .ok_or_else(|| anyhow!("No .exe asset found in release {}", response.tag_name))?;

    Ok(Some(Release {
        tag_name: remote_tag,
        body: response.body.unwrap_or_default(),
        asset_url: asset.browser_download_url,
    }))
}

pub fn download_and_apply(asset_url: &str, on_progress: impl Fn(&str)) -> Result<()> {
    on_progress("Téléchargement en cours…");

    let response = ureq::get(asset_url)
        .set("User-Agent", "email-to-markdown-updater")
        .call()
        .map_err(|e| anyhow!("Download error: {}", e))?;

    let mut tmp = tempfile::NamedTempFile::new()?;
    let mut reader = response.into_reader();
    std::io::copy(&mut reader, &mut tmp)?;
    tmp.flush()?;

    on_progress("Application de la mise à jour…");
    let (_, tmp_path) = tmp.keep().map_err(|e| anyhow!("failed to persist temp file: {}", e))?;
    self_replace::self_replace(&tmp_path)?;

    on_progress("Mise à jour terminée — fermez et relancez l'application.");
    Ok(())
}
