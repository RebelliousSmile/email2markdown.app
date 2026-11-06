//! Calendar invitations (`text/calendar`, `.ics`): the first `VEVENT` of an email,
//! rendered as an "Invitation" section of its note.
//!
//! Only what a reader needs is kept: title, time, place, organizer, method and
//! description. Recurrence rules, attendees and alarms stay in the `.ics` file.

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, Utc, Weekday};
use mailparse::{MailHeaderMap, ParsedMail};

/// The event of an invitation, values already unescaped.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Invitation {
    /// `REQUEST`, `CANCEL`, `REPLY`, `PUBLISH`… (upper case), when the calendar says.
    pub method: Option<String>,
    /// `STATUS` of the event (upper case), e.g. `CANCELLED`.
    pub status: Option<String>,
    pub summary: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    /// `Name <address>`, or the bare address.
    pub organizer: Option<String>,
    pub start: Option<Moment>,
    pub end: Option<Moment>,
}

/// A date-time of the calendar, as precise as it was written.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Moment {
    /// All-day value (`VALUE=DATE`).
    Day(NaiveDate),
    /// Wall-clock time, either floating or in the zone named by `TZID`.
    Local(NaiveDateTime),
    /// UTC time (`Z` suffix), shown in the machine's time zone.
    Utc(DateTime<Utc>),
}

impl Moment {
    fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if let Some(utc) = value.strip_suffix(['Z', 'z']) {
            let naive = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
            return Some(Moment::Utc(naive.and_utc()));
        }
        if value.len() == 8 {
            return NaiveDate::parse_from_str(value, "%Y%m%d")
                .ok()
                .map(Moment::Day);
        }
        NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S")
            .ok()
            .map(Moment::Local)
    }

    /// Wall-clock time for display; `None` for an all-day value.
    fn local(self) -> Option<NaiveDateTime> {
        match self {
            Moment::Day(_) => None,
            Moment::Local(time) => Some(time),
            Moment::Utc(time) => Some(time.with_timezone(&Local).naive_local()),
        }
    }

    fn date(self) -> NaiveDate {
        match self {
            Moment::Day(day) => day,
            other => other.local().map(|t| t.date()).unwrap_or_default(),
        }
    }
}

/// The first `VEVENT` of an iCalendar text; `None` when it holds no event.
pub fn parse(ics: &str) -> Option<Invitation> {
    let mut invitation = Invitation::default();
    let mut in_event = false;
    let mut found = false;
    // Depth of components nested in the event (`VALARM`), whose properties are not the event's.
    let mut nested = 0usize;
    for line in unfold(ics) {
        let Some((name, params, value)) = split_property(&line) else {
            continue;
        };
        match (name.as_str(), value.trim().to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT") if !found => {
                in_event = true;
                found = true;
                continue;
            }
            ("END", "VEVENT") if in_event => break,
            ("BEGIN", _) if in_event => {
                nested += 1;
                continue;
            }
            ("END", _) if in_event => {
                nested = nested.saturating_sub(1);
                continue;
            }
            _ => {}
        }
        if name == "METHOD" && !in_event {
            invitation.method = Some(value.trim().to_ascii_uppercase());
            continue;
        }
        if !in_event || nested > 0 {
            continue;
        }
        match name.as_str() {
            "SUMMARY" => invitation.summary = non_empty(unescape(&value)),
            "LOCATION" => invitation.location = non_empty(unescape(&value)),
            "DESCRIPTION" => invitation.description = non_empty(unescape(&value)),
            "STATUS" => invitation.status = Some(value.trim().to_ascii_uppercase()),
            "DTSTART" => invitation.start = Moment::parse(&value),
            "DTEND" => invitation.end = Moment::parse(&value),
            "ORGANIZER" => invitation.organizer = organizer(&params, &value),
            _ => {}
        }
    }
    found.then_some(invitation)
}

/// Content lines with folding undone: a line starting with a space or tab continues the previous one.
fn unfold(ics: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in ics.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        match (raw.strip_prefix([' ', '\t']), lines.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ => lines.push(raw.to_string()),
        }
    }
    lines
}

/// `(NAME upper case, raw parameters, value)` of a content line. The value starts
/// at the first `:` outside a quoted parameter value.
fn split_property(line: &str) -> Option<(String, String, String)> {
    let mut quoted = false;
    let colon = line.char_indices().find_map(|(i, c)| match c {
        '"' => {
            quoted = !quoted;
            None
        }
        ':' if !quoted => Some(i),
        _ => None,
    })?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let (name, params) = head.split_once(';').unwrap_or((head, ""));
    Some((
        name.trim().to_ascii_uppercase(),
        params.to_string(),
        value.to_string(),
    ))
}

/// iCalendar TEXT unescaping: `\n`, `\,`, `\;`, `\\`.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// `CN <address>` of an `ORGANIZER` property, the `mailto:` scheme removed.
fn organizer(params: &str, value: &str) -> Option<String> {
    let value = value.trim();
    let address = value
        .get(..7)
        .filter(|scheme| scheme.eq_ignore_ascii_case("mailto:"))
        .map_or(value, |_| &value[7..])
        .trim();
    let name = params.split(';').find_map(|param| {
        let (key, raw) = param.split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("CN")
            .then(|| raw.trim().trim_matches('"').trim().to_string())
    });
    match (name.filter(|n| !n.is_empty()), address.is_empty()) {
        (Some(name), false) => Some(format!("{name} <{address}>")),
        (Some(name), true) => Some(name),
        (None, false) => Some(address.to_string()),
        (None, true) => None,
    }
}

/// The invitation of an email: the first `text/calendar` part, `application/ics`
/// part or `.ics` attachment that holds an event.
pub fn find_invitation(mail: &ParsedMail) -> Option<Invitation> {
    if !mail.subparts.is_empty() {
        return mail.subparts.iter().find_map(find_invitation);
    }
    if !is_calendar_part(mail) {
        return None;
    }
    let raw = mail.get_body_raw().ok()?;
    parse(&String::from_utf8_lossy(&raw))
}

fn is_calendar_part(part: &ParsedMail) -> bool {
    let mimetype = part.ctype.mimetype.to_ascii_lowercase();
    if matches!(mimetype.as_str(), "text/calendar" | "application/ics") {
        return true;
    }
    let named_ics = |name: &str| name.trim().to_ascii_lowercase().ends_with(".ics");
    part.ctype.params.get("name").is_some_and(|n| named_ics(n))
        || part
            .headers
            .get_first_value("Content-Disposition")
            .is_some_and(|d| {
                mailparse::parse_content_disposition(&d)
                    .params
                    .get("filename")
                    .is_some_and(|n| named_ics(n))
            })
}

/// Markdown section of an invitation, placed after the email block. The
/// description is left out when `body` already carries it.
pub fn render(invitation: &Invitation, body: &str) -> String {
    let mut out = format!("\n\n### {} :\n", heading(invitation));
    let mut item = |label: &str, value: &str| {
        out.push_str(&format!("- **{label}** : {value}\n"));
    };
    if let Some(summary) = &invitation.summary {
        item("Titre", &one_line(summary));
    }
    if let Some(when) = when(invitation.start, invitation.end) {
        item("Quand", &when);
    }
    if let Some(location) = &invitation.location {
        item("Lieu", &one_line(location));
    }
    if let Some(organizer) = &invitation.organizer {
        item("Organisateur", organizer);
    }
    if let Some(description) = &invitation.description {
        if !already_in(description, body) {
            out.push('\n');
            for line in description.lines() {
                out.push_str(line.trim_end());
                out.push('\n');
            }
        }
    }
    out
}

fn heading(invitation: &Invitation) -> &'static str {
    let cancelled = invitation.method.as_deref() == Some("CANCEL")
        || invitation.status.as_deref() == Some("CANCELLED");
    if cancelled {
        return "Évènement annulé";
    }
    match invitation.method.as_deref() {
        Some("REPLY") => "Réponse à une invitation",
        Some("REQUEST") => "Invitation à un évènement",
        _ => "Évènement",
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether the opening of `description` is already written in `body`, ignoring
/// punctuation, case and wrapping.
fn already_in(description: &str, body: &str) -> bool {
    let key: String = alnum(description).chars().take(80).collect();
    !key.is_empty() && alnum(body).contains(&key)
}

fn alnum(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// `mercredi 5 mai 2027, 18:00 – 18:10`; the end date is repeated only when it differs.
fn when(start: Option<Moment>, end: Option<Moment>) -> Option<String> {
    let start = start?;
    let Some(start_time) = start.local() else {
        // All-day: DTEND is exclusive, so a one-day event ends the next day.
        let first = start.date();
        let last = end
            .map(|e| e.date().pred_opt().unwrap_or(first))
            .filter(|last| *last > first);
        return Some(match last {
            Some(last) => format!("{} – {}", long_date(first), long_date(last)),
            None => format!("{} (journée entière)", long_date(first)),
        });
    };
    let start_text = format!(
        "{}, {}",
        long_date(start_time.date()),
        start_time.format("%H:%M")
    );
    let Some(end_time) = end.and_then(Moment::local) else {
        return Some(start_text);
    };
    Some(if end_time.date() == start_time.date() {
        format!("{start_text} – {}", end_time.format("%H:%M"))
    } else {
        format!(
            "{start_text} – {}, {}",
            long_date(end_time.date()),
            end_time.format("%H:%M")
        )
    })
}

fn long_date(date: NaiveDate) -> String {
    const MONTHS: [&str; 12] = [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ];
    let weekday = match date.weekday() {
        Weekday::Mon => "lundi",
        Weekday::Tue => "mardi",
        Weekday::Wed => "mercredi",
        Weekday::Thu => "jeudi",
        Weekday::Fri => "vendredi",
        Weekday::Sat => "samedi",
        Weekday::Sun => "dimanche",
    };
    format!(
        "{weekday} {} {} {}",
        date.day(),
        MONTHS[date.month0() as usize],
        date.year()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCTOLIB: &str = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
METHOD:REQUEST\r\n\
BEGIN:VEVENT\r\n\
DTSTART;TZID=Europe/Paris:20270505T180000\r\n\
DTEND;TZID=Europe/Paris:20270505T181000\r\n\
SUMMARY:Rendez-vous chez Dr Gregory Meyrieu\r\n\
LOCATION:3 bis Rue Henry Bordeaux\\, 74000 Annecy\r\n\
ORGANIZER;CN=Doctolib:mailto:no-reply@doctolib.fr\r\n\
DESCRIPTION:Informations pratiques :\\n* Au Rez-de-chaussée\\n* Sans as\r\n censeur\r\n\
BEGIN:VALARM\r\n\
DESCRIPTION:Rappel\r\n\
END:VALARM\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

    fn doctolib() -> Invitation {
        parse(DOCTOLIB).expect("event")
    }

    #[test]
    fn test_parse_reads_summary() {
        assert_eq!(
            doctolib().summary.as_deref(),
            Some("Rendez-vous chez Dr Gregory Meyrieu")
        );
    }

    #[test]
    fn test_parse_unescapes_location() {
        assert_eq!(
            doctolib().location.as_deref(),
            Some("3 bis Rue Henry Bordeaux, 74000 Annecy")
        );
    }

    #[test]
    fn test_parse_unfolds_and_unescapes_description() {
        assert_eq!(
            doctolib().description.as_deref(),
            Some("Informations pratiques :\n* Au Rez-de-chaussée\n* Sans ascenseur")
        );
    }

    #[test]
    fn test_parse_ignores_alarm_properties() {
        assert!(!doctolib().description.unwrap().contains("Rappel"));
    }

    #[test]
    fn test_parse_reads_organizer_name_and_address() {
        assert_eq!(
            doctolib().organizer.as_deref(),
            Some("Doctolib <no-reply@doctolib.fr>")
        );
    }

    #[test]
    fn test_parse_reads_method() {
        assert_eq!(doctolib().method.as_deref(), Some("REQUEST"));
    }

    #[test]
    fn test_parse_without_event_is_none() {
        assert_eq!(parse("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n"), None);
    }

    #[test]
    fn test_organizer_with_quoted_cn_holding_colon() {
        let ics = "BEGIN:VEVENT\nORGANIZER;CN=\"Dupont: RH\":MAILTO:rh@a.test\nEND:VEVENT\n";
        assert_eq!(
            parse(ics).unwrap().organizer.as_deref(),
            Some("Dupont: RH <rh@a.test>")
        );
    }

    #[test]
    fn test_when_same_day_shows_end_time_only() {
        assert_eq!(
            when(doctolib().start, doctolib().end).as_deref(),
            Some("mercredi 5 mai 2027, 18:00 – 18:10")
        );
    }

    #[test]
    fn test_when_across_days_repeats_end_date() {
        let start = Moment::parse("20271231T220000");
        let end = Moment::parse("20280101T010000");
        assert_eq!(
            when(start, end).as_deref(),
            Some("vendredi 31 décembre 2027, 22:00 – samedi 1 janvier 2028, 01:00")
        );
    }

    #[test]
    fn test_when_with_single_all_day_event() {
        let start = Moment::parse("20270505");
        let end = Moment::parse("20270506");
        assert_eq!(
            when(start, end).as_deref(),
            Some("mercredi 5 mai 2027 (journée entière)")
        );
    }

    #[test]
    fn test_when_with_utc_converts_to_local_time() {
        let start = Moment::parse("20270505T160000Z");
        let expected = DateTime::parse_from_rfc3339("2027-05-05T16:00:00Z")
            .unwrap()
            .with_timezone(&Local)
            .format("%H:%M")
            .to_string();
        assert!(when(start, None).unwrap().ends_with(&expected));
    }

    #[test]
    fn test_render_lists_title_time_place_organizer() {
        let section = render(&doctolib(), "");
        assert!(section.starts_with("\n\n### Invitation à un évènement :\n"));
        assert!(section.contains("- **Titre** : Rendez-vous chez Dr Gregory Meyrieu\n"));
        assert!(section.contains("- **Quand** : mercredi 5 mai 2027, 18:00 – 18:10\n"));
        assert!(section.contains("- **Lieu** : 3 bis Rue Henry Bordeaux, 74000 Annecy\n"));
        assert!(section.contains("- **Organisateur** : Doctolib <no-reply@doctolib.fr>\n"));
    }

    #[test]
    fn test_render_keeps_description_missing_from_body() {
        let section = render(&doctolib(), "Votre rendez-vous est confirmé.");
        assert!(section.contains("\n* Au Rez-de-chaussée\n* Sans ascenseur\n"));
    }

    #[test]
    fn test_render_skips_description_already_in_body() {
        let body = "Informations pratiques : * Au Rez-de-chaussée * Sans ascenseur";
        assert!(!render(&doctolib(), body).contains("Sans ascenseur"));
    }

    #[test]
    fn test_render_with_cancel_method_says_cancelled() {
        let invitation = Invitation {
            method: Some("CANCEL".into()),
            ..doctolib()
        };
        assert!(render(&invitation, "").contains("### Évènement annulé :"));
    }
}
