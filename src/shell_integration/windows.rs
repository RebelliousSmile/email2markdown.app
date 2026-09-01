use super::{ArtifactState, ArtifactStatus};
use anyhow::{Context, Result};
use std::path::Path;
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

const SELECTED_KEY: &str = r"Software\Classes\Directory\shell\EmailToMarkdownContextual";
const BACKGROUND_KEY: &str = r"Software\Classes\Directory\Background\shell\EmailToMarkdownContextual";

fn command(binary: &Path, placeholder: &str) -> String {
    format!("\"{}\" contextual \"{}\"", binary.display(), placeholder)
}

fn write_verb(root: &RegKey, key_path: &str, binary: &Path, placeholder: &str) -> Result<()> {
    let (verb, _) = root.create_subkey(key_path)?;
    verb.set_value("", &"Convertir les emails associés en Markdown")?;
    verb.set_value("Icon", &binary.to_string_lossy().as_ref())?;
    let (command_key, _) = verb.create_subkey("command")?;
    command_key.set_value("", &command(binary, placeholder))?;
    Ok(())
}

pub fn install(binary: &Path) -> Result<Vec<ArtifactStatus>> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    write_verb(&root, SELECTED_KEY, binary, "%1")?;
    write_verb(&root, BACKGROUND_KEY, binary, "%V")?;
    status(binary)
}

fn one_status(root: &RegKey, key_path: &str, name: &str, binary: &Path, placeholder: &str) -> ArtifactStatus {
    let expected = command(binary, placeholder);
    let configured = root
        .open_subkey(format!(r"{}\command", key_path))
        .and_then(|key| key.get_value::<String, _>(""));
    let state = match configured {
        Ok(value) if value == expected => ArtifactState::Installed,
        Ok(value) => ArtifactState::Stale { configured_binary: value },
        Err(_) => ArtifactState::Missing,
    };
    ArtifactStatus { name: name.into(), location: format!(r"HKCU\{}", key_path), state }
}

pub fn status(binary: &Path) -> Result<Vec<ArtifactStatus>> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    Ok(vec![
        one_status(&root, SELECTED_KEY, "Explorer — dossier sélectionné", binary, "%1"),
        one_status(&root, BACKGROUND_KEY, "Explorer — fond du dossier", binary, "%V"),
    ])
}

pub fn uninstall() -> Result<()> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    for key in [SELECTED_KEY, BACKGROUND_KEY] {
        match root.delete_subkey_all(key) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("remove managed registry key {key}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::command;
    use std::path::Path;

    #[test]
    fn explorer_command_quotes_unicode_paths_and_placeholder() {
        assert_eq!(
            command(Path::new(r"C:\Program Files\Email Été\email-to-markdown.exe"), "%1"),
            r#""C:\Program Files\Email Été\email-to-markdown.exe" contextual "%1""#
        );
    }
}
