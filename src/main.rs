use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

use email_to_markdown::config::{self, Config, SortConfig};
use email_to_markdown::email_export::ImapExporter;
use email_to_markdown::fix_yaml;
use email_to_markdown::sort_emails::EmailSorter;
use email_to_markdown::thunderbird;  // [1] Import Thunderbird

#[cfg(feature = "tray")]
use email_to_markdown::tray;

#[derive(Parser)]
#[command(name = "email-to-markdown")]
#[command(author = "FX Guillois")]
#[command(version = "0.1.0")]
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

    /// Fix malformed YAML in email files
    Fix {
        /// Directory containing email files to fix
        directory: PathBuf,

        /// Scan only, show what would be fixed
        #[arg(long)]
        dry_run: bool,

        /// Actually fix the files (default is dry-run)
        #[arg(long)]
        apply: bool,
    },

    /// Sort emails into categories (delete/summarize/keep)
    Sort {
        /// Directory containing email markdown files
        directory: Option<PathBuf>,

        /// Sort emails for a specific account from accounts.yaml
        #[arg(short, long)]
        account: Option<String>,

        /// Config file for sorting rules
        #[arg(short, long)]
        config: Option<PathBuf>,

        /// Output report file name
        #[arg(short, long, default_value = "sort_report.json")]
        report: String,

        /// Show detailed output
        #[arg(short, long)]
        verbose: bool,

        /// Simulate sorting without creating reports
        #[arg(long)]
        dry_run: bool,

        /// List available accounts from accounts.yaml
        #[arg(long)]
        list_accounts: bool,

        /// Create a default configuration file
        #[arg(long)]
        create_config: bool,
    },

    /// Run as system tray application (requires --features tray)
    #[cfg(feature = "tray")]
    Tray,
}

fn main() -> Result<()> {
    // Load .env from the platform config directory
    dotenv::from_path(config::env_file_path()).ok();

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
                        match exporter.export_account() {
                            Ok(results) => {
                                let total_exported: usize = results.values().map(|s| s.exported).sum();
                                let total_skipped: usize = results.values().map(|s| s.skipped).sum();
                                let total_errors: usize = results.values().map(|s| s.errors).sum();

                                println!(
                                    "\nExport completed for {}: {} exported, {} skipped, {} errors",
                                    account.name, total_exported, total_skipped, total_errors
                                );
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

        Commands::Fix {
            directory,
            dry_run,
            apply,
        } => {
            if !directory.exists() {
                println!("Directory not found: {}", directory.display());
                return Ok(());
            }

            println!("Scanning for malformed email files in: {}", directory.display());

            // Default to dry-run unless --apply is specified
            let is_dry_run = !apply || dry_run;

            let stats = fix_yaml::scan_and_fix_directory(&directory, is_dry_run)?;
            fix_yaml::print_summary(&stats, is_dry_run);
        }

        Commands::Sort {
            directory,
            account,
            config,
            report,
            verbose,
            dry_run,
            list_accounts,
            create_config,
        } => {
            if create_config {
                let config_path = config.unwrap_or_else(config::sort_config_path);
                let sort_config = SortConfig::default();
                sort_config.save(&config_path)?;
                println!("Configuration file created: {}", config_path.display());
                return Ok(());
            }

            if list_accounts {
                let accounts_config = Config::load(&config::accounts_yaml_path());
                if let Ok(cfg) = accounts_config {
                    println!("Available accounts from accounts.yaml:");
                    for (i, acc) in cfg.accounts.iter().enumerate() {
                        println!(
                            "   {}. {} -> {}",
                            i + 1,
                            acc.name,
                            acc.export_directory
                        );
                    }
                } else {
                    println!("No accounts found in accounts.yaml");
                }
                return Ok(());
            }

            // Determine directory to sort
            let sort_directory = if let Some(acc_name) = account {
                let accounts_config = Config::load(&config::accounts_yaml_path())
                    .context("Failed to load accounts configuration")?;

                let acc = accounts_config
                    .get_account(&acc_name)
                    .context(format!("Account '{}' not found", acc_name))?;

                println!("Sorting emails for account: {}", acc.name);
                PathBuf::from(&acc.export_directory)
            } else if let Some(dir) = directory {
                dir
            } else {
                println!("Please specify a directory or account");
                return Ok(());
            };

            // Load sort config
            let sort_config = SortConfig::load(&config.unwrap_or_else(config::sort_config_path))?;

            let mut sorter = EmailSorter::new(sort_directory, sort_config);

            if dry_run {
                println!("DRY RUN MODE: Analyzing emails without creating reports");
            }

            sorter.sort_emails()?;

            let sort_report = sorter.generate_report();

            if !dry_run {
                sorter.save_report(&sort_report, &report)?;
            } else {
                println!("DRY RUN: Would create report at: {}", report);
            }

            sorter.print_summary();

            if verbose {
                println!("\nDETAILED RESULTS:");
                for (category, emails) in sorter.categories() {
                    println!("\n{} ({} emails):", category.to_string().to_uppercase(), emails.len());
                    for email in emails.iter().take(5) {
                        println!(
                            "  - {} (from: {}, score: {})",
                            email.subject, email.sender, email.score
                        );
                    }
                    if emails.len() > 5 {
                        println!("  ... and {} more", emails.len() - 5);
                    }
                }
            }

            if dry_run {
                println!("\nDRY RUN COMPLETE");
                println!("No files were modified. To apply these changes, run without --dry-run");
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
