use super::{ArtifactState, ArtifactStatus};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Manager { Nautilus, Dolphin }

fn home() -> Result<PathBuf> { dirs::home_dir().context("cannot locate user home directory") }
fn shell_quote(value: &str) -> String { format!("'{}'", value.replace('\'', "'\\''")) }
fn desktop_quote(value: &str) -> String { format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")) }

fn detect(home: &Path) -> Vec<Manager> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    let mut managers = Vec::new();
    if desktop.contains("gnome") || desktop.contains("unity") || home.join(".local/share/nautilus").exists() {
        managers.push(Manager::Nautilus);
    }
    if desktop.contains("kde") || home.join(".local/share/kio").exists() {
        managers.push(Manager::Dolphin);
    }
    managers
}

fn nautilus_path(home: &Path) -> PathBuf { home.join(".local/share/nautilus/scripts/Email to Markdown") }
fn dolphin_path(home: &Path) -> PathBuf { home.join(".local/share/kio/servicemenus/email-to-markdown.desktop") }
fn render_nautilus(binary: &Path) -> String {
    include_str!("../../packaging/linux/email-to-markdown-nautilus")
        .replace("__BINARY_SHELL_QUOTED__", &shell_quote(&binary.to_string_lossy()))
}
fn render_dolphin(binary: &Path) -> String {
    include_str!("../../packaging/linux/email-to-markdown-dolphin.desktop")
        .replace("__BINARY_DESKTOP_QUOTED__", &desktop_quote(&binary.to_string_lossy()))
}

fn install_at(home: &Path, binary: &Path, managers: &[Manager]) -> Result<()> {
    if managers.is_empty() {
        anyhow::bail!("no supported Linux file manager detected (Nautilus or Dolphin); use: {} contextual <directory>", binary.display());
    }
    for manager in managers {
        let (path, content) = match manager {
            Manager::Nautilus => (nautilus_path(home), render_nautilus(binary)),
            Manager::Dolphin => (dolphin_path(home), render_dolphin(binary)),
        };
        if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
        fs::write(&path, content)?;
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn artifact(path: PathBuf, name: &str, expected: String) -> Result<ArtifactStatus> {
    let state = match fs::read_to_string(&path) {
        Ok(content) if content == expected => ArtifactState::Installed,
        Ok(content) => ArtifactState::Stale {
            configured_binary: content.lines().find(|line| line.contains("contextual")).unwrap_or("artefact différent").trim().into(),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ArtifactState::Missing,
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    Ok(ArtifactStatus { name: name.into(), location: path.display().to_string(), state })
}

pub fn install(binary: &Path) -> Result<Vec<ArtifactStatus>> {
    let home = home()?;
    let managers = detect(&home);
    install_at(&home, binary, &managers)?;
    status(binary)
}

pub fn status(binary: &Path) -> Result<Vec<ArtifactStatus>> {
    let home = home()?;
    Ok(vec![
        artifact(nautilus_path(&home), "Nautilus — script", render_nautilus(binary))?,
        artifact(dolphin_path(&home), "Dolphin — menu de service", render_dolphin(binary))?,
    ])
}

fn uninstall_at(home: &Path) -> Result<()> {
    for path in [nautilus_path(&home), dolphin_path(&home)] {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("remove managed artifact {}", path.display())),
        }
    }
    Ok(())
}

pub fn uninstall() -> Result<()> {
    uninstall_at(&home()?)
}

#[cfg(test)]
mod tests {
    use super::{artifact, dolphin_path, install_at, nautilus_path, uninstall_at, Manager};
    use crate::shell_integration::ArtifactState;
    use std::path::Path;

    #[test]
    fn fixture_install_is_idempotent_repairs_stale_and_removes_no_third_party_file() {
        let temp = tempfile::TempDir::new().unwrap();
        let binary = Path::new("/opt/Email Été/email-to-markdown");
        let managers = [Manager::Nautilus, Manager::Dolphin];
        install_at(temp.path(), binary, &managers).unwrap();
        install_at(temp.path(), binary, &managers).unwrap();
        let third_party = temp.path().join(".local/share/nautilus/scripts/keep-me");
        std::fs::write(&third_party, "third party").unwrap();
        assert!(nautilus_path(temp.path()).exists());
        assert!(dolphin_path(temp.path()).exists());
        assert!(third_party.exists());

        std::fs::write(nautilus_path(temp.path()), "stale binary").unwrap();
        let stale = artifact(nautilus_path(temp.path()), "Nautilus", "expected".into()).unwrap();
        assert!(matches!(stale.state, ArtifactState::Stale { .. }));
        install_at(temp.path(), binary, &managers).unwrap();
        assert!(!std::fs::read_to_string(nautilus_path(temp.path())).unwrap().contains("stale binary"));
        uninstall_at(temp.path()).unwrap();
        uninstall_at(temp.path()).unwrap();
        assert!(!nautilus_path(temp.path()).exists());
        assert!(!dolphin_path(temp.path()).exists());
        assert!(third_party.exists());
    }

    #[test]
    fn unsupported_manager_writes_nothing() {
        let temp = tempfile::TempDir::new().unwrap();
        assert!(install_at(temp.path(), Path::new("/usr/bin/email-to-markdown"), &[]).is_err());
        assert!(!temp.path().join(".local").exists());
    }
}
