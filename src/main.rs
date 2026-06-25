use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

use email_to_markdown::config::{self, Config, Settings};
use email_to_markdown::email_export::ImapExporter;
use email_to_markdown::route;
use email_to_markdown::thunderbird;  // [1] Import Thunderbird

#[cfg(feature = "tray")]
use email_to_markdown::tray;

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

    /// Run as system tray application (requires --features tray)
    #[cfg(feature = "tray")]
    Tray,
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
                let profiles = thunderbird::list_profiles()
                    .context("Could not find Thunderbird profiles")?;

                // Prefer the marked default, but only if it has prefs.js (it may be an empty placeholder)
                let has_prefs = |p: &thunderbird::ThunderbirdProfile| p.path.join("prefs.js").exists();

                profiles
                    .iter()
                    .find(|p| p.is_default && has_prefs(p))
                    .or_else(|| profiles.iter().find(|p| has_prefs(p)))
                    .cloned()
                    .context("No usable Thunderbird profiles found (no prefs.js)")?
            };

            println!("Using Thunderbird profile: {} ({})", tb_profile.name, tb_profile.path.display());

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
                let env_template_path = output.parent().unwrap_or(Path::new(".")).join(".env.template");
                let env_content = thunderbird::generate_env_template(&accounts);
                std::fs::write(&env_template_path, &env_content)?;
                println!("Generated: {}", env_template_path.display());
                println!("\nRemember to:");
                println!("  1. Review and adjust accounts.yaml");
                println!("  2. Copy .env.template to {} and add passwords", config::env_file_path().display());
            } else if !extract_passwords {
                println!("\nRemember to add passwords to {}", config::env_file_path().display());
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
                            match thunderbird::write_passwords_to_env(&accounts, &passwords, &env_path) {
                                Ok(n) => println!("Written {} password(s) to {}", n, env_path.display()),
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
            let config = Config::load(&config_path)
                .context("Failed to load configuration")?;

            if list_accounts {
                println!("Available accounts from accounts.yaml:");
                for (i, acc) in config.accounts.iter().enumerate() {
                    println!(
                        "   {}. {} -> {}",
                        i + 1,
                        acc.name,
                        acc.export_directory
                    );
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
                println!("Add your IMAP accounts to {}", config::accounts_yaml_path().display());
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
                println!("\nProcessing account: {} -> {}", account.name, account.export_directory);

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
                                let total_exported: usize = results.values().map(|s| s.exported).sum();
                                let total_skipped: usize = results.values().map(|s| s.skipped).sum();
                                let total_errors: usize = results.values().map(|s| s.errors).sum();

                                println!(
                                    "\nExport completed for {}: {} exported, {} skipped, {} errors",
                                    account.name, total_exported, total_skipped, total_errors
                                );

                                // CLI mode (D8): apply routing decisions automatically, no review.
                                // Pipeline order: Export → route decisions accumulated above → apply now.
                                // IMAP deletion flags were set during Export; local .md files remain
                                // in staging until this apply step moves them into notes_dir.
                                let settings = Settings::load(&config::settings_path())
                                    .unwrap_or_default();
                                if let Some(notes_dir_str) = &settings.notes_dir {
                                    let notes_dir = PathBuf::from(notes_dir_str);
                                    let mut moved = 0usize;
                                    let mut apply_errors = 0usize;
                                    for (staging_path, decision) in &decisions {
                                        match route::apply_decision(staging_path, &decision.rel_path, &notes_dir) {
                                            Ok(()) => moved += 1,
                                            Err(e) => {
                                                apply_errors += 1;
                                                eprintln!(
                                                    "Warning: could not route {}: {:#}",
                                                    staging_path.display(), e
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

        #[cfg(feature = "tray")]
        Commands::Tray => {
            println!("Starting system tray application...");
            tray::run_tray().context("Failed to run system tray")?;
        }
    }

    Ok(())
}
