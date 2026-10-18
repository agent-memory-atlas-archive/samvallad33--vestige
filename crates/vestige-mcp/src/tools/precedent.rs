//! `precedent` MCP tool — what the Operator gate ruled, and why. Read-only.
//!
//! Reads the Operator gate's own record at `$OPERATOR_HOME` or `~/.operator`:
//! `receipts/YYYY-MM-DD.jsonl` (one JSON object per line, UTC `ts`, hash-chained
//! `digest`) and `permits/<action_digest>.json[.used|.expired]`. A stop links to
//! its permit by `action_digest`, and by the permit's `stop_receipt` when the
//! file carries one (older permits do not). Nothing here writes, approves,
//! issues, or deletes anything; the hash chain is not verified in this version.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Local, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use vestige_core::Storage;

const PERMIT_NOTE: &str =
    "history, not permission. Permits are single-use and cannot be reused or issued through this tool.";

pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "since": {"type": "string", "default": "24h", "description": "Window, e.g. 30m, 24h, 7d."},
            "cwd": {"type": "string", "description": "Only receipts whose cwd starts with this path."},
            "source": {"type": "string", "description": "Exact source, e.g. claude, zcode."},
            "session": {"type": "string", "description": "Exact session id."},
            "rule": {"type": "string", "description": "Commitment or rule id/name, e.g. OP-011."},
            "decision": {"type": "string", "description": "STOP, ALLOW, SHADOW_STOP, PASS, another recorded decision, or all. Default: STOP plus ALLOW."},
            "limit": {"type": "integer", "default": 10, "description": "1 to 50, newest records first."}
        }
    })
}

#[derive(Deserialize, Default)]
struct Args {
    since: Option<String>,
    cwd: Option<String>,
    source: Option<String>,
    session: Option<String>,
    rule: Option<String>,
    decision: Option<String>,
    limit: Option<i64>,
}

struct Query {
    cutoff: i64,
    cwd: Option<String>,
    source: Option<String>,
    session: Option<String>,
    rule: Option<String>,
    decisions: Option<Vec<String>>,
    limit: usize,
    label: String,
}

fn nonempty(s: Option<String>) -> Option<String> {
    s.map(|x| x.trim().to_string()).filter(|x| !x.is_empty())
}

fn parse_since(s: &str) -> Result<i64, String> {
    let t = s.trim();
    let mut chars = t.chars();
    let unit = chars.next_back().ok_or_else(|| bad_since(s))?;
    let mult = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86_400,
        'w' => 604_800,
        _ => return Err(bad_since(s)),
    };
    chars
        .as_str()
        .trim()
        .parse::<i64>()
        .map(|n| n * mult)
        .map_err(|_| bad_since(s))
}

fn bad_since(s: &str) -> String {
    format!("invalid since {s:?}: use forms like 30m, 24h, 7d")
}

fn parse_decision(v: Option<&str>) -> Result<Option<Vec<String>>, String> {
    let d = v.map(str::trim).filter(|s| !s.is_empty());
    match d.map(str::to_uppercase) {
        None => Ok(Some(vec!["STOP".into(), "ALLOW".into()])),
        Some(x) if x == "ALL" => Ok(None),
        Some(x) if matches!(
            x.as_str(),
            "STOP" | "ALLOW" | "SHADOW_STOP" | "SHADOW_TASTE" | "SHADOW_TASTE_MULTI"
                | "PASS" | "PERMIT_GRANTED" | "GATE_ERROR"
        ) =>
        {
            Ok(Some(vec![x]))
        }
        Some(other) => Err(format!(
            "unknown decision {other:?}: use STOP, ALLOW, SHADOW_STOP, PASS, or all"
        )),
    }
}

fn parse_query(args: Option<Value>) -> Result<Query, String> {
    let a: Args = match args {
        Some(v) => serde_json::from_value(v).map_err(|e| e.to_string())?,
        None => Args::default(),
    };
    let label = nonempty(a.since).unwrap_or_else(|| "24h".into());
    Ok(Query {
        cutoff: Utc::now().timestamp() - parse_since(&label)?,
        cwd: nonempty(a.cwd),
        source: nonempty(a.source),
        session: nonempty(a.session),
        rule: nonempty(a.rule),
        decisions: parse_decision(a.decision.as_deref())?,
        limit: a.limit.unwrap_or(10).clamp(1, 50) as usize,
        label,
    })
}

fn operator_home() -> PathBuf {
    if let Ok(h) = std::env::var("OPERATOR_HOME") {
        let p = PathBuf::from(h);
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    let mut p = directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_default();
    p.push(".operator");
    p
}

fn ts_unix(ts: &str) -> Option<i64> {
    if let Ok(t) = DateTime::parse_from_rfc3339(ts) {
        return Some(t.timestamp());
    }
    chrono::NaiveDateTime::parse_from_str(ts.get(..19)?, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|t| t.and_utc().timestamp())
}

/// (ts, scan order, record) for every receipt in the window, oldest scan first.
/// Lines that do not parse, or carry no readable ts, count in the second slot.
fn read_receipts(home: &Path, cutoff: i64) -> (Vec<(i64, usize, Value)>, usize) {
    let rdir = home.join("receipts");
    let mut files: Vec<PathBuf> = match std::fs::read_dir(&rdir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect(),
        Err(_) => return (Vec::new(), 0),
    };
    files.sort();
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        for line in text.lines() {
            let Ok(rec) = serde_json::from_str::<Value>(line.trim()) else {
                if !line.trim().is_empty() {
                    skipped += 1;
                }
                continue;
            };
            let Some(ts) = rec.get("ts").and_then(Value::as_str).and_then(ts_unix) else {
                skipped += 1;
                continue;
            };
            if ts >= cutoff {
                let seq = out.len();
                out.push((ts, seq, rec));
            }
        }
    }
    (out, skipped)
}

/// Every permit file, each carrying its suffix status and the note.
fn load_permits(home: &Path) -> Vec<Value> {
    let pdir = home.join("permits");
    let Ok(rd) = std::fs::read_dir(&pdir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.filter_map(|e| e.ok()) {
        let Some(name) = e.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let (status, digest) = if let Some(s) = name.strip_suffix(".json.used") {
            ("used", s)
        } else if let Some(s) = name.strip_suffix(".json.expired") {
            ("expired", s)
        } else if let Some(s) = name.strip_suffix(".json") {
            ("active", s)
        } else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(e.path()) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let mut p = json!({
            "action_digest": v.get("action_digest").cloned().unwrap_or(json!(digest)),
            "status": status,
            "granted": v.get("granted").cloned().unwrap_or(Value::Null),
            "expires": v.get("expires").cloned().unwrap_or(Value::Null),
            "single_use": v.get("single_use").cloned().unwrap_or(Value::Null),
            "by": v.get("by").cloned().unwrap_or(Value::Null),
            "note": PERMIT_NOTE,
        });
        for k in ["stop_receipt", "session", "origin_source"] {
            if let Some(x) = v.get(k).filter(|x| !x.is_null()) {
                p[k] = x.clone();
            }
        }
        out.push(p);
    }
    out
}

fn find_permit<'a>(permits: &'a [Value], rec: &Value) -> Option<&'a Value> {
    let ad = rec.get("action_digest").and_then(Value::as_str);
    let dg = rec.get("digest").and_then(Value::as_str);
    permits.iter().find(|p| {
        p.get("action_digest").and_then(Value::as_str).is_some_and(|s| ad.is_some_and(|a| s == a))
            || p
                .get("stop_receipt")
                .and_then(Value::as_str)
                .is_some_and(|s| dg.is_some_and(|d| s == d))
    })
}

fn keep(rec: &Value, q: &Query) -> bool {
    let d = rec.get("decision").and_then(Value::as_str).unwrap_or("");
    if !match &q.decisions {
        Some(set) => set.iter().any(|x| x == d),
        None => !d.is_empty(),
    } {
        return false;
    }
    for (filter, field) in [(&q.source, "source"), (&q.session, "session")] {
        if let Some(want) = filter
            && rec.get(field).and_then(Value::as_str) != Some(want.as_str())
        {
            return false;
        }
    }
    if let Some(want) = &q.cwd
        && !rec
            .get("cwd")
            .and_then(Value::as_str)
            .is_some_and(|c| c.starts_with(want.as_str()))
    {
        return false;
    }
    if let Some(needle) = &q.rule {
        let low = needle.to_lowercase();
        let hit = |s: &str| s.to_lowercase().contains(&low);
        let in_commitments = rec
            .get("commitments")
            .and_then(Value::as_array)
            .map(|a| a.iter().any(|v| v.as_str().is_some_and(&hit)))
            .unwrap_or(false);
        if !rec
            .get("rule")
            .and_then(Value::as_str)
            .is_some_and(&hit)
            && !rec
                .get("commitment")
                .and_then(Value::as_str)
                .is_some_and(hit)
            && !in_commitments
        {
            return false;
        }
    }
    true
}

fn build_item(rec: &Value, permits: &[Value]) -> Value {
    let local = rec
        .get("ts")
        .and_then(Value::as_str)
        .and_then(ts_unix)
        .and_then(|t| DateTime::from_timestamp(t, 0))
        .map(|t| t.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default();
    let mut item = json!({
        "ts_utc": rec.get("ts").cloned().unwrap_or(Value::Null),
        "ts_local": local,
        "decision": rec.get("decision").cloned().unwrap_or(Value::Null),
        "source": rec.get("source").cloned().unwrap_or(Value::Null),
        "session": rec.get("session").cloned().unwrap_or(Value::Null),
        "cwd": rec.get("cwd").cloned().unwrap_or(Value::Null),
        "tool": rec.get("tool").cloned().unwrap_or(Value::Null),
        "action_preview": rec.get("action_preview").cloned().unwrap_or(Value::Null),
        "rule": rec.get("rule").or_else(|| rec.get("commitment")).cloned().unwrap_or(Value::Null),
        "detail": rec.get("detail").cloned().unwrap_or(Value::Null),
        "digest": rec.get("digest").cloned().unwrap_or(Value::Null),
    });
    for k in ["via", "action_digest", "commitment"] {
        if let Some(v) = rec.get(k) {
            item[k] = v.clone();
        }
    }
    if let Some(p) = find_permit(permits, rec) {
        item["permit"] = p.clone();
    }
    mask_value(&mut item);
    item
}

fn mask_secrets(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < s.len() {
        match secret_at(s, i) {
            Some(next) => {
                out.push_str("[REDACTED]");
                i = next;
            }
            None => {
                let ch = s[i..].chars().next().unwrap_or('?');
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

fn boundary_before(s: &str, i: usize) -> bool {
    s[..i]
        .chars()
        .next_back()
        .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
}

/// One secret-shaped run starting exactly at `i`, as the end offset of the
/// whole match, or None. Mirrors the gate's own redact() backstop.
fn secret_at(s: &str, i: usize) -> Option<usize> {
    let b = s.as_bytes();
    for pat in ["sk_live_", "rk_live_", "ghp_", "sk-"] {
        if s[i..].starts_with(pat) && boundary_before(s, i) {
            let mut j = i + pat.len();
            while b
                .get(j)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
            {
                j += 1;
            }
            if pat == "sk_live_" || pat == "rk_live_" || j - i - pat.len() >= 8 {
                return Some(j);
            }
        }
    }
    if s[i..].starts_with("AKIA") && boundary_before(s, i) {
        let mut j = i + 4;
        while b.get(j).is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
            j += 1;
        }
        if j - i >= 8 {
            return Some(j);
        }
    }
    if !boundary_before(s, i) {
        return None;
    }
    for name in ["api_key", "api-key", "apikey", "token", "secret", "password"] {
        let nb = name.as_bytes();
        if b.len() - i < nb.len() || !b[i..i + nb.len()].iter().zip(nb).all(|(a, c)| a.eq_ignore_ascii_case(c)) {
            continue;
        }
        let mut j = i + nb.len();
        while b.get(j).is_some_and(|c| *c == b' ' || *c == b'\t') {
            j += 1;
        }
        if !matches!(b.get(j), Some(b'=') | Some(b':')) {
            continue;
        }
        j += 1;
        while b.get(j).is_some_and(|c| *c == b' ' || *c == b'\t') {
            j += 1;
        }
        let mut k = j;
        while let Some(c) = s[k..].chars().next().filter(|c| !c.is_whitespace()) {
            k += c.len_utf8();
        }
        if k > j {
            return Some(k);
        }
    }
    if b.len() - i >= 7 && b[i..i + 7].iter().zip(b"bearer ").all(|(a, c)| a.eq_ignore_ascii_case(c)) {
        let mut k = i + 7;
        while let Some(c) = s[k..].chars().next().filter(|c| !c.is_whitespace()) {
            k += c.len_utf8();
        }
        if k > i + 7 {
            return Some(k);
        }
    }
    None
}

fn mask_value(v: &mut Value) {
    match v {
        Value::String(s) => *s = mask_secrets(s),
        Value::Array(a) => a.iter_mut().for_each(mask_value),
        Value::Object(o) => o.values_mut().for_each(mask_value),
        _ => {}
    }
}

fn run(home: &Path, q: &Query) -> Result<Value, String> {
    let rdir = home.join("receipts");
    if !rdir.is_dir() {
        return Ok(json!({
            "tool": "precedent",
            "summary": format!("Operator receipts not found at {}", rdir.display()),
            "items": [],
            "skipped_lines": 0
        }));
    }
    let (recs, skipped) = read_receipts(home, q.cutoff);
    let permits = load_permits(home);
    let mut hits: Vec<(i64, usize, Value)> = recs
        .into_iter()
        .filter(|(_, _, rec)| keep(rec, q))
        .map(|(ts, seq, rec)| (ts, seq, build_item(&rec, &permits)))
        .collect();
    let stops = hits.iter().filter(|(_, _, i)| i["decision"] == "STOP").count();
    let allowed = hits
        .iter()
        .filter(|(_, _, i)| i["decision"] == "ALLOW" && i["via"] == "permit")
        .count();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let mut summary = format!(
        "{} stop{} in the last {}",
        stops,
        if stops == 1 { "" } else { "s" },
        q.label
    );
    if allowed > 0 {
        summary.push_str(&format!(
            ", {} permit{} used",
            allowed,
            if allowed == 1 { "" } else { "s" }
        ));
    }
    Ok(json!({
        "tool": "precedent",
        "summary": summary,
        "items": hits.into_iter().map(|(_, _, i)| i).take(q.limit).collect::<Vec<_>>(),
        "skipped_lines": skipped
    }))
}

pub async fn execute(_storage: &Arc<Storage>, args: Option<Value>) -> Result<Value, String> {
    let q = parse_query(args)?;
    run(&operator_home(), &q)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn iso(ts: DateTime<Utc>) -> String {
        ts.format("%Y-%m-%dT%H:%M:%SZ").to_string()
    }

    fn write(home: &Path, rel: &str, body: &str) {
        let p = home.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    fn rec(ts: &str, decision: &str, extra: &str) -> String {
        format!(
            r#"{{"action_preview":"do a thing","commitments":[],"cwd":"/repo","decision":"{decision}","digest":"{digest}","gate_version":"0.3.13","session":"sess-1","source":"claude","tool":"Bash","ts":"{ts}","v":1{extra}}}"#,
            digest = "d".repeat(64),
        )
    }

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        let home = dir.path().to_path_buf();
        let now = Utc::now();
        let fresh = iso(now - chrono::Duration::hours(2));
        let stale = iso(now - chrono::Duration::days(3));
        let stop = rec(
            &fresh,
            "STOP",
            r#","action_digest":"aaaa1111aaaa1111aaaa1111","commitment":"OP-011","rule":"no-unrestorable-loss","detail":"git checkout discards uncommitted changes""#,
        );
        let allowed = rec(
            &fresh,
            "ALLOW",
            r#","action_digest":"aaaa1111aaaa1111aaaa1111","commitments":["OP-011"],"via":"permit""#,
        );
        write(
            &home,
            "receipts/2026-10-12.jsonl",
            &format!("{stop}\nnot json at all\n\n{allowed}\n{}\n", rec(&fresh, "PASS", "")),
        );
        write(
            &home,
            "receipts/2026-10-09.jsonl",
            &format!(
                "{}\n",
                rec(&stale, "STOP", r#","action_digest":"cccc3333cccc3333cccc3333","commitment":"OP-003","rule":"no-blind-recursive-delete","detail":"old stop""#)
            ),
        );
        write(
            &home,
            "permits/aaaa1111aaaa1111aaaa1111.json.used",
            r#"{"action_digest":"aaaa1111aaaa1111aaaa1111","granted":1791763200.0,"expires":1791764100.0,"single_use":true,"by":"owner-tty","stop_receipt":"dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444","session":"sess-1","origin_source":"claude"}"#,
        );
        write(
            &home,
            "permits/eeee5555eeee5555eeee5555.json",
            r#"{"action_digest":"eeee5555eeee5555eeee5555","granted":1000000000.0,"expires":1000000900.0,"single_use":true,"by":"owner-tty"}"#,
        );
        write(
            &home,
            "permits/ffff6666ffff6666ffff6666.json.expired",
            r#"{"action_digest":"ffff6666ffff6666ffff6666","granted":1000000000.0,"expires":1000000900.0,"single_use":true,"by":"owner-tty"}"#,
        );
        (dir, home)
    }

    fn query(json: Value) -> Query {
        parse_query(Some(json)).unwrap()
    }

    #[test]
    fn since_parsing() {
        assert_eq!(parse_since("24h").unwrap(), 86_400);
        assert_eq!(parse_since("7d").unwrap(), 604_800);
        assert_eq!(parse_since("30m").unwrap(), 1_800);
        assert_eq!(parse_since("2w").unwrap(), 1_209_600);
        assert!(parse_since("yesterday").is_err());
        assert!(parse_since("").is_err());
    }

    #[test]
    fn decision_parsing() {
        assert_eq!(
            parse_decision(None).unwrap(),
            Some(vec!["STOP".to_string(), "ALLOW".to_string()])
        );
        assert_eq!(parse_decision(Some("all")).unwrap(), None);
        assert_eq!(
            parse_decision(Some("shadow_stop")).unwrap(),
            Some(vec!["SHADOW_STOP".to_string()])
        );
        assert!(parse_decision(Some("maybe")).is_err());
    }

    #[test]
    fn secret_masking() {
        assert_eq!(mask_secrets("key sk-abcdefghijklmnop1234 end"), "key [REDACTED] end");
        assert_eq!(mask_secrets("tok ghp_abcdefghijklmnopqrst"), "tok [REDACTED]");
        assert_eq!(mask_secrets("AKIAIOSFODNN7EXAMPLE"), "[REDACTED]");
        assert_eq!(mask_secrets("sk_live_abc123"), "[REDACTED]");
        assert_eq!(mask_secrets("Authorization: Bearer abc.xyz"), "Authorization: [REDACTED]");
        assert_eq!(mask_secrets("api_key=supersecret"), "[REDACTED]");
        assert_eq!(mask_secrets("Token: abc123"), "[REDACTED]");
        assert_eq!(mask_secrets("password: hunter2"), "[REDACTED]");
        assert_eq!(mask_secrets("a task-x and tokens stay"), "a task-x and tokens stay");
        assert_eq!(mask_secrets("sk-short stays"), "sk-short stays");
    }

    #[test]
    fn secret_masking_survives_non_ascii_values() {
        assert_eq!(mask_secrets("Authorization: Bearer abc… end"), "Authorization: [REDACTED] end");
        assert_eq!(mask_secrets("token: café ok"), "[REDACTED] ok");
        assert_eq!(mask_secrets("password=🔑🔑 x"), "[REDACTED] x");
        assert_eq!(mask_secrets("Bearer é"), "[REDACTED]");
    }

    #[test]
    fn masks_inside_items_and_permits() {
        let mut v = json!({"action_preview": "echo api_key=zzz", "permit": {"session": "Bearer qq"}});
        mask_value(&mut v);
        assert_eq!(v["action_preview"], "echo [REDACTED]");
        assert_eq!(v["permit"]["session"], "[REDACTED]");
    }

    #[test]
    fn default_window_and_decision_filter() {
        let (_d, home) = fixture();
        let out = run(&home, &query(json!({}))).unwrap();
        assert_eq!(out["summary"], "1 stop in the last 24h, 1 permit used");
        let items = out["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["decision"], "ALLOW");
        assert_eq!(items[1]["decision"], "STOP");
        assert_eq!(out["skipped_lines"], 1);
    }

    #[test]
    fn since_excludes_old_receipts() {
        let (_d, home) = fixture();
        let out = run(&home, &query(json!({"since": "1d", "decision": "all"}))).unwrap();
        let items = out["items"].as_array().unwrap();
        assert!(items.iter().all(|i| i["detail"] != "old stop"));
    }

    #[test]
    fn all_decisions_includes_pass() {
        let (_d, home) = fixture();
        let out = run(&home, &query(json!({"decision": "all"}))).unwrap();
        let items = out["items"].as_array().unwrap();
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn rule_filter_hits_commitments_and_rule_names() {
        let (_d, home) = fixture();
        for needle in ["OP-011", "op-011"] {
            let out = run(&home, &query(json!({"rule": needle}))).unwrap();
            let items = out["items"].as_array().unwrap();
            assert_eq!(items.len(), 2, "needle {needle}");
            assert_eq!(items[1]["rule"], "no-unrestorable-loss");
        }
        let out = run(&home, &query(json!({"rule": "no-unrestorable-loss"}))).unwrap();
        let items = out["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["rule"], "no-unrestorable-loss");
        let out = run(&home, &query(json!({"rule": "OP-003"}))).unwrap();
        assert_eq!(out["items"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn cwd_prefix_and_exact_filters() {
        let (_d, home) = fixture();
        let out = run(&home, &query(json!({"cwd": "/repo", "decision": "all"}))).unwrap();
        assert_eq!(out["items"].as_array().unwrap().len(), 3);
        let out = run(&home, &query(json!({"cwd": "/other"}))).unwrap();
        assert_eq!(out["items"].as_array().unwrap().len(), 0);
        let out = run(&home, &query(json!({"source": "claude"}))).unwrap();
        assert_eq!(out["items"].as_array().unwrap().len(), 2);
        let out = run(&home, &query(json!({"source": "codex"}))).unwrap();
        assert_eq!(out["items"].as_array().unwrap().len(), 0);
        let out = run(&home, &query(json!({"session": "sess-1"}))).unwrap();
        assert_eq!(out["items"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn permit_links_by_digest_with_status_from_suffix() {
        let (_d, home) = fixture();
        let out = run(&home, &query(json!({}))).unwrap();
        let items = out["items"].as_array().unwrap();
        let stopped = items.iter().find(|i| i["decision"] == "STOP").unwrap();
        let allowed = items.iter().find(|i| i["decision"] == "ALLOW").unwrap();
        assert_eq!(stopped["permit"]["status"], "used");
        assert_eq!(
            stopped["permit"]["stop_receipt"],
            "dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444"
        );
        assert_eq!(stopped["permit"]["note"], PERMIT_NOTE);
        assert_eq!(allowed["permit"]["action_digest"], "aaaa1111aaaa1111aaaa1111");
        assert_eq!(allowed["permit"]["status"], "used");
        assert_eq!(allowed["permit"]["origin_source"], "claude");
    }

    #[test]
    fn active_and_expired_permits_are_visible_via_all() {
        let (_d, home) = fixture();
        let h1 = iso(Utc::now() - chrono::Duration::hours(1));
        write(
            &home,
            "receipts/2026-10-12.jsonl",
            &format!(
                "{}\n{}\n{}\n",
                rec(&h1, "STOP", r#","action_digest":"eeee5555eeee5555eeee5555","commitment":"OP-006","rule":"no-paid-deploy","detail":"x""#),
                rec(&h1, "STOP", r#","action_digest":"ffff6666ffff6666ffff6666","commitment":"OP-005","rule":"no-unreviewed-publish","detail":"y""#),
                rec(&h1, "STOP", r#","action_digest":"012345678901234567890123","commitment":"OP-003","rule":"no-blind-recursive-delete","detail":"z""#),
            ),
        );
        let out = run(&home, &query(json!({}))).unwrap();
        let items = out["items"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        let find = |ad: &str| {
            items
                .iter()
                .find(|i| i["action_digest"] == ad)
                .unwrap()
                .clone()
        };
        assert_eq!(find("eeee5555eeee5555eeee5555")["permit"]["status"], "active");
        assert_eq!(
            find("ffff6666ffff6666ffff6666")["permit"]["status"],
            "expired"
        );
        assert!(find("012345678901234567890123").get("permit").is_none());
        assert_eq!(out["summary"], "3 stops in the last 24h");
    }

    #[test]
    fn limit_keeps_newest_first() {
        let (_d, home) = fixture();
        let out = run(&home, &query(json!({"decision": "all", "limit": 2}))).unwrap();
        let items = out["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(out["summary"], "1 stop in the last 24h, 1 permit used");
    }

    #[test]
    fn missing_directory_is_a_message_not_an_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let out = run(dir.path(), &query(json!({}))).unwrap();
        assert!(
            out["summary"]
                .as_str()
                .unwrap()
                .starts_with("Operator receipts not found")
        );
        assert_eq!(out["items"].as_array().unwrap().len(), 0);
        assert_eq!(out["skipped_lines"], 0);
    }

    #[test]
    fn local_time_is_populated() {
        let (_d, home) = fixture();
        let out = run(&home, &query(json!({}))).unwrap();
        let items = out["items"].as_array().unwrap();
        assert_eq!(items[0]["ts_local"].as_str().unwrap().len(), 19);
    }
}
