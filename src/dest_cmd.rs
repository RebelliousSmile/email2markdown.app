//! `dest` CLI subcommand: inspect and edit routing destinations without AI.
//!
//! - `list`  — human-readable dump of `destinations.yaml` + anomaly warnings.
//! - `add`   — upsert a path (bare = classification option) with optional rules.
//!
//! The interactive `suggest` command is wired in Part 3.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::config;
use crate::destinations::{self, DestinationEntry, DestinationRule, DestinationsConfig};
use crate::route;

#[derive(Args)]
pub struct DestArgs {
    /// Omit the subcommand to open the guided interactive editor.
    #[command(subcommand)]
    pub command: Option<DestCommand>,
}

#[derive(Subcommand)]
pub enum DestCommand {
    /// List current routing destinations
    List,

    /// Add a destination path, optionally with routing rule(s)
    Add {
        /// Relative path under notes_dir (e.g. Perso/Banque)
        path: String,

        /// Route emails from this sender domain here
        #[arg(long)]
        domain: Option<String>,

        /// Route emails from this exact address here
        #[arg(long)]
        from: Option<String>,

        /// Route emails whose subject contains this keyword here
        #[arg(long)]
        subject: Option<String>,

        /// Route emails received on this IMAP account here
        #[arg(long)]
        account: Option<String>,

        /// Human note describing this destination
        #[arg(long)]
        note: Option<String>,

        /// Mark this as the fallback destination
        #[arg(long)]
        default: bool,
    },

    /// Scan the default folder and interactively suggest domain rules
    Suggest,

    /// Open the destinations management GUI window (requires --features tray)
    #[cfg(feature = "tray")]
    Gui,
}

/// Entry point dispatched from `main`.
pub fn run(args: DestArgs) -> Result<()> {
    let dest_file = route::destinations_path();

    let Some(command) = args.command else {
        return interactive(&dest_file);
    };

    match command {
        DestCommand::List => {
            let cfg = destinations::load_yaml(&dest_file)
                .with_context(|| format!("failed to load {}", dest_file.display()))?;
            if cfg.destinations.is_empty() {
                println!("No destinations configured ({}).", dest_file.display());
            } else {
                for entry in &cfg.destinations {
                    println!("{}", format_entry(entry));
                }
            }
            for warning in detect_anomalies(&cfg) {
                eprintln!("warning: {warning}");
            }
        }

        DestCommand::Add {
            path,
            domain,
            from,
            subject,
            account,
            note,
            default,
        } => {
            let mut rules = Vec::new();
            if let Some(d) = domain {
                // Lowercase the domain — parity with migration/tray and the
                // case-insensitive comparison in `route_email`.
                rules.push(DestinationRule::Domain(d.to_lowercase()));
            }
            if let Some(a) = from {
                rules.push(DestinationRule::From(a));
            }
            if let Some(s) = subject {
                rules.push(DestinationRule::Subject(s));
            }
            if let Some(a) = account {
                rules.push(DestinationRule::Account(a));
            }

            add_entry(&dest_file, &path, &rules, note.as_deref(), default)?;

            if rules.is_empty() {
                println!("Added \"{path}\" (classification option).");
            } else {
                println!("Added/updated \"{path}\" with {} rule(s).", rules.len());
            }
        }

        DestCommand::Suggest => suggest(&dest_file)?,

        #[cfg(feature = "tray")]
        DestCommand::Gui => {
            println!("Ouverture de la fenêtre destinations…");
            if let Err(e) = crate::tray::send_command(crate::tray::AppCommand::OpenDestGui { dest_file }) {
                eprintln!("Erreur : impossible d'ouvrir la fenêtre (tray non démarré ?) — {:#}", e);
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

// ── interactive editor ────────────────────────────────────────────────────────

/// Read one trimmed line from stdin. `Ok(None)` signals EOF.
fn prompt_line(prompt: &str) -> Result<Option<String>> {
    print!("{prompt}");
    io::stdout().flush().ok();
    let mut line = String::new();
    if io::stdin().read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_string()))
}

/// Print the filtered, 1-based numbered list of entries.
fn print_list(cfg: &DestinationsConfig, indices: &[usize]) {
    if indices.is_empty() {
        println!("No entries match — press f to change filter or a to add.");
        return;
    }
    for (n, &i) in indices.iter().enumerate() {
        println!("{:>3}. {}", n + 1, format_entry(&cfg.destinations[i]));
    }
}

/// Resolve a visible 1-based number to a real index into `cfg.destinations`.
fn resolve_index(indices: &[usize], token: &str) -> Option<usize> {
    let n: usize = token.parse().ok()?;
    if n >= 1 && n <= indices.len() {
        Some(indices[n - 1])
    } else {
        None
    }
}

/// Prompt for a rule kind + value. Returns `Ok(None)` on EOF or empty/`none` kind.
fn prompt_rule(kinds: &str) -> Result<Option<DestinationRule>> {
    let Some(kind) = prompt_line(&format!("Type [{kinds}]: "))? else {
        return Ok(None);
    };
    let kind = kind.to_lowercase();
    if kind.is_empty() || kind == "none" {
        return Ok(None);
    }
    let Some(value) = prompt_line("Valeur: ")? else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    let rule = match kind.as_str() {
        "domain" => DestinationRule::Domain(value.to_lowercase()),
        "from" => DestinationRule::From(value),
        "subject" => DestinationRule::Subject(value),
        "account" => DestinationRule::Account(value),
        _ => {
            eprintln!("  unknown rule type: {kind}");
            return Ok(None);
        }
    };
    Ok(Some(rule))
}

/// Guided interactive editor for `destinations.yaml`.
///
/// Outer loop prompts a filter; inner loop dispatches single-key actions.
/// Every mutating action saves immediately, then recomputes the filtered view.
fn interactive(dest_file: &Path) -> Result<()> {
    let mut cfg = destinations::load_yaml(dest_file)
        .with_context(|| format!("failed to load {}", dest_file.display()))?;

    println!("Editing {} ({} entries).", dest_file.display(), cfg.destinations.len());

    'outer: loop {
        let filter = match prompt_line("\nFilter [Enter=all]: ")? {
            None => return Ok(()), // EOF
            Some(f) => f,
        };
        let mut indices = filter_entries(&cfg, &filter);
        print_list(&cfg, &indices);

        // Inner action loop.
        loop {
            let Some(action) = prompt_line("[a]jouter [e]N [s]N [r]N [d]N [f]iltrer [q]uitter > ")?
            else {
                return Ok(()); // EOF
            };
            let mut parts = action.split_whitespace();
            let verb = parts.next().unwrap_or("");
            let arg = parts.next().unwrap_or("");

            match verb {
                "" => continue,
                "q" => return Ok(()),
                "f" => continue 'outer,

                "a" => {
                    if action_add(&mut cfg)? {
                        save_and_reprint(dest_file, &cfg, &filter, &mut indices)?;
                    }
                }

                "e" => match resolve_index(&indices, arg) {
                    Some(i) => {
                        let path = cfg.destinations[i].path.clone();
                        let note = prompt_line("New note (Enter to clear): ")?.unwrap_or_default();
                        let note = (!note.is_empty()).then_some(note);
                        destinations::set_note(&mut cfg, &path, note);
                        save_and_reprint(dest_file, &cfg, &filter, &mut indices)?;
                    }
                    None => eprintln!("Index invalide."),
                },

                "s" => match resolve_index(&indices, arg) {
                    Some(i) => {
                        let path = cfg.destinations[i].path.clone();
                        println!("{}", format_entry(&cfg.destinations[i]));
                        let confirm = prompt_line("Delete? [y/N]: ")?.unwrap_or_default();
                        if confirm.eq_ignore_ascii_case("y") {
                            destinations::remove_entry(&mut cfg, &path);
                            save_and_reprint(dest_file, &cfg, &filter, &mut indices)?;
                        }
                    }
                    None => eprintln!("Index invalide."),
                },

                "d" => match resolve_index(&indices, arg) {
                    Some(i) => {
                        let path = cfg.destinations[i].path.clone();
                        let prev = cfg
                            .destinations
                            .iter()
                            .find(|e| e.default && !e.path.eq_ignore_ascii_case(&path))
                            .map(|e| e.path.clone());
                        destinations::set_default(&mut cfg, &path);
                        if let Some(prev) = prev {
                            println!("Cleared previous default: {prev}");
                        }
                        save_and_reprint(dest_file, &cfg, &filter, &mut indices)?;
                    }
                    None => eprintln!("Index invalide."),
                },

                "r" => match resolve_index(&indices, arg) {
                    Some(i) => {
                        if action_rules(&mut cfg, i)? {
                            save_and_reprint(dest_file, &cfg, &filter, &mut indices)?;
                        }
                    }
                    None => eprintln!("Index invalide."),
                },

                _ => eprintln!("Action inconnue."),
            }
        }
    }
}

/// Save the config and reprint the freshly recomputed filtered list.
fn save_and_reprint(
    dest_file: &Path,
    cfg: &DestinationsConfig,
    filter: &str,
    indices: &mut Vec<usize>,
) -> Result<()> {
    destinations::save_yaml(dest_file, cfg)
        .with_context(|| format!("failed to save {}", dest_file.display()))?;
    *indices = filter_entries(cfg, filter);
    print_list(cfg, indices);
    Ok(())
}

/// `a` action: prompt for a new entry (path + optional rule + optional note).
/// Returns `true` if the config was mutated (and should be saved).
fn action_add(cfg: &mut DestinationsConfig) -> Result<bool> {
    let Some(path) = prompt_line("Path: ")? else {
        return Ok(false);
    };
    if path.is_empty() {
        return Ok(false);
    }
    if route::join_safe_segments(Path::new(""), &path).is_err() {
        eprintln!("  invalid path.");
        return Ok(false);
    }

    let rules: Vec<DestinationRule> = match prompt_rule("domain/from/subject/account/none")? {
        Some(rule) => vec![rule],
        None => vec![],
    };

    destinations::upsert_entry(cfg, &path, &rules);

    if let Some(note) = prompt_line("Note (optional): ")? {
        if !note.is_empty() {
            destinations::set_note(cfg, &path, Some(note));
        }
    }

    println!("Added \"{path}\".");
    Ok(true)
}

/// `r N` action: add or remove a rule on entry `i`. Returns `true` if mutated.
fn action_rules(cfg: &mut DestinationsConfig, i: usize) -> Result<bool> {
    let path = cfg.destinations[i].path.clone();
    loop {
        let rules = cfg.destinations[i].rules.clone();
        if rules.is_empty() {
            println!("(no rules)");
        } else {
            for (k, rule) in rules.iter().enumerate() {
                println!("{:>3}. {}", k + 1, rule_label(rule));
            }
        }
        let Some(action) = prompt_line("[a]jouter [r]N supprimer [q]retour > ")? else {
            return Ok(false);
        };
        let mut parts = action.split_whitespace();
        let verb = parts.next().unwrap_or("");
        let arg = parts.next().unwrap_or("");

        match verb {
            "q" | "" => return Ok(false),
            "a" => {
                if let Some(rule) = prompt_rule("domain/from/subject/account")? {
                    destinations::upsert_entry(cfg, &path, &[rule]);
                    return Ok(true);
                }
            }
            "r" => {
                let k: usize = arg.parse().unwrap_or(0);
                if k >= 1 && k <= rules.len() {
                    destinations::remove_rule(cfg, &path, &rules[k - 1]);
                    return Ok(true);
                }
                eprintln!("Index invalide.");
            }
            _ => eprintln!("Action inconnue."),
        }
    }
}

/// One-line label for a rule (reuses the `format_entry` tag style).
fn rule_label(rule: &DestinationRule) -> String {
    match rule {
        DestinationRule::Domain(d) => format!("domain:{d}"),
        DestinationRule::From(a) => format!("from:{a}"),
        DestinationRule::Subject(k) => format!("subject:{k}"),
        DestinationRule::Account(n) => format!("account:{n}"),
    }
}

// ── suggest ───────────────────────────────────────────────────────────────────

/// Interactive: scan the default folder, group emails by sender domain, and let
/// the user assign a destination to each uncovered domain.
fn suggest(dest_file: &Path) -> Result<()> {
    let settings = config::Settings::load(&config::settings_path()).unwrap_or_default();
    let mut cfg = destinations::load_yaml(dest_file)
        .with_context(|| format!("failed to load {}", dest_file.display()))?;

    let scan_root = resolve_scan_root(&settings, &cfg)?;
    println!("Scanning {} ...", scan_root.display());

    let groups = scan_domains(&scan_root)?;
    let candidates = uncovered_domains(groups, &cfg);
    if candidates.is_empty() {
        println!("Nothing to suggest — no uncovered sender domains found.");
        return Ok(());
    }

    let stdin = io::stdin();
    let mut added = 0usize;
    let mut skipped = 0usize;

    for (domain, count) in candidates {
        loop {
            println!("\n{domain}  ({count} email(s))");
            print!("-> destination (Enter to skip, - to ignore): ");
            io::stdout().flush().ok();

            let mut line = String::new();
            if stdin.read_line(&mut line)? == 0 {
                // EOF (e.g. piped/non-interactive): stop the loop.
                println!();
                return finish_suggest(dest_file, &cfg, added, skipped);
            }
            let input = line.trim();

            // Empty or `-` → in-session skip (not persisted).
            if input.is_empty() || input == "-" {
                skipped += 1;
                break;
            }

            // Validate the typed path (anti-traversal) before accepting it.
            if route::join_safe_segments(Path::new(""), input).is_err() {
                eprintln!("  invalid path, try again");
                continue;
            }

            let path = strip_trailing_year_month(input);
            destinations::upsert_entry(&mut cfg, &path, &[DestinationRule::Domain(domain.clone())]);
            added += 1;
            break;
        }
    }

    finish_suggest(dest_file, &cfg, added, skipped)
}

/// Save (only if anything changed) and print the run summary.
fn finish_suggest(
    dest_file: &Path,
    cfg: &DestinationsConfig,
    added: usize,
    skipped: usize,
) -> Result<()> {
    if added > 0 {
        destinations::save_yaml(dest_file, cfg)
            .with_context(|| format!("failed to save {}", dest_file.display()))?;
    }
    println!("{added} rule(s) added, {skipped} domain(s) skipped.");
    Ok(())
}

/// Resolve the folder to scan: `notes_dir / <default-destination-path>`.
pub fn resolve_scan_root(settings: &config::Settings, cfg: &DestinationsConfig) -> Result<PathBuf> {
    let notes_dir = settings
        .notes_dir
        .as_deref()
        .map(PathBuf::from)
        .context("notes_dir not set in settings.yaml — set it before running `dest suggest`")?;
    let default_sub = cfg
        .destinations
        .iter()
        .find(|e| e.default)
        .map(|e| e.path.clone())
        .unwrap_or_else(|| route::DEFAULT_BASE.to_string());
    Ok(notes_dir.join(default_sub))
}

/// Walk `root` and count emails per sender domain.
///
/// Excludes any entry whose name starts with `.` or `_`, never follows symlinks
/// (rule 02-rust-filesystem-safety), and is depth-bounded as a runaway guard.
pub fn scan_domains(root: &Path) -> Result<HashMap<String, usize>> {
    let mut files = Vec::new();
    if root.exists() {
        walk_md(root, 0, 6, &mut files)?;
    }

    let mut groups: HashMap<String, usize> = HashMap::new();
    for file in files {
        let Ok(content) = std::fs::read_to_string(&file) else {
            continue;
        };
        if let Some(from) = parse_from(&content) {
            if let Some(domain) = extract_domain(&from) {
                *groups.entry(domain).or_insert(0) += 1;
            }
        }
    }
    Ok(groups)
}

/// Recursively collect `.md` files, honoring the exclusion and symlink rules.
fn walk_md(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<PathBuf>) -> Result<()> {
    if depth > max_depth {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("failed to read {}", dir.display()))?;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Skip internal/technical dirs and dotfiles.
        if name.starts_with('.') || name.starts_with('_') {
            continue;
        }
        let file_type = entry.file_type()?;
        // Never follow symlinks (check before is_dir/is_file).
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            walk_md(&path, depth + 1, max_depth, out)?;
        } else if file_type.is_file()
            && path.extension().and_then(|e| e.to_str()) == Some("md")
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Extract the `from:` value from a `.md`'s YAML frontmatter, if present.
fn parse_from(content: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct FromHead {
        #[serde(default)]
        from: String,
    }
    let rest = content.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let frontmatter = &rest[..end];
    serde_yaml::from_str::<FromHead>(frontmatter)
        .ok()
        .map(|h| h.from)
        .filter(|s| !s.is_empty())
}

/// Extract the lowercase domain from a `From:` value.
///
/// Handles bare `alice@ubs.ch` and display forms `Alice <alice@ubs.ch>`.
pub fn extract_domain(from: &str) -> Option<String> {
    let at = from.rfind('@')?;
    let domain: String = from[at + 1..]
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, '>' | ',' | ';'))
        .collect();
    let domain = domain.trim().to_lowercase();
    (!domain.is_empty()).then_some(domain)
}

/// Drop domains already covered by an existing `Domain` rule; sort by count desc.
pub fn uncovered_domains(
    groups: HashMap<String, usize>,
    cfg: &DestinationsConfig,
) -> Vec<(String, usize)> {
    let covered: std::collections::HashSet<String> = cfg
        .destinations
        .iter()
        .flat_map(|e| e.rules.iter())
        .filter_map(|r| match r {
            DestinationRule::Domain(d) => Some(d.to_lowercase()),
            _ => None,
        })
        .collect();

    let mut out: Vec<(String, usize)> = groups
        .into_iter()
        .filter(|(d, _)| !covered.contains(d))
        .collect();
    // Count desc, then domain asc for determinism.
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// Strip a trailing `<Year>/<Month>` from a user-typed path (the router re-appends it).
pub fn strip_trailing_year_month(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if route::ends_with_year_month(trimmed) {
        let segs: Vec<&str> = trimmed.split('/').collect();
        segs[..segs.len() - 2].join("/")
    } else {
        trimmed.to_string()
    }
}

/// Upsert a destination into `dest_file`: validate the path, apply rules, note and
/// the `default` flag, then save.
///
/// # Errors
/// - Path fails the anti-traversal check (`join_safe_segments`).
/// - `make_default` is set while another entry already holds the default flag.
/// - I/O / serialization failure.
pub fn add_entry(
    dest_file: &Path,
    path: &str,
    rules: &[DestinationRule],
    note: Option<&str>,
    make_default: bool,
) -> Result<()> {
    // Validate the path shape (anti-traversal); the joined value is discarded.
    route::join_safe_segments(Path::new(""), path)
        .with_context(|| format!("invalid destination path {path:?}"))?;

    let mut cfg = destinations::load_yaml(dest_file)
        .with_context(|| format!("failed to load {}", dest_file.display()))?;

    // Guard: only one `default` entry allowed.
    if make_default {
        if let Some(existing) = cfg
            .destinations
            .iter()
            .find(|e| e.default && !e.path.eq_ignore_ascii_case(path))
        {
            anyhow::bail!("a default destination already exists: {}", existing.path);
        }
    }

    destinations::upsert_entry(&mut cfg, path, rules);

    // Apply note / default on the now-present entry.
    if let Some(entry) = cfg
        .destinations
        .iter_mut()
        .find(|e| e.path.eq_ignore_ascii_case(path))
    {
        if let Some(n) = note {
            entry.note = Some(n.to_string());
        }
        if make_default {
            entry.default = true;
        }
    }

    destinations::save_yaml(dest_file, &cfg)
        .with_context(|| format!("failed to save {}", dest_file.display()))
}

/// Indices into `cfg.destinations` whose `path` contains `query` (case-insensitive).
/// An empty/whitespace query returns every index.
pub fn filter_entries(cfg: &DestinationsConfig, query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    cfg.destinations
        .iter()
        .enumerate()
        .filter(|(_, e)| q.is_empty() || e.path.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

/// Format one entry for `list` output: `path  [rules]  — note`.
fn format_entry(entry: &DestinationEntry) -> String {
    let mut tags: Vec<String> = entry.rules.iter().map(rule_label).collect();
    if entry.default {
        tags.push("default".to_string());
    }

    let mut line = entry.path.clone();
    if tags.is_empty() {
        line.push_str("  (no rules — manual classification)");
    } else {
        line.push_str(&format!("  [{}]", tags.join(", ")));
    }
    if let Some(note) = &entry.note {
        line.push_str(&format!("  — {note}"));
    }
    line
}

/// Detect anomalies surfaced as warnings by `list`.
pub fn detect_anomalies(cfg: &DestinationsConfig) -> Vec<String> {
    let mut warnings = Vec::new();

    let default_count = cfg.destinations.iter().filter(|e| e.default).count();
    if default_count > 1 {
        warnings.push(format!(
            "{default_count} entries marked `default` — only one is allowed"
        ));
    }

    if cfg.destinations.iter().any(|e| e.path.trim().is_empty()) {
        warnings.push("an entry has an empty path".to_string());
    }

    warnings
}
