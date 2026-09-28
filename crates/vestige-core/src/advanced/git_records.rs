//! # Commit records
//!
//! Turn `git log -p` output into memory records so Backfill can join a failure
//! to the change that caused it, not only to another description of it. Each
//! commit becomes one record tagged `git-commit`; its files, modules and
//! hunk-header symbols ride in the content, so the query-time entity extractor
//! picks them up as join keys with no schema change.

use chrono::{DateTime, Utc};
use std::collections::BTreeSet;

/// Tag marking a commit record. Deliberately not identifier-shaped, so it
/// never becomes a causal join key itself.
pub const COMMIT_TAG: &str = "git-commit";
/// `source_system` key for the idempotent source upsert.
pub const SOURCE_SYSTEM: &str = "git";

/// One parsed commit.
#[derive(Debug, Clone, PartialEq)]
pub struct GitCommit {
    pub sha: String,
    pub time: DateTime<Utc>,
    pub subject: String,
    pub files: Vec<String>,
    /// Symbols from diff hunk headers, path-qualified (`<file>/<symbol>`) so
    /// they pass the entity shape test; a bare lowercase `fn_name` would not.
    pub symbols: Vec<String>,
}

const RECORD_SEP: char = '\u{1e}';
const UNIT_SEP: char = '\u{1f}';
const MAX_FILES: usize = 50;
const MAX_SYMBOLS: usize = 40;

/// Parse `git log -p --unified=0 --no-color --pretty=format:%x1e%H%x1f%aI%x1f%s`.
/// Files come from `diff --git a/X b/Y` lines (the b/ side wins, so renames
/// land under the new name); symbols from hunk-header trailing context.
pub fn parse_git_log(raw: &str) -> Vec<GitCommit> {
    let mut out: Vec<GitCommit> = Vec::new();
    for chunk in raw.split(RECORD_SEP) {
        if chunk.is_empty() {
            continue;
        }
        let (head, body) = match chunk.split_once('\n') {
            Some((h, b)) => (h, b),
            None => (chunk, ""),
        };
        let mut fields = head.split(UNIT_SEP);
        let sha = fields.next().unwrap_or("").trim().to_string();
        if sha.is_empty() {
            continue;
        }
        let time = fields
            .next()
            .unwrap_or("")
            .trim()
            .parse::<DateTime<Utc>>()
            .unwrap_or_default();
        let subject = fields.next().unwrap_or("").trim().to_string();

        let mut files: Vec<String> = Vec::new();
        let mut symbols: BTreeSet<String> = BTreeSet::new();
        for line in body.lines() {
            if let Some(rest) = line.strip_prefix("diff --git a/") {
                if let Some((_, b)) = rest.split_once(" b/") {
                    push_bounded(&mut files, b.trim(), MAX_FILES);
                }
            } else if line.starts_with("@@") {
                if let Some(ctx) = line.split("@@").nth(2) {
                    if let Some(sym) = leading_identifier(ctx) {
                        if let Some(file) = files.last() {
                            if symbols.len() < MAX_SYMBOLS {
                                symbols.insert(format!("{file}/{sym}"));
                            }
                        }
                    }
                }
            }
        }
        out.push(GitCommit {
            sha,
            time,
            subject,
            files,
            symbols: symbols.into_iter().collect(),
        });
    }
    out
}

fn push_bounded(v: &mut Vec<String>, item: &str, cap: usize) {
    if !item.is_empty() && v.len() < cap && !v.iter().any(|x| x == item) {
        v.push(item.to_string());
    }
}

/// Leading identifier of a hunk-header context, skipping language keywords:
/// `@@ -1,2 +3,4 @@ fn write_file(x: u8)` -> `write_file`,
/// `@@ ... @@ def save(self)` -> `save`.
fn leading_identifier(ctx: &str) -> Option<String> {
    const KEYWORDS: &[&str] = &[
        "fn", "def", "function", "func", "method", "class", "struct", "impl",
        "public", "private", "protected", "static", "async", "const", "let",
        "var", "extern", "unsafe",
    ];
    let mut ident = String::new();
    for raw in ctx.split_whitespace() {
        let word = raw.trim_start_matches('&');
        if word.is_empty() {
            continue;
        }
        if ident.is_empty() && KEYWORDS.contains(&word) {
            continue;
        }
        // cut at the first non-identifier char: "write_event(self," -> "write_event"
        ident = word
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        break;
    }
    (!ident.is_empty() && ident.chars().any(|c| !c.is_ascii_digit())).then_some(ident)
}

/// Content for a commit record. Every token on the files/modules/symbols lines
/// is identifier-shaped by construction (see [`super::retroactive_backfill`]),
/// so query-time extraction turns each into a causal join key.
pub fn record_content(c: &GitCommit) -> String {
    let mut s = format!("commit {} {}", c.sha, c.subject);
    if !c.files.is_empty() {
        s.push_str("\nfiles: ");
        s.push_str(&c.files.join(", "));
    }
    // Only multi-segment module paths pass the shape test; single-segment dirs
    // ("src") are already covered by the file entities beneath them.
    let modules: BTreeSet<String> = c
        .files
        .iter()
        .filter_map(|f| f.rsplit_once('/').map(|(d, _)| d.to_string()))
        .filter(|d| d.contains('/'))
        .collect();
    if !modules.is_empty() {
        s.push_str("\nmodules: ");
        s.push_str(&modules.into_iter().collect::<Vec<_>>().join(", "));
    }
    if !c.symbols.is_empty() {
        s.push_str("\nsymbols: ");
        s.push_str(&c.symbols.join(", "));
    }
    s
}

/// Extract X.Y / X.Y.Z version tokens from prose ("worked in 1.41.0, broke in 1.42.1").
/// Groups are capped at 3 digits so calendar strings ("2026.09") don't match.
pub fn extract_versions(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut tok = String::new();
    let flush = |tok: &mut String, out: &mut Vec<String>| {
        let groups: Vec<&str> = tok.split('.').collect();
        let ok = groups.len() >= 2
            && groups.len() <= 3
            && groups.iter().all(|g| !g.is_empty() && g.len() <= 3 && g.chars().all(|c| c.is_ascii_digit()));
        if ok && !out.iter().any(|v| v == tok) {
            out.push(tok.clone());
        }
        tok.clear();
    };
    for c in text.chars() {
        if c.is_ascii_digit() || c == '.' {
            tok.push(c);
        } else {
            flush(&mut tok, &mut out);
        }
    }
    flush(&mut tok, &mut out);
    out
}

fn parse_version(v: &str) -> Vec<u64> {
    v.split('.').map(|g| g.parse().unwrap_or(0)).collect()
}

fn cmp_version(a: &str, b: &str) -> std::cmp::Ordering {
    let (a, b) = (parse_version(a), parse_version(b));
    let n = a.len().max(b.len());
    for i in 0..n {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x.cmp(&y);
        }
    }
    std::cmp::Ordering::Equal
}

/// Tags embedding one of `versions` ("v1.42.1", "release-1.42.1"). Returns the
/// matching tag names, sorted ascending by version.
pub fn match_version_tags<'a>(tags: &[&'a str], versions: &[String]) -> Vec<String> {
    let mut matched: Vec<String> = tags
        .iter()
        .filter(|t| {
            versions.iter().any(|v| {
                t.rsplit(|c: char| c == 'v' || c == '-' || c == '_')
                    .any(|seg| extract_versions(seg).iter().any(|ev| ev == v))
            })
        })
        .map(|t| t.to_string())
        .collect();
    matched.sort_by(|a, b| cmp_version(a, b));
    matched
}

/// (worked_in = lowest, broke_in = highest) when at least two distinct
/// versions matched — the "broke after upgrading" window.
pub fn version_range(matched_tags: &[String]) -> Option<(String, String)> {
    let mut versions: Vec<String> = matched_tags
        .iter()
        .filter_map(|t| extract_versions(t).into_iter().next_back())
        .collect();
    versions.sort_by(|a, b| cmp_version(a, b));
    versions.dedup();
    if versions.len() < 2 {
        return None;
    }
    Some((versions[0].clone(), versions[versions.len() - 1].clone()))
}

/// SHAs from `git rev-list A..B` output, lowercased.
pub fn parse_rev_list(raw: &str) -> std::collections::HashSet<String> {
    raw.lines()
        .map(|l| l.trim().to_ascii_lowercase())
        .filter(|l| !l.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::advanced::retroactive_backfill::extract_entities;

    const FIXTURE: &str = "\u{1e}abc123def4567890abc123def4567890abc12345\u{1f}2026-09-01T12:00:00+00:00\u{1f}fix: bound the heartbeat write
diff --git a/events/local.py b/events/local.py
index 111..222 100644
--- a/events/local.py
+++ b/events/local.py
@@ -10,7 +10,8 @@ def write_event(self, payload):
     return out
diff --git a/src/store.rs b/src/store.rs
@@ -40,6 +40,7 @@ impl LocalFileStore {
     ok
\u{1e}fff000111222333444555666777888999aaaabbbb\u{1f}2026-09-02T08:30:00+00:00\u{1f}docs: readme
diff --git a/README.md b/README.md
@@ -1,3 +1,4 @@ 
     text
";

    #[test]
    fn parse_extracts_files_symbols_and_time() {
        let commits = parse_git_log(FIXTURE);
        assert_eq!(commits.len(), 2);
        let c = &commits[0];
        assert_eq!(c.sha, "abc123def4567890abc123def4567890abc12345");
        assert_eq!(c.files, vec!["events/local.py", "src/store.rs"]);
        assert!(c.symbols.contains(&"events/local.py/write_event".to_string()));
        assert!(c.symbols.contains(&"src/store.rs/LocalFileStore".to_string()));
        assert_eq!(c.time.to_rfc3339(), "2026-09-01T12:00:00+00:00");
        // the docs commit touches files but has no parseable symbol context
        assert_eq!(commits[1].files, vec!["README.md"]);
        assert!(commits[1].symbols.is_empty());
    }

    #[test]
    fn record_content_tokens_are_live_join_keys() {
        let commits = parse_git_log(FIXTURE);
        let content = record_content(&commits[0]);
        let ents = extract_entities(&content, &[]);
        for want in ["events/local.py", "src/store.rs", "events/local.py/write_event"] {
            assert!(ents.iter().any(|e| e == want), "missing {want} in {ents:?}");
        }
    }

    #[test]
    fn versions_ranges_and_rev_list() {
        let text = "Worked in 1.41.0, broke after upgrading to 1.42.1";
        let versions = extract_versions(text);
        assert_eq!(versions, vec!["1.41.0", "1.42.1"]);
        // calendar strings and years must not match
        assert!(extract_versions("since 2026.09 it failed on 2026-09-28").is_empty());

        let tags = ["v1.41.0", "v1.42.1", "whitepaper-tr-2026-01"];
        let matched = match_version_tags(&tags, &versions);
        assert_eq!(matched, vec!["v1.41.0", "v1.42.1"]);
        assert_eq!(
            version_range(&matched),
            Some(("1.41.0".into(), "1.42.1".into()))
        );
        assert_eq!(version_range(&["v1.42.1".into()]), None);

        let shas = parse_rev_list("ABC111\n\n def222 ");
        assert!(shas.contains("abc111") && shas.contains("def222"));
    }
}
