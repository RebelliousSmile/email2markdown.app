use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

use email_to_markdown::config::{self, Config, Settings};
use email_to_markdown::dest_cmd;
use email_to_markdown::email_export::ImapExporter;
use email_to_markdown::route;
use email_to_markdown::thunderbird; // [1] Import Thunderbird

#[cfg(feature = "tray")]
use email_to_markdown::tray;

/// Detach the tray process from a hosting console (e.g. Windows Terminal),
/// best-effort. Redirects stdout/stderr to `NUL` first so any later
/// `println!`/`eprintln!` in `tray.rs` writes silently instead of panicking
/// on the now-invalid inherited console handle.
#[cfg(all(windows, feature = "tray"))]
fn detach_console() {
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        FreeConsole, SetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    };

    let nul: Vec<u16> = std::ffi::OsStr::new("NUL")
        .encode_wide()
        .chain(once(0))
        .collect();

    let handle = unsafe {
        CreateFileW(
            nul.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };

    if handle != INVALID_HANDLE_VALUE {
        unsafe {
            SetStdHandle(STD_OUTPUT_HANDLE, handle);
            SetStdHandle(STD_ERROR_HANDLE, handle);
        }
    }

    unsafe {
        FreeConsole();
    }
}

fn run_contextual_or_bail(directory: PathBuf) -> Result<()> {
    #[cfg(feature = "tray")]
    {
        tray::run_contextual(directory).context("Failed to run contextual email export")
    }
    #[cfg(not(feature = "tray"))]
    {
        let _ = directory;
        anyhow::bail!("the contextual window requires a build with the 'tray' GUI feature")
    }
}

#[derive(Parser)]
#[command(name = "email-to-markdown")]
#[command(author = "FX Guillois")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Export emails from IMAP accounts to Markdown files", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// [1] Import accounts configuration from Thunderbird
    Import {
        /// Path to Thunderbird profile (optional, auto-detect if not specified)
        #[arg(short, long)]
        profile: Option<PathBuf>,

        /// List available Thunderbird profiles
        #[arg(long)]
        list_profiles: bool,

        /// Output path for accounts.yaml (default: platform config dir)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Also generate .env template
        #[arg(long)]
        generate_env: bool,

        /// Extract passwords from Thunderbird and write them to .env
        /// (Thunderbird must be closed during this operation)
        #[arg(long)]
        extract_passwords: bool,

        /// Thunderbird Master Password (only needed if you configured one)
        #[arg(long)]
        master_password: Option<String>,
    },

    /// Export emails from IMAP accounts
    Export {
        /// Export only specific account(s) - comma separated
        #[arg(short, long)]
        account: Option<String>,

        /// List available accounts
        #[arg(long)]
        list_accounts: bool,

        /// Delete emails after export (dangerous!)
        #[arg(long)]
        delete_after_export: bool,

        /// Path to config file (default: platform config dir)
        #[arg(short, long)]
        config: Option<PathBuf>,

        /// Enable debug mode (verbose IMAP output)
        #[arg(short, long)]
        debug: bool,
    },

    /// Manage routing destinations (list, add)
    Dest(dest_cmd::DestArgs),

    /// Repair legacy centralized attachment paths in already-moved .md files.
    ///
    /// When emails were exported with the old centralized scheme (attachments stored
    /// under `<account>/attachments/<folder>/`), moving them to notes left the
    /// frontmatter paths broken. This command finds those files, moves the attachments
    /// co-located with their .md, and updates the frontmatter to bare filenames.
    RepairAttachments {
        /// Account name (e.g. pro@fxguillois.email) — locates the attachment source dir
        #[arg(short, long)]
        account: String,

        /// Preview changes without applying them
        #[arg(long)]
        dry_run: bool,

        /// Path to config file (default: platform config dir)
        #[arg(short, long)]
        config: Option<PathBuf>,
    },

    /// Search and convert related emails into one configured local directory.
    Contextual {
        /// Exact configured destination directory selected in the file manager.
        directory: PathBuf,
    },

    /// Install, inspect or remove the current OS file-manager action.
    Shell {
        #[command(subcommand)]
        action: ShellAction,
    },

    /// Open from a local file:// URI supplied by a file manager.
    #[command(hide = true)]
    ContextualUri { uri: String },

    /// Run as system tray application (requires --features tray)
    #[cfg(feature = "tray")]
    Tray,
}

#[derive(Subcommand)]
enum ShellAction {
    /// Install or repair the per-user action for the current executable.
    Install,
    /// Report missing, installed or stale managed artifacts.
    Status,
    /// Remove only artifacts owned by Email to Markdown.
    Uninstall,
}

fn main() -> Result<()> {
    // Load .env from the platform config directory
    dotenvy::from_path(config::env_file_path()).ok();

    let cli = Cli::parse();

    match cli.command {
        // [1] Handler pour l'import Thunderbird
        Commands::Import {
            profile,
            list_profiles,
            output,
            generate_env,
            extract_passwords,
            master_password,
        } => {
            if list_profiles {
                println!("Available Thunderbird profiles:");
                match thunderbird::list_profiles() {
                    Ok(profiles) => {
                        for (i, p) in profiles.iter().enumerate() {
                            let default_marker = if p.is_default { " (default)" } else { "" };
                            println!(
                                "   {}. {}{} -> {}",
                                i + 1,
                                p.name,
                                default_marker,
                                p.path.display()
                            );
                        }
                    }
                    Err(e) => {
                        println!("Could not list profiles: {}", e);
                    }
                }
                return Ok(());
            }

            // Get profile to use
            let tb_profile = if let Some(profile_path) = profile {
                thunderbird::ThunderbirdProfile {
                    name: "Custom".to_string(),
                    path: profile_path,
                    is_default: false,
                }
            } else {
                // Auto-detect default profile
                let profiles =
                    thunderbird::list_profiles().context("Could not find Thunderbird profiles")?;

                // Prefer the marked default, but only if it has prefs.js (it may be an empty placeholder)
                let has_prefs =
                    |p: &thunderbird::ThunderbirdProfile| p.path.join("prefs.js").exists();

                profiles
                    .iter()
                    .find(|p| p.is_default && has_prefs(p))
                    .or_else(|| profiles.iter().find(|p| has_prefs(p)))
                    .cloned()
                    .context("No usable Thunderbird profiles found (no prefs.js)")?
            };

            println!(
                "Using Thunderbird profile: {} ({})",
                tb_profile.name,
                tb_profile.path.display()
            );

            // Extract accounts
            let accounts = thunderbird::extract_accounts(&tb_profile)
                .context("Failed to extract accounts from Thunderbird")?;

            if accounts.is_empty() {
                println!("No IMAP accounts found in Thunderbird profile");
                return Ok(());
            }

            println!("Found {} IMAP account(s):", accounts.len());
            for acc in &accounts {
                println!("   - {} ({})", acc.name, acc.server);
            }

            // Generate accounts.yaml
            let yaml_content = thunderbird::generate_accounts_yaml(&accounts);
            let output = output.unwrap_or_else(config::accounts_yaml_path);

            // Create output directory if needed
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)?;
            }

            std::fs::write(&output, &yaml_content)?;
            println!("\nGenerated: {}", output.display());

            // Generate .env template if requested
            if generate_env {
                let env_template_path = output
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join(".env.template");
                let env_content = thunderbird::generate_env_template(&accounts);
                std::fs::write(&env_template_path, &env_content)?;
                println!("Generated: {}", env_template_path.display());
                println!("\nRemember to:");
                println!("  1. Review and adjust accounts.yaml");
                println!(
                    "  2. Copy .env.template to {} and add passwords",
                    config::env_file_path().display()
                );
            } else if !extract_passwords {
                println!(
                    "\nRemember to add passwords to {}",
                    config::env_file_path().display()
                );
            }

            // Extract and write passwords from Thunderbird keystore
            if extract_passwords {
                println!("\nExtracting passwords from Thunderbird...");
                println!("Note: Thunderbird must be closed during this operation.");

                if master_password.is_some() {
                    println!("Using provided Master Password for authentication.");
                }

                match thunderbird::extract_passwords(&tb_profile, master_password.as_deref()) {
                    Ok(passwords) => {
                        if passwords.is_empty() {
                            println!("No IMAP passwords found in Thunderbird profile.");
                        } else {
                            println!("Decrypted {} password(s).", passwords.len());
                            let env_path = config::env_file_path();
                            if let Some(parent) = env_path.parent() {
                                std::fs::create_dir_all(parent)?;
                            }
                            match thunderbird::write_passwords_to_env(
                                &accounts, &passwords, &env_path,
                            ) {
                                Ok(n) => {
                                    println!("Written {} password(s) to {}", n, env_path.display())
                                }
                                Err(e) => println!("Warning: Could not write .env: {}", e),
                            }
                        }
                    }
                    Err(e) => {
                        println!("Could not extract passwords: {}", e);
                        println!("\nTips:");
                        println!("  - Close Thunderbird before running this command");
                        println!("  - If you have a Master Password, pass it with --master-password <PASSWORD>");
                    }
                }
            }
        }

        Commands::Export {
            account,
            list_accounts,
            delete_after_export,
            config,
            debug,
        } => {
            let config_path = config.unwrap_or_else(config::accounts_yaml_path);
            let config = Config::load(&config_path).context("Failed to load configuration")?;

            if list_accounts {
                println!("Available accounts from accounts.yaml:");
                for (i, acc) in config.accounts.iter().enumerate() {
                    println!("   {}. {} -> {}", i + 1, acc.name, acc.export_directory);
                }
                return Ok(());
            }

            // Determine which accounts to export
            let accounts_to_export: Vec<_> = if let Some(account_names) = account {
                let names: Vec<_> = account_names
                    .split(',')
                    .map(|s| s.trim().to_lowercase())
                    .collect();

                config
                    .accounts
                    .iter()
                    .filter(|a| names.contains(&a.name.to_lowercase()))
                    .cloned()
                    .collect()
            } else {
                config.accounts.clone()
            };

            if config.accounts.is_empty() {
                println!("No accounts configured.");
                println!(
                    "Add your IMAP accounts to {}",
                    config::accounts_yaml_path().display()
                );
                println!("Or import from Thunderbird: cargo run -- import");
                return Ok(());
            }

            if accounts_to_export.is_empty() {
                println!("No accounts selected for export");
                println!("Available accounts:");
                for acc in &config.accounts {
                    println!("   - {}", acc.name);
                }
                return Ok(());
            }

            println!("Exporting {} account(s)", accounts_to_export.len());

            for mut account in accounts_to_export {
                println!(
                    "\nProcessing account: {} -> {}",
                    account.name, account.export_directory
                );

                if account.password.is_none() {
                    println!(
                        "Error for {}: No password found. Check your .env file.",
                        account.name
                    );
                    continue;
                }

                account.delete_after_export = delete_after_export || account.delete_after_export;

                let mut exporter = ImapExporter::new(account.clone(), debug);

                match exporter.connect() {
                    Ok(_) => {
                        match exporter.export_account(None, None, None) {
                            Ok((results, decisions)) => {
                                let total_exported: usize =
                                    results.values().map(|s| s.exported).sum();
                                let total_skipped: usize =
                                    results.values().map(|s| s.skipped).sum();
                                let total_errors: usize = results.values().map(|s| s.errors).sum();

                                println!(
                                    "\nExport completed for {}: {} exported, {} skipped, {} errors",
                                    account.name, total_exported, total_skipped, total_errors
                                );

                                // CLI mode (D8): apply routing decisions automatically, no review.
                                // Pipeline order: Export → route decisions accumulated above → apply now.
                                // IMAP deletion flags were set during Export; local .md files remain
                                // in staging until this apply step moves them into notes_dir.
                                let settings =
                                    Settings::load(&config::settings_path()).unwrap_or_default();
                                if let Some(notes_dir_str) = &settings.notes_dir {
                                    let notes_dir = PathBuf::from(notes_dir_str);
                                    let mut moved = 0usize;
                                    let mut apply_errors = 0usize;
                                    for (staging_path, decision) in &decisions {
                                        match route::apply_decision(
                                            staging_path,
                                            &decision.rel_path,
                                            &notes_dir,
                                        ) {
                                            Ok(()) => moved += 1,
                                            Err(e) => {
                                                apply_errors += 1;
                                                eprintln!(
                                                    "Warning: could not route {}: {:#}",
                                                    staging_path.display(),
                                                    e
                                                );
                                            }
                                        }
                                    }
                                    if !decisions.is_empty() {
                                        println!(
                                            "Routing: {} moved to notes_dir, {} errors",
                                            moved, apply_errors
                                        );
                                    }
                                } else if !decisions.is_empty() {
                                    println!(
                                        "Note: notes_dir not configured in settings.yaml — \
                                         {} emails remain in staging (not routed)",
                                        decisions.len()
                                    );
                                }
                            }
                            Err(e) => {
                                println!("Export failed for {}: {}", account.name, e);
                            }
                        }

                        if let Err(e) = exporter.disconnect() {
                            println!("Warning: Disconnect error: {}", e);
                        }
                    }
                    Err(e) => {
                        println!("Connection failed for {}: {}", account.name, e);
                    }
                }
            }
        }

        Commands::Dest(args) => {
            dest_cmd::run(args)?;
        }

        Commands::RepairAttachments {
            account,
            dry_run,
            config,
        } => {
            let settings = Settings::load(config.as_deref().unwrap_or(&config::settings_path()))
                .unwrap_or_default();

            let export_base = settings.export_base_dir.as_deref().unwrap_or(".");
            let notes_dir_str = settings.notes_dir.as_deref().unwrap_or(".");

            let account_root = PathBuf::from(export_base).join(&account);
            let notes_dir = PathBuf::from(notes_dir_str);
            let export_base_dir = PathBuf::from(export_base);

            if !account_root.exists() {
                anyhow::bail!("account root not found: {}", account_root.display());
            }

            println!("Repairing legacy attachments for {}", account);
            println!("  account root : {}", account_root.display());
            println!("  notes dir    : {}", notes_dir.display());
            println!(
                "  excluding    : {} (staging area)",
                export_base_dir.display()
            );
            if dry_run {
                println!("  (dry-run — no files will be changed)");
            }
            println!();

            let count = route::repair_legacy_attachments(
                &notes_dir,
                &account_root,
                Some(&export_base_dir),
                dry_run,
            )?;

            if dry_run {
                println!("\n{} attachment(s) would be repaired.", count);
            } else {
                println!("\n{} attachment(s) repaired.", count);
            }
        }

        Commands::Contextual { directory } => {
            #[cfg(all(windows, feature = "tray"))]
            detach_console();
            run_contextual_or_bail(directory)?
        }

        Commands::ContextualUri { uri } => {
            #[cfg(all(windows, feature = "tray"))]
            detach_console();
            let directory = email_to_markdown::shell_integration::local_path_from_uri(&uri)?;
            run_contextual_or_bail(directory)?;
        }

        Commands::Shell { action } => {
            use email_to_markdown::shell_integration as integration;
            let statuses = match action {
                ShellAction::Install => integration::install()?,
                ShellAction::Status => integration::status()?,
                ShellAction::Uninstall => integration::uninstall()?,
            };
            println!("{}", integration::format_status(&statuses));
        }

        #[cfg(feature = "tray")]
        Commands::Tray => {
            println!("Starting system tray application...");
            #[cfg(windows)]
            detach_console();
            tray::run_tray().context("Failed to run system tray")?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Cli, Commands};
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn contextual_command_accepts_one_native_path_argument() {
        for raw in [
            r"C:\Notes\Pro\Client",
            "/Users/alice/Notes/Client",
            "/home/alice/Notes/Client",
        ] {
            let cli = Cli::try_parse_from(["email-to-markdown", "contextual", raw]).unwrap();
            match cli.command {
                Commands::Contextual { directory } => assert_eq!(directory, PathBuf::from(raw)),
                _ => panic!("contextual command expected"),
            }
        }
        assert!(Cli::try_parse_from(["email-to-markdown", "contextual"]).is_err());
        assert!(Cli::try_parse_from(["email-to-markdown", "contextual", "one", "two"]).is_err());
    }

    #[test]
    fn shell_command_exposes_install_status_and_uninstall() {
        for action in ["install", "status", "uninstall"] {
            let cli = Cli::try_parse_from(["email-to-markdown", "shell", action]).unwrap();
            assert!(matches!(cli.command, Commands::Shell { .. }));
        }
    }
}
