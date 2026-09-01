//! Contextual IMAP search contracts and pure transformations.
//!
//! This module deliberately has no dependency on the tray/WebView or on an OS
//! shell integration. Network orchestration lives on `ImapExporter`, while the
//! query construction, header parsing and logical-message merging stay testable.

use crate::route::{normalize_address, MatchRule};
use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset};
use fs2::FileExt;
use imap_proto::{AttributeValue, Response};
use mailparse::MailHeaderMap;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

pub const SEARCH_RULE_BATCH: usize = 50;
pub const HEADER_UID_BATCH: usize = 500;
pub const HEADER_FETCH_FIELDS: &str =
    "BODY.PEEK[HEADER.FIELDS (MESSAGE-ID DATE FROM TO CC BCC SUBJECT)]";

static HEADER_ADDRESS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"#)
        .expect("static header address regex")
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageLocation {
    pub folder_raw: String,
    pub folder_display: String,
    pub uid_validity: u32,
    pub uid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceIdentity {
    pub account: String,
    pub message_id: Option<String>,
    pub header_fingerprint: String,
    pub provider_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalSourceProof {
    pub identity: SourceIdentity,
    pub location: MessageLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextualCandidate {
    pub source: SourceIdentity,
    pub locations: Vec<MessageLocation>,
    pub date: Option<DateTime<FixedOffset>>,
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
}

impl ContextualCandidate {
    pub fn logical_key(&self) -> String {
        if let Some(provider) = &self.source.provider_id {
            return format!("provider:{}:{}", self.source.account, provider);
        }
        if let Some(message_id) = &self.source.message_id {
            return format!(
                "message:{}:{}:{}",
                self.source.account, message_id, self.source.header_fingerprint
            );
        }
        let location = &self.locations[0];
        format!(
            "location:{}:{}:{}:{}",
            self.source.account, location.folder_raw, location.uid_validity, location.uid
        )
    }
}

/// Build bounded server-side prefilters. Every batch contains at most 50
/// configured address rules, regardless of how many IMAP fields a correspondent
/// expands to.
pub fn build_search_batches(rules: &[MatchRule]) -> Result<Vec<String>> {
    let mut unique = BTreeSet::new();
    for rule in rules {
        let key = match rule {
            MatchRule::Correspondent(address) => {
                format!("correspondent:{}", normalize_address(address)?)
            }
            MatchRule::From(address) => format!("from:{}", normalize_address(address)?),
            MatchRule::Domain(domain) if !domain.trim().is_empty() => {
                format!("domain:{}", domain.trim().to_lowercase())
            }
            _ => continue,
        };
        unique.insert(key);
    }
    if unique.is_empty() {
        anyhow::bail!("contextual search has no usable address rule");
    }

    unique
        .into_iter()
        .collect::<Vec<_>>()
        .chunks(SEARCH_RULE_BATCH)
        .map(|chunk| {
            let mut atoms = Vec::new();
            for key in chunk {
                let (kind, value) = key.split_once(':').expect("internal rule key");
                let quoted = quote_imap(value)?;
                match kind {
                    "correspondent" => {
                        atoms.push(format!("FROM {quoted}"));
                        atoms.push(format!("TO {quoted}"));
                        atoms.push(format!("CC {quoted}"));
                        atoms.push(format!("BCC {quoted}"));
                    }
                    "from" | "domain" => atoms.push(format!("FROM {quoted}")),
                    _ => unreachable!("known contextual rule"),
                }
            }
            Ok(format!("UNDELETED {}", fold_or(&atoms)))
        })
        .collect()
}

fn quote_imap(value: &str) -> Result<String> {
    if value.chars().any(|c| matches!(c, '\r' | '\n' | '\0')) {
        anyhow::bail!("unsafe character in IMAP search value");
    }
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn fold_or(atoms: &[String]) -> String {
    match atoms {
        [] => String::new(),
        [only] => only.clone(),
        [first, rest @ ..] => format!("OR {} {}", first, fold_or(rest)),
    }
}

pub fn parse_header_candidate(
    account: &str,
    location: MessageLocation,
    header: &[u8],
    provider_id: Option<String>,
) -> Result<ContextualCandidate> {
    let mail = mailparse::parse_mail(header).context("parse contextual email headers")?;
    let get = |name| mail.headers.get_first_value(name).unwrap_or_default();
    let message_id = normalize_message_id(&get("Message-ID"));
    let date_raw = get("Date");
    let from_raw = get("From");
    let to_raw = get("To");
    let cc_raw = get("Cc");
    let bcc_raw = get("Bcc");
    let subject = get("Subject");
    let from = extract_addresses(&from_raw);
    let to = extract_addresses(&to_raw);
    let cc = extract_addresses(&cc_raw);
    let bcc = extract_addresses(&bcc_raw);
    let normalized_headers = format!(
        "id={}|date={}|from={}|to={}|cc={}|bcc={}|subject={}",
        message_id.as_deref().unwrap_or_default(),
        normalize_header(&date_raw),
        from.join(","),
        to.join(","),
        cc.join(","),
        bcc.join(","),
        normalize_header(&subject)
    );
    let header_fingerprint = format!("{:x}", md5::compute(normalized_headers.as_bytes()));

    Ok(ContextualCandidate {
        source: SourceIdentity {
            account: account.to_string(),
            message_id,
            header_fingerprint,
            provider_id,
        },
        locations: vec![location],
        date: mailparse::dateparse(&date_raw)
            .ok()
            .and_then(|timestamp| DateTime::from_timestamp(timestamp, 0))
            .map(|date| date.fixed_offset()),
        from,
        to,
        cc,
        bcc,
        subject,
    })
}

fn normalize_message_id(value: &str) -> Option<String> {
    let value = value.trim().trim_matches(['<', '>']).to_lowercase();
    (!value.is_empty()).then_some(value)
}

fn normalize_header(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn extract_addresses(value: &str) -> Vec<String> {
    let mut addresses: Vec<String> = HEADER_ADDRESS_RE
        .find_iter(value)
        .filter_map(|m| normalize_address(m.as_str()).ok())
        .collect();
    addresses.sort();
    addresses.dedup();
    addresses
}

pub fn candidate_matches_rules(candidate: &ContextualCandidate, rules: &[MatchRule]) -> bool {
    rules.iter().any(|rule| match rule {
        MatchRule::Correspondent(address) => normalize_address(address).ok().map_or(false, |wanted| {
            candidate
                .from
                .iter()
                .chain(candidate.to.iter())
                .chain(candidate.cc.iter())
                .chain(candidate.bcc.iter())
                .any(|actual| actual == &wanted)
        }),
        MatchRule::From(address) => normalize_address(address).ok().map_or(false, |wanted| {
            candidate.from.iter().any(|actual| actual == &wanted)
        }),
        MatchRule::Domain(domain) => {
            let domain = domain.trim().to_lowercase();
            candidate.from.iter().any(|address| {
                address.rsplit_once('@').map_or(false, |(_, actual)| {
                    actual == domain || actual.ends_with(&format!(".{domain}"))
                })
            })
        }
        MatchRule::Subject(_) | MatchRule::Account(_) => false,
    })
}

/// Guard used immediately before a later UID operation. A changed UIDVALIDITY
/// invalidates every UID captured by the earlier search.
pub fn validate_uidvalidity(location: &MessageLocation, current: Option<u32>) -> Result<()> {
    if current != Some(location.uid_validity) {
        anyhow::bail!(
            "stale contextual result for {}: expected UIDVALIDITY {}, got {:?}",
            location.folder_display,
            location.uid_validity,
            current
        );
    }
    Ok(())
}

pub fn merge_candidates(candidates: Vec<ContextualCandidate>) -> Vec<ContextualCandidate> {
    let mut merged: HashMap<String, ContextualCandidate> = HashMap::new();
    for mut candidate in candidates {
        let key = candidate.logical_key();
        if let Some(existing) = merged.get_mut(&key) {
            existing.locations.append(&mut candidate.locations);
            existing.locations.sort_by(|a, b| {
                (&a.folder_raw, a.uid_validity, a.uid).cmp(&(&b.folder_raw, b.uid_validity, b.uid))
            });
            existing.locations.dedup();
        } else {
            merged.insert(key, candidate);
        }
    }
    let mut result: Vec<_> = merged.into_values().collect();
    result.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.logical_key().cmp(&b.logical_key())));
    result
}

/// Parse a raw low-level UID FETCH response, including Gmail's provider id.
pub fn parse_uid_fetch_response(raw: &[u8]) -> Result<Vec<(u32, Option<String>, Vec<u8>)>> {
    let mut remaining = raw;
    let mut result = Vec::new();
    while !remaining.is_empty() {
        let (rest, response) = Response::from_bytes(remaining)
            .map_err(|e| anyhow::anyhow!("parse IMAP FETCH response: {e:?}"))?;
        remaining = rest;
        let Response::Fetch(_, attributes) = response else {
            continue;
        };
        let mut uid = None;
        let mut provider = None;
        let mut header = None;
        for attribute in attributes {
            match attribute {
                AttributeValue::Uid(value) => uid = Some(value),
                AttributeValue::GmailMsgId(value) => provider = Some(value.to_string()),
                AttributeValue::BodySection { data: Some(value), .. } => {
                    header = Some(value.into_owned())
                }
                AttributeValue::Rfc822Header(Some(value)) => header = Some(value.into_owned()),
                _ => {}
            }
        }
        if let (Some(uid), Some(header)) = (uid, header) {
            result.push((uid, provider, header));
        }
    }
    Ok(result)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversionStatus {
    Written { markdown: PathBuf, proof: LocalSourceProof },
    AlreadyPresent { markdown: PathBuf, proof: LocalSourceProof },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageConversionResult {
    pub candidate_key: String,
    pub status: Option<ConversionStatus>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeletionProvider {
    None,
    GenericUidPlus,
    Gmail { trash_folder: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletionPreflight {
    pub required: bool,
    pub supported: bool,
    pub provider: DeletionProvider,
    pub reason: Option<String>,
}

pub fn evaluate_deletion_preflight(
    required: bool,
    gmail: bool,
    capabilities: &[String],
    trash_folder: Option<String>,
) -> DeletionPreflight {
    if !required {
        return DeletionPreflight {
            required: false,
            supported: true,
            provider: DeletionProvider::None,
            reason: None,
        };
    }
    let has = |value: &str| capabilities.iter().any(|cap| cap.eq_ignore_ascii_case(value));
    if gmail {
        let missing: Vec<&str> = ["X-GM-EXT-1", "UIDPLUS", "MOVE"]
            .into_iter()
            .filter(|capability| !has(capability))
            .collect();
        if !missing.is_empty() {
            return DeletionPreflight {
                required: true,
                supported: false,
                provider: DeletionProvider::None,
                reason: Some(format!("missing Gmail capabilities: {}", missing.join(", "))),
            };
        }
        let Some(trash_folder) = trash_folder else {
            return DeletionPreflight {
                required: true,
                supported: false,
                provider: DeletionProvider::None,
                reason: Some("Gmail Trash mailbox with SPECIAL-USE is missing".into()),
            };
        };
        DeletionPreflight {
            required: true,
            supported: true,
            provider: DeletionProvider::Gmail { trash_folder },
            reason: None,
        }
    } else if has("UIDPLUS") {
        DeletionPreflight {
            required: true,
            supported: true,
            provider: DeletionProvider::GenericUidPlus,
            reason: None,
        }
    } else {
        DeletionPreflight {
            required: true,
            supported: false,
            provider: DeletionProvider::None,
            reason: Some("UIDPLUS is required for targeted UID EXPUNGE".into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletionRequest {
    pub markdown: PathBuf,
    pub proof: LocalSourceProof,
}

pub fn build_deletion_batch(results: &[MessageConversionResult]) -> Vec<DeletionRequest> {
    results
        .iter()
        .filter_map(|result| match &result.status {
            Some(ConversionStatus::Written { markdown, proof })
            | Some(ConversionStatus::AlreadyPresent { markdown, proof })
                if proof_is_complete(proof)
                    && markdown.is_file()
                    && read_source_proof(markdown).ok().flatten().as_ref() == Some(proof) =>
            {
                Some(DeletionRequest {
                    markdown: markdown.clone(),
                    proof: proof.clone(),
                })
            }
            _ => None,
        })
        .collect()
}

fn proof_is_complete(proof: &LocalSourceProof) -> bool {
    !proof.identity.account.trim().is_empty()
        && !proof.identity.header_fingerprint.trim().is_empty()
        && proof.location.uid > 0
        && proof.location.uid_validity > 0
        && (proof.identity.provider_id.is_some() || proof.identity.message_id.is_some())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeletionOutcome {
    Deleted,
    AlreadyAbsent,
    RetryRequired(String),
    StaleUidValidity { expected: u32, actual: Option<u32> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageDeletionResult {
    pub proof: LocalSourceProof,
    pub outcome: DeletionOutcome,
}

/// Exclusive lock for one target/account pair. The OS releases the lock even
/// after a crash; the file content remains useful for diagnostics until the
/// owning process exits normally.
pub struct ContextualLock {
    file: File,
    path: PathBuf,
}

impl ContextualLock {
    pub fn acquire(target: &Path, account: &str) -> Result<Self> {
        let target_meta = fs::symlink_metadata(target)
            .with_context(|| format!("inspect contextual target {}", target.display()))?;
        if target_meta.file_type().is_symlink() || !target_meta.is_dir() {
            anyhow::bail!("contextual target must be an existing non-symlink directory");
        }
        let account_key = format!("{:x}", md5::compute(account.as_bytes()));
        let path = target.join(format!(".email-to-markdown-{account_key}.lock"));
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("open contextual lock {}", path.display()))?;
        file.try_lock_exclusive().with_context(|| {
            format!("another contextual export is active for account {account}")
        })?;
        let started = SystemTime::now();
        file.set_len(0)?;
        writeln!(
            file,
            "pid={}\nstarted_unix={}",
            std::process::id(),
            started.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
        )?;
        file.sync_all()?;
        cleanup_stale_staging(target, started)?;
        Ok(Self { file, path })
    }
}

impl Drop for ContextualLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
        let _ = fs::remove_file(&self.path);
    }
}

/// Delete only application-owned staging directories older than the active
/// lock. Arbitrary files and newer work are never considered residues.
pub fn cleanup_stale_staging(target: &Path, active_since: SystemTime) -> Result<usize> {
    let mut removed = 0;
    for entry in fs::read_dir(target)? {
        let entry = entry?;
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(".email-to-markdown-tmp-") {
            continue;
        }
        let metadata = entry.metadata()?;
        if metadata.is_dir() && metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH) < active_since {
            fs::remove_dir_all(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

pub fn proof_matches_candidate(proof: &LocalSourceProof, candidate: &ContextualCandidate) -> bool {
    if proof.identity.account != candidate.source.account {
        return false;
    }
    if let (Some(left), Some(right)) = (
        proof.identity.provider_id.as_deref(),
        candidate.source.provider_id.as_deref(),
    ) {
        return left == right;
    }
    if candidate.locations.iter().any(|location| location == &proof.location) {
        return true;
    }
    proof.identity.message_id.is_some()
        && proof.identity.message_id == candidate.source.message_id
        && proof.identity.header_fingerprint == candidate.source.header_fingerprint
}

pub fn read_source_proof(path: &Path) -> Result<Option<LocalSourceProof>> {
    #[derive(Deserialize)]
    struct ProofHead {
        #[serde(default)]
        source: Option<LocalSourceProof>,
    }
    let content = fs::read_to_string(path)?;
    let Some(rest) = content.strip_prefix("---\n") else {
        return Ok(None);
    };
    let Some(end) = rest.find("\n---") else {
        return Ok(None);
    };
    Ok(serde_yaml::from_str::<ProofHead>(&rest[..end])
        .ok()
        .and_then(|head| head.source))
}

pub fn find_existing_proof(
    target: &Path,
    candidate: &ContextualCandidate,
) -> Result<Option<(PathBuf, LocalSourceProof)>> {
    for entry in fs::read_dir(target)? {
        let entry = entry?;
        let metadata = entry.file_type()?;
        let path = entry.path();
        if metadata.is_symlink()
            || !metadata.is_file()
            || path.extension().and_then(|value| value.to_str()) != Some("md")
        {
            continue;
        }
        if let Some(proof) = read_source_proof(&path)? {
            if proof_matches_candidate(&proof, candidate) {
                return Ok(Some((path, proof)));
            }
        }
    }
    Ok(None)
}

/// Convert one fetched RFC822 message through the existing converter, then
/// atomically commit its output into the exact contextual target.
pub fn convert_raw_contextual(
    target: &Path,
    account: &crate::config::Account,
    candidate: &ContextualCandidate,
    location: &MessageLocation,
    raw_email: &[u8],
) -> Result<ConversionStatus> {
    let _lock = ContextualLock::acquire(target, &account.name)?;
    convert_raw_contextual_unlocked(target, account, candidate, location, raw_email)
}

pub(crate) fn convert_raw_contextual_unlocked(
    target: &Path,
    account: &crate::config::Account,
    candidate: &ContextualCandidate,
    location: &MessageLocation,
    raw_email: &[u8],
) -> Result<ConversionStatus> {
    if let Some((markdown, proof)) = find_existing_proof(target, candidate)? {
        return Ok(ConversionStatus::AlreadyPresent { markdown, proof });
    }
    if !candidate.locations.iter().any(|known| known == location) {
        anyhow::bail!("selected IMAP location does not belong to the candidate");
    }
    let proof = LocalSourceProof {
        identity: candidate.source.clone(),
        location: location.clone(),
    };
    let staging = target.join(format!(
        ".email-to-markdown-tmp-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir(&staging)?;

    let result = (|| {
        let mut contextual_account = account.clone();
        contextual_account.skip_existing = false;
        contextual_account.export_directory = staging.to_string_lossy().into_owned();
        let mut context = crate::email_export::ExportContext {
            export_directory: &staging,
            base_export_directory: &staging,
            account: &contextual_account,
            debug_mode: false,
            dests: &[],
            source_proof: Some(&proof),
        };
        let (markdown, _) = crate::email_export::export_to_markdown(
            raw_email,
            vec![location.folder_display.clone()],
            None,
            &mut context,
        )?
        .context("contextual converter skipped a selected message")?;
        install_staged(target, &staging, &markdown, candidate, &proof)
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}

fn install_staged(
    target: &Path,
    staging: &Path,
    staged_markdown: &Path,
    candidate: &ContextualCandidate,
    proof: &LocalSourceProof,
) -> Result<ConversionStatus> {
    let stable_key = &format!("{:x}", md5::compute(candidate.logical_key().as_bytes()))[..10];
    let original_name = staged_markdown
        .file_name()
        .context("contextual converter returned a path without filename")?;
    let final_markdown = unique_stable_path(target, original_name, stable_key);
    let mut markdown_content = fs::read_to_string(staged_markdown)?;
    let mut installed = Vec::new();

    let operation = (|| {
        for entry in fs::read_dir(staging)? {
            let entry = entry?;
            let path = entry.path();
            if path == staged_markdown || !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let destination = unique_stable_path(target, &name, stable_key);
            if destination.file_name() != Some(name.as_os_str()) {
                markdown_content = markdown_content.replace(
                    name.to_string_lossy().as_ref(),
                    destination.file_name().unwrap().to_string_lossy().as_ref(),
                );
            }
            fs::rename(&path, &destination).with_context(|| {
                format!("install attachment {}", destination.display())
            })?;
            installed.push(destination);
        }
        fs::write(staged_markdown, markdown_content)?;
        // Markdown is the local commit marker and is always installed last.
        fs::rename(staged_markdown, &final_markdown)
            .with_context(|| format!("install Markdown {}", final_markdown.display()))?;
        Ok(ConversionStatus::Written {
            markdown: final_markdown,
            proof: proof.clone(),
        })
    })();

    if operation.is_err() {
        for path in installed {
            let _ = fs::remove_file(path);
        }
    }
    operation
}

fn unique_stable_path(target: &Path, original_name: &std::ffi::OsStr, stable_key: &str) -> PathBuf {
    let original = Path::new(original_name);
    let direct = target.join(original);
    if !direct.exists() {
        return direct;
    }
    let stem = original.file_stem().unwrap_or(original_name).to_string_lossy();
    let extension = original.extension().map(|value| value.to_string_lossy());
    for counter in 1usize.. {
        let suffix = if counter == 1 {
            stable_key.to_string()
        } else {
            format!("{stable_key}-{counter}")
        };
        let name = match &extension {
            Some(extension) => format!("{stem}-{suffix}.{extension}"),
            None => format!("{stem}-{suffix}"),
        };
        let candidate = target.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}
