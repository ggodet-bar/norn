use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;

const LEVELS: &[&str] = &[
    "TRACE", "TRC", "DEBUG", "DBG", "INFO", "INF", "NOTICE", "NOTE",
    "WARN", "WARNING", "WRN", "ERROR", "ERR", "FATAL", "CRITICAL", "CRIT",
    "VERBOSE", "VRB",
];

/// Extract candidate category tags from a raw log line. The same line can
/// belong to multiple categories. Anything inside `[...]` or `(...)` is a
/// candidate as long as it doesn't look like a timestamp, log level, or pure
/// number — noise filtering happens later via the promotion threshold.
pub fn extract(line: &str) -> Vec<String> {
    let plain = strip_ansi(line);
    let mut out = Vec::new();
    let mut seen = HashSet::<String>::new();

    for re in [bracketed_re(), parenthesized_re()] {
        for cap in re.captures_iter(&plain) {
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
