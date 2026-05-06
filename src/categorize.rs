use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;

const LEVELS: &[&str] = &[
    "TRACE", "TRC", "DEBUG", "DBG", "INFO", "INF", "NOTICE", "NOTE",
    "WARN", "WARNING", "WRN", "ERROR", "ERR", "FATAL", "CRITICAL", "CRIT",
    "VERBOSE", "VRB",
];

/// Bytes of "glue" punctuation tolerated between two header tokens before the
/// contiguous prefix is considered broken. Wide enough for `" | "`, `" - "`,
/// or `": "`, narrow enough that a sentence's space-word-space cadence ends
/// the header on the first free-text token.
const MAX_GLUE: usize = 3;

/// Extract candidate category tags from a raw log line. Candidates come from
/// `[...]` and `(...)` groups that sit inside the line's leading "header"
/// region — a contiguous run of timestamp / severity / bracketed-or-paren
/// tokens separated by a few glue characters. Bracket/paren groups in the
/// free-text payload are ignored, which is what keeps recurring parenthetical
/// asides ("(session expired)", "(retrying)") out of the category set.
pub fn extract(line: &str) -> Vec<String> {
    let plain = strip_ansi(line);
    let header_end = header_end(&plain);
    let mut out = Vec::new();
    let mut seen = HashSet::<String>::new();

    for re in [bracketed_re(), parenthesized_re()] {
        for cap in re.captures_iter(&plain) {
            if cap.get(0).unwrap().start() >= header_end {
                continue;
            }
            let inner = cap.get(1).unwrap().as_str().trim();
            if accept_tag(inner) && seen.insert(inner.to_string()) {
                out.push(inner.to_string());
            }
        }
    }
    out
}

fn strip_ansi(s: &str) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| Regex::new(r"\x1B\[[0-9;?]*[A-Za-z]").unwrap());
    re.replace_all(s, "").into_owned()
}

fn bracketed_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\[([^\[\]]+)\]").unwrap())
}

fn parenthesized_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\(([^()]+)\)").unwrap())
}

/// Byte offset where the line's contiguous header region ends. The walk
/// starts after any leading whitespace, then greedily eats header tokens
/// (bracketed/parenthesized groups, ISO dates, times, bare severities)
/// separated by up to `MAX_GLUE` bytes of punctuation/whitespace. Returns 0
/// when no header token sits at the start — every bracket/paren group on
/// such lines is then treated as payload and ignored.
fn header_end(plain: &str) -> usize {
    let bytes = plain.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    let re = header_token_re();
    let mut last_end = i;
    let mut found_any = false;

    loop {
        let Some(m) = re.find(&plain[i..]) else { break };
        if m.start() != 0 {
            break;
        }
        i += m.end();
        last_end = i;
        found_any = true;

        let mut glue = 0;
        while i < bytes.len() && glue < MAX_GLUE && is_glue(bytes[i]) {
            i += 1;
            glue += 1;
        }
    }

    if found_any { last_end } else { 0 }
}

fn is_glue(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b':' | b'-' | b'|' | b'>' | b',')
}

fn header_token_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        let levels = LEVELS.join("|");
        let pattern = format!(
            r"(?xi)
            ^(?:
                \[[^\[\]]+\]
              | \([^()]+\)
              | \d{{4}}-\d{{2}}-\d{{2}}(?:[T\- ]\d{{2}}:\d{{2}}:\d{{2}}(?:\.\d+)?(?:Z|[+\-]\d{{2}}:?\d{{2}})?)?
              | \d{{8}}[T\- ]\d{{2}}:\d{{2}}:\d{{2}}(?:\.\d+)?(?:Z|[+\-]\d{{2}}:?\d{{2}})?
              | \d{{2}}:\d{{2}}:\d{{2}}(?:\.\d+)?
              | (?:{levels})\b
            )"
        );
        Regex::new(&pattern).unwrap()
    })
}

fn accept_tag(s: &str) -> bool {
    if s.is_empty() || s.len() > 64 {
        return false;
    }
    if is_level(s) || is_timestampish(s) {
        return false;
    }
    // Pure numeric / dotted-number / whitespace-only content is noise (PIDs,
    // counts, decimals); leave proper identifiers alone.
    if s.chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c.is_whitespace())
    {
        return false;
    }
    true
}

fn is_level(s: &str) -> bool {
    let upper = s.to_ascii_uppercase();
    LEVELS.iter().any(|&l| upper == l)
}

fn is_timestampish(s: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| {
        Regex::new(r"\d{4}-\d{2}-\d{2}|\d{2}:\d{2}:\d{2}|^\d+(\.\d+)?$").unwrap()
    });
    re.is_match(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paren_group_in_payload_is_excluded() {
        let cats = extract(
            "2026-05-06 12:00:00 INFO [auth] User did X (session expired)",
        );
        assert_eq!(cats, vec!["auth".to_string()]);
    }

    #[test]
    fn paren_group_inside_header_is_kept() {
        let cats = extract("2026-05-06 INFO [auth] (worker-3) message body");
        assert_eq!(cats, vec!["auth".to_string(), "worker-3".to_string()]);
    }

    #[test]
    fn bare_bracket_prefix_still_works() {
        let cats = extract("[auth] message with (note)");
        assert_eq!(cats, vec!["auth".to_string()]);
    }

    #[test]
    fn line_without_header_yields_no_categories() {
        // No timestamp, no severity, no leading bracket: everything is payload.
        let cats = extract("just a message with (parens) in it");
        assert!(cats.is_empty(), "got {cats:?}");
    }

    #[test]
    fn long_separator_breaks_the_header() {
        // More than MAX_GLUE chars between tokens stops the contiguous run,
        // so the trailing `[b]` is treated as payload.
        let cats = extract("[a] xxxxxxxxx [b]");
        assert_eq!(cats, vec!["a".to_string()]);
    }

    #[test]
    fn bracketed_severity_extends_header_without_becoming_a_category() {
        let cats = extract("[INFO] [auth] message (skip)");
        assert_eq!(cats, vec!["auth".to_string()]);
    }

    #[test]
    fn iso_datetime_with_timezone_extends_header() {
        let cats = extract("2026-05-06T12:00:00.123Z [svc] hello (world)");
        assert_eq!(cats, vec!["svc".to_string()]);
    }

    #[test]
    fn compact_yyyymmdd_dash_time_extends_header() {
        let cats = extract(
            "20250605-16:47:03.940196000 DEBUG [Strategy/ETHUSDC] \
             Rejecting pending position 62 while at px 2610.95",
        );
        assert_eq!(cats, vec!["Strategy/ETHUSDC".to_string()]);
    }
}
