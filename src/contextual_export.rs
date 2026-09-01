//! Contextual IMAP search contracts and pure transformations.
//!
//! This module deliberately has no dependency on the tray/WebView or on an OS
//! shell integration. Network orchestration lives on `ImapExporter`, while the
//! query construction, header parsing and logical-message merging stay testable.

use crate::route::{normalize_address, MatchRule};
use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset};
use imap_proto::{AttributeValue, Response};
use mailparse::MailHeaderMap;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

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
