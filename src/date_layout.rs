//! Date suffix of a note folder: `<dest>/<YYYY>`, `<dest>/<YYYY>/<MM>`, …
//!
//! The only place that builds or recognizes the dated part of a destination path.
//! Recognition is a per-segment check (no `Regex`): years `19xx`/`20xx`,
//! months `01`–`12`.

use chrono::Datelike;
use serde::{Deserialize, Serialize};

/// How exported notes are filed by date under their destination folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateLayout {
    /// Notes go straight into the destination folder.
    None,
    /// `<dest>/<YYYY>`
    Year,
    /// `<dest>/<YYYY>/<MM>`
    YearMonth,
    /// `<dest>/<YYYY>/<MM>/<DD>`
    YearMonthDay,
}

impl DateLayout {
    /// Date folder names for `date`, outermost first (empty for `None`).
    pub fn segments<D: Datelike>(self, date: &D) -> Vec<String> {
        let year = format!("{:04}", date.year());
        let month = format!("{:02}", date.month());
        let day = format!("{:02}", date.day());
        match self {
            DateLayout::None => Vec::new(),
            DateLayout::Year => vec![year],
            DateLayout::YearMonth => vec![year, month],
            DateLayout::YearMonthDay => vec![year, month, day],
        }
    }

    /// `base` followed by the date folders of `date` (forward slashes, no double `/`).
    pub fn apply<D: Datelike>(self, base: &str, date: &D) -> String {
        let mut path = base.trim_end_matches('/').to_string();
        for segment in self.segments(date) {
            path.push('/');
            path.push_str(&segment);
        }
        path
    }

    /// Like [`apply`](Self::apply), but leaves `rel` as is when it already ends
    /// with a recognized date suffix (a path picked by hand in the review window
    /// may already be dated: no `…/2026/06/2026/06`). `None` returns `rel` unchanged.
    pub fn ensure<D: Datelike>(self, rel: &str, date: &D) -> String {
        if self == DateLayout::None {
            return rel.to_string();
        }
        if date_suffix_len(rel) > 0 {
            rel.trim_end_matches('/').to_string()
        } else {
            self.apply(rel, date)
        }
    }
}

fn is_year(segment: &str) -> bool {
    segment.len() == 4
        && segment.bytes().all(|b| b.is_ascii_digit())
        && (segment.starts_with("19") || segment.starts_with("20"))
}

fn is_month(segment: &str) -> bool {
    segment.len() == 2
        && segment.bytes().all(|b| b.is_ascii_digit())
        && ("01"..="12").contains(&segment)
}

/// Number of trailing date segments of `rel` (0 when it carries none).
///
/// Recognized suffix: `<YYYY>/<MM>`.
pub fn date_suffix_len(rel: &str) -> usize {
    let segs: Vec<&str> = rel.trim_end_matches('/').split('/').collect();
    let n = segs.len();
    if n >= 2 && is_year(segs[n - 2]) && is_month(segs[n - 1]) {
        2
    } else {
        0
    }
}

/// `rel` without its trailing date suffix (a user-typed path: the router re-appends it).
pub fn strip_date_suffix(rel: &str) -> String {
    let trimmed = rel.trim_end_matches('/');
    let len = date_suffix_len(trimmed);
    if len == 0 {
        return trimmed.to_string();
    }
    let segs: Vec<&str> = trimmed.split('/').collect();
    segs[..segs.len() - len].join("/")
}

/// Destination of a note directory: `rel_dir` minus a trailing `<date>` or
/// `<date>/<bucket>` (forward slashes).
pub fn destination_of(rel_dir: &str) -> String {
    let segs: Vec<&str> = rel_dir.split('/').filter(|s| !s.is_empty()).collect();
    let direct = date_suffix_len(&segs.join("/"));
    let keep = if direct > 0 {
        segs.len() - direct
    } else {
        let in_bucket = match segs.split_last() {
            Some((_, parent)) if !parent.is_empty() => date_suffix_len(&parent.join("/")),
            _ => 0,
        };
        if in_bucket > 0 {
            segs.len() - 1 - in_bucket
        } else {
            segs.len()
        }
    };
    segs[..keep].join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn oct3() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 3).unwrap()
    }

    #[test]
    fn test_apply_year_month() {
        assert_eq!(
            DateLayout::YearMonth.apply("Perso/Banque", &oct3()),
            "Perso/Banque/2026/10"
        );
    }

    #[test]
    fn test_apply_none_keeps_base() {
        assert_eq!(
            DateLayout::None.apply("Perso/Banque", &oct3()),
            "Perso/Banque"
        );
    }

    #[test]
    fn test_apply_no_double_slash() {
        assert_eq!(
            DateLayout::YearMonth.apply("Perso/Banque/", &oct3()),
            "Perso/Banque/2026/10"
        );
    }

    #[test]
    fn test_ensure_skips_already_dated() {
        assert_eq!(
            DateLayout::YearMonth.ensure("Perso/Banque/2026/06", &oct3()),
            "Perso/Banque/2026/06"
        );
    }

    #[test]
    fn test_ensure_appends_to_bare_path() {
        assert_eq!(
            DateLayout::YearMonth.ensure("Perso/Banque", &oct3()),
            "Perso/Banque/2026/10"
        );
    }

    #[test]
    fn test_ensure_none_unchanged() {
        assert_eq!(
            DateLayout::None.ensure("Perso/Banque/", &oct3()),
            "Perso/Banque/"
        );
    }

    #[test]
    fn test_suffix_rejects_year_1899() {
        assert_eq!(date_suffix_len("Perso/Banque/1899/06"), 0);
    }

    #[test]
    fn test_suffix_rejects_month_13() {
        assert_eq!(date_suffix_len("Perso/Banque/2026/13"), 0);
    }

    #[test]
    fn test_suffix_accepts_year_month() {
        assert_eq!(date_suffix_len("Perso/Banque/2026/06"), 2);
        assert_eq!(date_suffix_len("2026/06/"), 2);
    }

    #[test]
    fn test_strip_removes_year_month() {
        assert_eq!(strip_date_suffix("Perso/Banque/2026/06"), "Perso/Banque");
        assert_eq!(strip_date_suffix("Perso/Banque/"), "Perso/Banque");
    }

    #[test]
    fn test_destination_of_direct_and_bucket() {
        assert_eq!(destination_of("Perso/Banque/2026/06"), "Perso/Banque");
        assert_eq!(
            destination_of("Perso/Banque/2026/06/releves"),
            "Perso/Banque"
        );
        assert_eq!(destination_of("Perso/Banque"), "Perso/Banque");
        assert_eq!(destination_of("2026/06"), "");
        assert_eq!(destination_of("2026/06/bucket"), "");
    }
}
