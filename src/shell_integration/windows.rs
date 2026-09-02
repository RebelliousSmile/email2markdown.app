use super::{ArtifactState, ArtifactStatus};
use crate::app_icon;
use anyhow::{Context, Result};
use std::path::Path;
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

const SELECTED_KEY: &str = r"Software\Classes\Directory\shell\EmailToMarkdownContextual";
const BACKGROUND_KEY: &str =
    r"Software\Classes\Directory\Background\shell\EmailToMarkdownContextual";
const COMMAND_CLSID: &str = email_to_markdown_shell_extension_contract::COMMAND_CLSID_BRACED;
static CLSID_KEY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(r"Software\Classes\CLSID\{COMMAND_CLSID}")
});
const MODERN_DLL_NAME: &str =
    concat!("email-to-markdown-shell-extension-", env!("CARGO_PKG_VERSION"), ".dll");
const MAIL_ICON_NAME: &str = "email-to-markdown-mail.ico";
const IDENTITY_PACKAGE_NAME: &str = "FXGuillois.EmailToMarkdown";
const IDENTITY_PACKAGE_FILE: &str = "email-to-markdown-identity.msix";

#[link(name = "shell32")]
extern "system" {
    fn SHChangeNotify(
        event_id: i32,
        flags: u32,
        item1: *const std::ffi::c_void,
        item2: *const std::ffi::c_void,
    );
}

fn refresh_explorer_associations() {
    const SHCNE_ASSOCCHANGED: i32 = 0x0800_0000;
    const SHCNF_IDLIST: u32 = 0;
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
}

fn command(binary: &Path, placeholder: &str) -> String {
    format!("\"{}\" contextual \"{}\"", binary.display(), placeholder)
}

fn write_verb(
    root: &RegKey,
    key_path: &str,
    binary: &Path,
    icon: &Path,
    placeholder: &str,
) -> Result<()> {
    let (verb, _) = root.create_subkey(key_path)?;
    verb.set_value("", &"Convertir les emails associés en Markdown")?;
    verb.set_value("Icon", &icon.to_string_lossy().as_ref())?;
    let (command_key, _) = verb.create_subkey("command")?;
    command_key.set_value("", &command(binary, placeholder))?;
    Ok(())
}

fn modern_extension_path(binary: &Path) -> std::path::PathBuf {
    binary.with_file_name(MODERN_DLL_NAME)
}

fn mail_icon_path(binary: &Path) -> std::path::PathBuf {
    binary.with_file_name(MAIL_ICON_NAME)
}

fn identity_package_path(binary: &Path) -> std::path::PathBuf {
    binary.with_file_name(IDENTITY_PACKAGE_FILE)
}

fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn run_powershell(script: &str) -> Result<std::process::Output> {
    let powershell = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"))
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    std::process::Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
        .context("launch PowerShell for Windows package registration")
}

fn register_identity_package(binary: &Path) -> Result<bool> {
    let package = identity_package_path(binary);
    if !package.is_file() {
        return Ok(false);
    }
    let external_location = binary
        .parent()
        .context("the executable has no installation directory")?;
    let script = format!(
        "$existing = Get-AppxPackage -Name {}; if ($existing) {{ $existing | Remove-AppxPackage }}; \
         Add-AppxPackage -Path {} -ExternalLocation {}",
        powershell_quote(IDENTITY_PACKAGE_NAME),
        powershell_quote(&package.to_string_lossy()),
        powershell_quote(&external_location.to_string_lossy())
    );
    let output = run_powershell(&script)?;
    if !output.status.success() {
        anyhow::bail!(
            "Windows 11 identity package registration failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(true)
}

fn identity_package_is_registered() -> bool {
    let script = format!(
        "if (Get-AppxPackage -Name {}) {{ exit 0 }} else {{ exit 1 }}",
        powershell_quote(IDENTITY_PACKAGE_NAME)
    );
    run_powershell(&script)
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn unregister_identity_package() -> Result<()> {
    let script = format!(
        "Get-AppxPackage -Name {} | Remove-AppxPackage",
        powershell_quote(IDENTITY_PACKAGE_NAME)
    );
    let output = run_powershell(&script)?;
    if !output.status.success() {
        anyhow::bail!(
            "Windows 11 identity package removal failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn install_modern_verb(root: &RegKey, binary: &Path, icon: &Path) -> Result<bool> {
    let extension = modern_extension_path(binary);
    if !extension.is_file() {
        return Ok(false);
    }
    let (class, _) = root.create_subkey(CLSID_KEY.as_str())?;
    class.set_value("", &"Email to Markdown Explorer command")?;
    let (server, _) = class.create_subkey("InprocServer32")?;
    server.set_value("", &extension.to_string_lossy().as_ref())?;
    server.set_value("ThreadingModel", &"Apartment")?;

    for key_path in [SELECTED_KEY, BACKGROUND_KEY] {
        let (verb, _) = root.create_subkey(key_path)?;
        verb.set_value("MUIVerb", &"Importer les emails en Markdown")?;
        verb.set_value("Icon", &icon.to_string_lossy().as_ref())?;
        verb.set_value("ExplorerCommandHandler", &COMMAND_CLSID)?;
    }
    Ok(true)
}

pub fn install(binary: &Path) -> Result<Vec<ArtifactStatus>> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    // Register the identity first: if Windows rejects the signed package, do
    // not leave a legacy-only integration that looks like a successful install.
    register_identity_package(binary)?;
    let icon = mail_icon_path(binary);
    std::fs::write(&icon, app_icon::windows_ico())
        .with_context(|| format!("write Windows application icon {}", icon.display()))?;
    write_verb(&root, SELECTED_KEY, binary, &icon, "%1")?;
    write_verb(&root, BACKGROUND_KEY, binary, &icon, "%V")?;
    install_modern_verb(&root, binary, &icon)?;
    refresh_explorer_associations();
    status(binary)
}

fn modern_status(root: &RegKey, binary: &Path) -> ArtifactStatus {
    let extension = modern_extension_path(binary);
    let registered_server = root
        .open_subkey(format!(r"{}\InprocServer32", CLSID_KEY.as_str()))
        .and_then(|key| key.get_value::<String, _>(""));
    let registered_handler = root
        .open_subkey(SELECTED_KEY)
        .and_then(|key| key.get_value::<String, _>("ExplorerCommandHandler"));
    let state = match (
        extension.is_file(),
        identity_package_is_registered(),
        registered_server,
        registered_handler,
    ) {
        (true, true, Ok(server), Ok(handler))
            if server == extension.to_string_lossy() && handler == COMMAND_CLSID =>
        {
            ArtifactState::Installed
        }
        (false, _, _, _) | (_, false, _, _) => ArtifactState::Missing,
        (_, _, server, handler) => ArtifactState::Stale {
            configured_binary: format!(
                "serveur={}, gestionnaire={}",
                server.unwrap_or_else(|_| "absent".into()),
                handler.unwrap_or_else(|_| "absent".into())
            ),
        },
    };
    ArtifactStatus {
        name: "Explorer Windows 11 — menu principal".into(),
        location: format!(r"HKCU\{}", CLSID_KEY.as_str()),
        state,
    }
}

fn one_status(
    root: &RegKey,
    key_path: &str,
    name: &str,
    binary: &Path,
    placeholder: &str,
) -> ArtifactStatus {
    let expected = command(binary, placeholder);
    let configured = root
        .open_subkey(format!(r"{}\command", key_path))
        .and_then(|key| key.get_value::<String, _>(""));
    let state = match configured {
        Ok(value) if value == expected => ArtifactState::Installed,
        Ok(value) => ArtifactState::Stale {
            configured_binary: value,
        },
        Err(_) => ArtifactState::Missing,
    };
    ArtifactStatus {
        name: name.into(),
        location: format!(r"HKCU\{}", key_path),
        state,
    }
}

pub fn status(binary: &Path) -> Result<Vec<ArtifactStatus>> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    Ok(vec![
        modern_status(&root, binary),
        one_status(
            &root,
            SELECTED_KEY,
            "Explorer — dossier sélectionné",
            binary,
            "%1",
        ),
        one_status(
            &root,
            BACKGROUND_KEY,
            "Explorer — fond du dossier",
            binary,
            "%V",
        ),
    ])
}

pub fn uninstall() -> Result<()> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    unregister_identity_package()?;
    for key in [SELECTED_KEY, BACKGROUND_KEY, CLSID_KEY.as_str()] {
        match root.delete_subkey_all(key) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("remove managed registry key {key}"))
            }
        }
    }
    refresh_explorer_associations();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        command, identity_package_path, modern_extension_path, powershell_quote, MODERN_DLL_NAME,
    };
    use std::path::Path;

    #[test]
    fn explorer_command_quotes_unicode_paths_and_placeholder() {
        assert_eq!(
            command(
                Path::new(r"C:\Program Files\Email Été\email-to-markdown.exe"),
                "%1"
            ),
            r#""C:\Program Files\Email Été\email-to-markdown.exe" contextual "%1""#
        );
    }

    #[test]
    fn modern_extension_lives_beside_the_portable_executable() {
        assert_eq!(
            modern_extension_path(Path::new(r"C:\Apps\email-to-markdown.exe")),
            Path::new(r"C:\Apps").join(MODERN_DLL_NAME)
        );
    }

    #[test]
    fn identity_package_lives_beside_the_installed_executable() {
        assert_eq!(
            identity_package_path(Path::new(r"C:\Apps\email-to-markdown.exe")),
            Path::new(r"C:\Apps\email-to-markdown-identity.msix")
        );
    }

    #[test]
    fn powershell_paths_are_single_quote_escaped() {
        assert_eq!(
            powershell_quote(r"C:\L'été\app.msix"),
            r"'C:\L''été\app.msix'"
        );
    }
}
