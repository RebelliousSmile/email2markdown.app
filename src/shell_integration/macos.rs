use super::{ArtifactState, ArtifactStatus};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

const WORKFLOW_NAME: &str = "Email to Markdown.workflow";

fn workflow_root() -> Result<PathBuf> {
    Ok(workflow_root_at(
        &dirs::home_dir().context("cannot locate user home directory")?,
    ))
}
fn workflow_root_at(home: &Path) -> PathBuf {
    home.join("Library/Services").join(WORKFLOW_NAME)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn render(binary: &Path) -> String {
    include_str!("../../packaging/macos/Email to Markdown.workflow/Contents/document.wflow")
        .replace(
            "__BINARY_SHELL_QUOTED__",
            &xml_escape(&shell_quote(&binary.to_string_lossy())),
        )
}

fn install_at(home: &Path, binary: &Path) -> Result<()> {
    let root = workflow_root_at(home);
    let contents = root.join("Contents");
    fs::create_dir_all(&contents)?;
    fs::write(contents.join("document.wflow"), render(binary))?;
    Ok(())
}

pub fn install(binary: &Path) -> Result<Vec<ArtifactStatus>> {
    let home = dirs::home_dir().context("cannot locate user home directory")?;
    install_at(&home, binary)?;
    status(binary)
}

pub fn status(binary: &Path) -> Result<Vec<ArtifactStatus>> {
    let path = workflow_root()?.join("Contents/document.wflow");
    let state = super::classify_artifact(&path, &render(binary), "workflow différent")?;
    Ok(vec![ArtifactStatus {
        name: "Finder — Action rapide".into(),
        location: path.display().to_string(),
        state,
    }])
}

fn uninstall_at(home: &Path) -> Result<()> {
    let root = workflow_root_at(home);
    match fs::remove_dir_all(&root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("remove managed workflow {}", root.display()))
        }
    }
}

pub fn uninstall() -> Result<()> {
    uninstall_at(&dirs::home_dir().context("cannot locate user home directory")?)
}

#[cfg(test)]
mod tests {
    use super::{install_at, render, uninstall_at, workflow_root_at};
    use std::path::Path;
    #[test]
    fn workflow_quotes_binary_path() {
        let rendered = render(Path::new("/Applications/Email Été/email-to-markdown"));
        assert!(rendered.contains("'/Applications/Email Été/email-to-markdown' contextual"));
    }

    #[test]
    fn workflow_fixture_install_and_uninstall_are_scoped_and_idempotent() {
        let temp = tempfile::TempDir::new().unwrap();
        let binary = Path::new("/Applications/Email to Markdown/email-to-markdown");
        let third_party = temp.path().join("Library/Services/Keep.workflow/Contents");
        std::fs::create_dir_all(&third_party).unwrap();
        install_at(temp.path(), binary).unwrap();
        install_at(temp.path(), binary).unwrap();
        assert!(workflow_root_at(temp.path())
            .join("Contents/document.wflow")
            .exists());
        uninstall_at(temp.path()).unwrap();
        uninstall_at(temp.path()).unwrap();
        assert!(!workflow_root_at(temp.path()).exists());
        assert!(third_party.exists());
    }
}
