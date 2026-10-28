//! `reflex`: the Vestige tool that agent hooks call directly.
//!
//! Hidden: it is a dispatch arm, never part of `tools/list`, so it costs no
//! schema bytes and no agent ever calls it by choice. Claude Code reaches it
//! from `mcp_tool` hooks on the agent's own MCP connection; other hosts reach it
//! through the `@vestige/init` command shim. Every call answers in hook JSON.
//!
//! Its first handler is the stop-to-Strata reflex, moved into the server that
//! owns the store. On every hook firing it reads the Operator gate's day files
//! under `$OPERATOR_HOME/receipts` (default `~/.operator`) from where it last
//! stopped, checks each new line against the gate's digest chain, and:
//!
//! - saves every STOP as an `operator-stop` memory `derived_from` its rule
//!   memory, with the same tags and text `hooks/operator-stop-memory.py` writes,
//!   so `recall(handle='op-receipt-<16 hex>')` and the PRECEDENT journey see no
//!   change. A line that breaks the chain never becomes a memory.
//! - records which memories the stopped agent held at that exact tool call,
//!   when a PreToolUse firing captured them.
//! - links a later owner permit, or the law's own compliant path, to the stop it
//!   answers with a `closes` edge, and names that path the next time the same
//!   rule stops a call.
//! - tells the stopped session once, inside a per-event and per-turn budget,
//!   and warns before the exact same command is tried again.
//!
//! Every firing appends one hash-chained line to `<data_dir>/reflex/ledger.jsonl`,
//! the Hook Ledger, which `reflex(action='ledger')` summarizes and verifies.
//!
//! The gate decides. Reflex records and tells. It never allows, retries, or
//! routes around a stop, and nothing here reads or writes the gate's own files
//! beyond reading its receipts.

use std::collections::{HashMap, VecDeque};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use chrono::{DateTime, Days, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Digest as _;
use tokio::sync::Mutex;
use vestige_core::{OutputConfig, Storage};

use crate::cognitive::CognitiveEngine;

/// The dispatch name hooks call.
pub const NAME: &str = "reflex";

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Gate receipts saved as memories per firing; the rest wait for the next one.
const MAX_WRITES: usize = 8;
/// Time one firing may spend writing memories before it answers.
const WRITE_BUDGET_MS: u128 = 250;
/// Day files read the first time reflex sees a gate home.
const FIRST_RUN_DAYS: u64 = 7;
const MAX_PENDING: usize = 500;
const MAX_DONE: usize = 2000;
const MAX_OPEN: usize = 500;
const MAX_COMMANDS: usize = 2000;
const RECENT_KEEP: usize = 20;
const RECENT_SHOWN: usize = 5;
const RECENT_DAYS: i64 = 7;
/// Characters reflex may add to one turn across every event.
const TURN_BUDGET: usize = 4000;
const MANIFEST_KEEP: usize = 400;
const MANIFEST_IN_STOP: usize = 40;
const SNAPSHOT_KEEP: usize = 128;
const SESSIONS_KEEP: usize = 64;
const PERSIST_EVERY_SECS: i64 = 30;
const HEARTBEAT_EVERY_SECS: i64 = 30;
const LEDGER_ROTATE_BYTES: u64 = 8 * 1024 * 1024;
/// Firings a permit or ALLOW waits for the stop it answers before it is dropped.
const RESOLUTION_MAX_TRIES: u32 = 50;

/// Characters one event may add to the agent's context. Zero means the host
/// does not take added context on that event, so reflex only records.
fn event_budget(event: &str) -> usize {
    match event {
        "UserPromptSubmit" | "SessionStart" => 2000,
        "SubagentStart" => 1500,
        "PreToolUse" | "PostToolUse" | "PostToolUseFailure" | "PostToolBatch" => 600,
        _ => 0,
    }
}

/// Input schema. Not advertised; kept for documentation and tests.
pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "action": {"type": "string", "description": "hook (default), ledger or status."},
            "event": {"type": "string", "description": "The hook event name, e.g. PostToolUse."},
            "session": {"type": "string", "description": "The host's session id."},
            "host": {"type": "string", "description": "claude, codex, cursor, gemini, opencode."},
            "tool": {"type": "string", "description": "The tool the event is about."},
            "call": {"type": "string", "description": "The tool_use_id."},
            "command": {"type": "string", "description": "The shell command, on PreToolUse of a shell tool."},
            "source": {"type": "string", "description": "SessionStart source: startup, resume, clear or compact."},
            "format": {"type": "string", "description": "claude (default) for hook JSON, plain for {context}."},
            "since": {"type": "string", "description": "Ledger window, e.g. 30m, 24h, 7d."}
        }
    })
}

// ---------------------------------------------------------------------------
// Per-connection state
// ---------------------------------------------------------------------------

/// What reflex knows about one MCP connection: the host session it belongs to,
/// learned from the first reflex call on that connection.
#[derive(Default)]
pub struct Connection {
    session: Option<String>,
}

pub type ConnectionState = Arc<std::sync::Mutex<Connection>>;

pub fn new_connection() -> ConnectionState {
    Arc::new(std::sync::Mutex::new(Connection::default()))
}

/// Record the memory ids a Vestige read put in front of the agent on this
/// connection, so a later stop can say what the agent held. Ids are read from
/// the result's JSON keys, never matched in text.
pub fn observe(conn: &ConnectionState, storage: &Arc<Storage>, tool: &str, result: &Value) {
    if !matches!(tool, "recall" | "memory" | "session_start" | "codebase") {
        return;
    }
    let Some(session) = conn.lock().ok().and_then(|c| c.session.clone()) else {
        return;
    };
    let mut ids = Vec::new();
    collect_ids(result, &mut ids, 0);
    if ids.is_empty() {
        return;
    }
    let global = global_for(storage);
    let Ok(mut g) = global.lock() else {
        return;
    };
    let state = g.session(&session);
    for id in ids {
        state.remember(id);
    }
}

fn collect_ids(v: &Value, out: &mut Vec<String>, depth: usize) {
    if depth > 8 || out.len() >= MANIFEST_KEEP {
        return;
    }
    match v {
        Value::Object(map) => {
            for (k, x) in map {
                if matches!(k.as_str(), "id" | "nodeId" | "memoryId" | "node_id")
                    && let Some(s) = x.as_str()
                    && is_memory_id(s)
                    && !out.iter().any(|o| o == s)
                {
                    out.push(s.to_string());
                }
                collect_ids(x, out, depth + 1);
            }
        }
        Value::Array(items) => {
            for x in items {
                collect_ids(x, out, depth + 1);
            }
        }
        _ => {}
    }
}

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_memory_id(s: &str) -> bool {
    if let Some(rest) = s.strip_prefix("mem-") {
        return rest.len() == 16 && is_hex(rest);
    }
    s.len() == 36
        && s.char_indices().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

fn is_digest(s: &str) -> bool {
    s.len() == 64 && is_hex(s)
}

fn is_rule(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric())
        && s.len() <= 64
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

#[derive(Default, Debug, Clone)]
struct Input {
    action: String,
    event: String,
    session: String,
    host: String,
    tool: String,
    call: String,
    command: String,
    source: String,
    format: String,
    since: String,
}

/// A substitution the host left unfilled (`${...}`) counts as absent.
fn field(args: &Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(s) = args.get(*k).and_then(Value::as_str) {
            let t = s.trim();
            if !t.is_empty() && !t.starts_with("${") {
                return t.chars().take(4000).collect();
            }
        }
    }
    String::new()
}

impl Input {
    fn parse(args: Option<&Value>) -> Input {
        let empty = json!({});
        let a = args.unwrap_or(&empty);
        let nested = a.get("tool_input").cloned().unwrap_or(Value::Null);
        let mut command = field(a, &["command"]);
        if command.is_empty() {
            command = field(&nested, &["command", "cmd"]);
        }
        let action = field(a, &["action"]);
        Input {
            action: if action.is_empty() {
                "hook".into()
            } else {
                action
            },
            event: field(a, &["event", "hook_event_name", "hookEventName"]),
            session: field(
                a,
                &["session", "session_id", "sessionId", "conversation_id"],
            ),
            host: {
                let h = field(a, &["host", "client"]);
                if h.is_empty() { "claude".into() } else { h }
            },
            tool: field(a, &["tool", "tool_name", "toolName"]),
            call: field(a, &["call", "tool_use_id", "toolUseId"]),
            command,
            source: field(a, &["source"]),
            format: field(a, &["format"]),
            since: field(a, &["since"]),
        }
    }
}

// ---------------------------------------------------------------------------
// Gate receipts
// ---------------------------------------------------------------------------

/// The fields of a gate receipt reflex acts on.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
struct GateRow {
    kind: String,
    digest: String,
    prev: String,
    rule: String,
    rule_name: String,
    ts: String,
    source: String,
    session: String,
    cwd: String,
    tool: String,
    preview: String,
    detail: String,
    action: String,
    call: String,
    gate_version: String,
    via: String,
    stop_receipt: String,
    #[serde(default)]
    tries: u32,
}

fn clip(v: Option<&Value>, n: usize) -> String {
    let s = match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(n)
        .collect()
}

impl GateRow {
    fn from_receipt(rec: &Value) -> Option<GateRow> {
        let kind = rec.get("decision")?.as_str()?.to_string();
        let digest = rec.get("digest")?.as_str()?.to_string();
        if !is_digest(&digest) {
            return None;
        }
        let row = GateRow {
            kind: kind.clone(),
            digest,
            prev: clip(rec.get("prev"), 64),
            rule: clip(rec.get("commitment"), 64),
            rule_name: clip(rec.get("rule"), 80),
            ts: clip(rec.get("ts"), 32),
            source: clip(rec.get("source"), 32),
            session: clip(rec.get("session"), 64),
            cwd: clip(rec.get("cwd"), 300),
            tool: clip(rec.get("tool"), 64),
            preview: clip(rec.get("action_preview"), 300),
            detail: clip(rec.get("detail"), 300),
            action: clip(rec.get("action_digest"), 24),
            call: clip(rec.get("call"), 80),
            gate_version: clip(rec.get("gate_version"), 16),
            via: clip(rec.get("via"), 32),
            stop_receipt: clip(rec.get("stop_receipt"), 64),
            tries: 0,
        };
        match kind.as_str() {
            "STOP" => is_rule(&row.rule).then_some(row),
            // Only the ALLOWs that answer a stop: the law's own compliant path,
            // or a one-time permit being used for an action the gate stopped.
            "ALLOW" => (row.via == "law_lift" || !row.action.is_empty()).then_some(row),
            "PERMIT_GRANTED" => is_digest(&row.stop_receipt).then_some(row),
            _ => None,
        }
    }

    fn d16(&self) -> String {
        self.digest.chars().take(16).collect()
    }
}

// ---------------------------------------------------------------------------
// State shared by every connection to one store
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct OpenStop {
    d16: String,
    node: String,
    session: String,
    rule: String,
    action: String,
    ts: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Resolution {
    node: String,
    how: String,
    ts: String,
    stop_d16: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Recent {
    d16: String,
    node: String,
    rule: String,
    ts: String,
    preview: String,
}

/// What reflex keeps across restarts, at `<data_dir>/reflex/state.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Persisted {
    #[serde(default)]
    files: Vec<(String, u64)>,
    #[serde(default)]
    tip: Option<String>,
    #[serde(default)]
    started: bool,
    #[serde(default)]
    lines_read: u64,
    #[serde(default)]
    chain_breaks: u64,
    #[serde(default)]
    pending: Vec<GateRow>,
    #[serde(default)]
    done: Vec<(String, String)>,
    #[serde(default)]
    rules: Vec<(String, String)>,
    #[serde(default)]
    open: Vec<OpenStop>,
    #[serde(default)]
    commands: Vec<(String, String)>,
    #[serde(default)]
    resolutions: Vec<(String, Resolution)>,
    #[serde(default)]
    recent: Vec<Recent>,
    #[serde(default)]
    ledger_tip: Option<String>,
}

fn lookup<'a, T>(list: &'a [(String, T)], key: &str) -> Option<&'a T> {
    list.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn upsert<T>(list: &mut Vec<(String, T)>, key: &str, value: T, cap: usize) {
    list.retain(|(k, _)| k != key);
    list.push((key.to_string(), value));
    if list.len() > cap {
        let drop = list.len() - cap;
        list.drain(..drop);
    }
}

#[derive(Clone, Debug, Default)]
struct StopNote {
    d16: String,
    node: String,
    rule: String,
    rule_node: String,
    preview: String,
    detail: String,
    ts: String,
    resolution: Option<Resolution>,
}

#[derive(Clone, Debug, Default)]
struct Snapshot {
    call: String,
    command: Option<String>,
    manifest: Vec<String>,
}

#[derive(Debug, Default)]
struct SessionState {
    manifest: VecDeque<String>,
    snapshots: VecDeque<Snapshot>,
    stops: Vec<StopNote>,
    told: Vec<String>,
    turn_chars: usize,
    touched: i64,
}

impl SessionState {
    fn remember(&mut self, id: String) {
        if let Some(pos) = self.manifest.iter().position(|x| *x == id) {
            self.manifest.remove(pos);
        }
        self.manifest.push_back(id);
        while self.manifest.len() > MANIFEST_KEEP {
            self.manifest.pop_front();
        }
    }
}

struct Global {
    dir: PathBuf,
    p: Persisted,
    sessions: HashMap<String, SessionState>,
    in_flight: Vec<String>,
    dirty: bool,
    last_persist: i64,
    last_heartbeat: i64,
    /// One background task per store drains other sessions' backlog.
    bg_running: bool,
}

impl Global {
    fn load(dir: PathBuf) -> Global {
        let p = std::fs::read_to_string(dir.join("state.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Persisted>(&t).ok())
            .unwrap_or_default();
        let mut g = Global {
            dir,
            p,
            sessions: HashMap::new(),
            in_flight: Vec::new(),
            dirty: false,
            last_persist: 0,
            last_heartbeat: 0,
            bg_running: false,
        };
        if g.p.ledger_tip.is_none() {
            g.p.ledger_tip = last_ledger_digest(&g.dir.join("ledger.jsonl"));
        }
        g
    }

    fn session(&mut self, id: &str) -> &mut SessionState {
        let now = Utc::now().timestamp();
        if !self.sessions.contains_key(id)
            && self.sessions.len() >= SESSIONS_KEEP
            && let Some(oldest) = self
                .sessions
                .iter()
                .min_by_key(|(_, s)| s.touched)
                .map(|(k, _)| k.clone())
        {
            self.sessions.remove(&oldest);
        }
        let s = self.sessions.entry(id.to_string()).or_default();
        s.touched = now;
        s
    }

    fn persist(&mut self, force: bool) {
        let now = Utc::now().timestamp();
        if !(force || (self.dirty && now - self.last_persist >= PERSIST_EVERY_SECS)) {
            return;
        }
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let tmp = self.dir.join("state.json.tmp");
        let Ok(text) = serde_json::to_string(&self.p) else {
            return;
        };
        if std::fs::write(&tmp, text).is_ok()
            && std::fs::rename(&tmp, self.dir.join("state.json")).is_ok()
        {
            self.dirty = false;
            self.last_persist = now;
        }
    }

    /// The Python stop hook steps aside while this file is fresh.
    fn heartbeat(&mut self) {
        let now = Utc::now().timestamp();
        if now - self.last_heartbeat < HEARTBEAT_EVERY_SECS {
            return;
        }
        if std::fs::create_dir_all(&self.dir).is_ok()
            && std::fs::write(self.dir.join("heartbeat"), now.to_string()).is_ok()
        {
            self.last_heartbeat = now;
        }
    }
}

fn registry() -> &'static std::sync::Mutex<HashMap<PathBuf, Arc<std::sync::Mutex<Global>>>> {
    static REG: OnceLock<std::sync::Mutex<HashMap<PathBuf, Arc<std::sync::Mutex<Global>>>>> =
        OnceLock::new();
    REG.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn global_for(storage: &Arc<Storage>) -> Arc<std::sync::Mutex<Global>> {
    let dir = storage.data_dir().join("reflex");
    let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    reg.entry(dir.clone())
        .or_insert_with(|| Arc::new(std::sync::Mutex::new(Global::load(dir))))
        .clone()
}

// ---------------------------------------------------------------------------
// Reading the gate's receipts
// ---------------------------------------------------------------------------

fn day_files(rdir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(rdir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            let stem = name.strip_suffix(".jsonl")?;
            NaiveDate::parse_from_str(stem, "%Y-%m-%d").ok()?;
            Some((name, e.path()))
        })
        .collect();
    out.sort();
    out
}

/// Read new lines from the gate's day files, check each against the digest
/// chain, and queue the rows reflex acts on. A line that breaks the chain is
/// counted and never queued.
fn scan(g: &mut Global, home: &Path) {
    let files = day_files(&home.join("receipts"));
    if files.is_empty() {
        return;
    }
    if !g.p.started {
        g.p.started = true;
        g.dirty = true;
        let first = Utc::now().date_naive() - Days::new(FIRST_RUN_DAYS);
        for (name, path) in &files {
            let day = name
                .strip_suffix(".jsonl")
                .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
            if day.is_some_and(|d| d < first) {
                let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                upsert(&mut g.p.files, name, size, 4096);
                g.p.tip = super::precedent::last_digest(path).or(g.p.tip.take());
            }
        }
    }
    for (name, path) in &files {
        let Ok(size) = std::fs::metadata(path).map(|m| m.len()) else {
            continue;
        };
        let mut start = lookup(&g.p.files, name).copied().unwrap_or(0);
        if start > size {
            start = 0;
        }
        if start == size {
            continue;
        }
        let Ok(mut f) = std::fs::File::open(path) else {
            continue;
        };
        if f.seek(SeekFrom::Start(start)).is_err() {
            continue;
        }
        let mut buf = Vec::with_capacity((size - start) as usize);
        if f.take(size - start).read_to_end(&mut buf).is_err() {
            continue;
        }
        // A line still being written waits for the next firing.
        let end = buf.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        if end == 0 {
            continue;
        }
        for line in String::from_utf8_lossy(&buf[..end]).lines() {
            if line.trim().is_empty() {
                continue;
            }
            g.p.lines_read += 1;
            let parsed = serde_json::from_str::<Value>(line.trim()).ok();
            let stored = parsed
                .as_ref()
                .and_then(|r| r.get("digest"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let broke = match (&parsed, &stored) {
                (Some(rec), Some(d)) => {
                    let linked =
                        g.p.tip
                            .as_deref()
                            .is_none_or(|t| rec.get("prev").and_then(Value::as_str) == Some(t));
                    let own = super::precedent::gate_digest(rec).is_none_or(|x| x == *d);
                    !(linked && own)
                }
                _ => true,
            };
            g.p.tip = stored;
            if broke {
                g.p.chain_breaks += 1;
                continue;
            }
            if let Some(row) = parsed.as_ref().and_then(GateRow::from_receipt) {
                let d16 = row.d16();
                let known = lookup(&g.p.done, &d16).is_some()
                    || g.p.pending.iter().any(|r| r.digest == row.digest);
                if !known {
                    g.p.pending.push(row);
                }
            }
        }
        upsert(&mut g.p.files, name, start + end as u64, 4096);
        g.dirty = true;
    }
    let names: Vec<&String> = files.iter().map(|(n, _)| n).collect();
    g.p.files.retain(|(n, _)| names.contains(&n));
    if g.p.pending.len() > MAX_PENDING {
        let drop = g.p.pending.len() - MAX_PENDING;
        g.p.pending.drain(..drop);
    }
}

// ---------------------------------------------------------------------------
// Turning rows into memories
// ---------------------------------------------------------------------------

enum Job {
    Stop {
        row: GateRow,
        rule_node: Option<String>,
        manifest: Vec<String>,
        command: Option<String>,
        prior: Option<Resolution>,
    },
    Resolve {
        row: GateRow,
        stop: OpenStop,
        rule_node: Option<String>,
    },
}

enum Outcome {
    Stop {
        row: GateRow,
        node: String,
        rule_node: String,
        command: Option<String>,
        prior: Option<Resolution>,
        wrote: bool,
    },
    Resolve {
        row: GateRow,
        node: String,
        stop: OpenStop,
        wrote: bool,
    },
    Wait {
        digest: String,
    },
}

fn how_of(row: &GateRow) -> String {
    match row.kind.as_str() {
        "PERMIT_GRANTED" => format!(
            "the owner granted a one-time permit from their own terminal for action id {}",
            if row.action.is_empty() {
                "unknown"
            } else {
                &row.action
            }
        ),
        _ if row.via == "law_lift" => format!(
            "the law's own compliant path, then the call ran once: {}",
            if row.preview.is_empty() {
                "(no preview)"
            } else {
                &row.preview
            }
        ),
        _ => format!(
            "the owner's one-time permit was used for: {}",
            if row.preview.is_empty() {
                "(no preview)"
            } else {
                &row.preview
            }
        ),
    }
}

/// Pick this firing's work under the lock: the current session's rows first,
/// each group oldest first, so a stop is saved before the rows that answer it.
fn take_jobs(g: &mut Global, session: &str) -> Vec<Job> {
    let mut order: Vec<usize> = (0..g.p.pending.len()).collect();
    order.sort_by_key(|&i| {
        let r = &g.p.pending[i];
        (r.session != session, r.ts.clone())
    });
    let mut jobs = Vec::new();
    let mut give_up = Vec::new();
    for i in order {
        if jobs.len() >= MAX_WRITES {
            break;
        }
        let row = g.p.pending[i].clone();
        if g.in_flight.contains(&row.digest) {
            continue;
        }
        if row.kind == "STOP" {
            let rule_node = lookup(&g.p.rules, &row.rule).cloned();
            let prior = lookup(&g.p.resolutions, &row.rule).cloned();
            let snap = g.sessions.get(&row.session).and_then(|s| {
                s.snapshots
                    .iter()
                    .rev()
                    .find(|x| !row.call.is_empty() && x.call == row.call)
                    .cloned()
            });
            let (manifest, command) = match snap {
                Some(s) => (s.manifest, s.command),
                None => (Vec::new(), None),
            };
            g.in_flight.push(row.digest.clone());
            jobs.push(Job::Stop {
                row,
                rule_node,
                manifest,
                command,
                prior,
            });
            continue;
        }
        let target = if row.kind == "PERMIT_GRANTED" {
            let want: String = row.stop_receipt.chars().take(16).collect();
            g.p.open.iter().find(|s| s.d16 == want).cloned()
        } else {
            g.p.open
                .iter()
                .rev()
                .find(|s| {
                    s.session == row.session
                        && s.ts <= row.ts
                        && ((row.via == "law_lift" && s.rule == row.rule)
                            || (!row.action.is_empty() && s.action == row.action))
                })
                .cloned()
        };
        match target {
            Some(stop) => {
                let rule_node = lookup(&g.p.rules, &stop.rule).cloned();
                g.in_flight.push(row.digest.clone());
                jobs.push(Job::Resolve {
                    row,
                    stop,
                    rule_node,
                });
            }
            None => {
                let r = &mut g.p.pending[i];
                r.tries += 1;
                if r.tries > RESOLUTION_MAX_TRIES {
                    give_up.push(r.digest.clone());
                }
            }
        }
    }
    if !give_up.is_empty() {
        g.p.pending.retain(|r| !give_up.contains(&r.digest));
        g.dirty = true;
    }
    jobs
}

struct Ctx<'a> {
    storage: &'a Arc<Storage>,
    cognitive: &'a Arc<Mutex<CognitiveEngine>>,
    output_config: &'a OutputConfig,
}

/// The memory carrying this exact tag, or None when no memory carries it.
async fn tagged(ctx: &Ctx<'_>, tag: &str) -> Result<Option<String>, String> {
    let out = super::recall::execute(
        ctx.storage,
        ctx.cognitive,
        ctx.output_config,
        Some(json!({"handle": tag, "detail_level": "brief", "limit": 5})),
    )
    .await;
    let v = match out {
        Ok(v) => v,
        Err(e) if e.contains("handle_required") => return Ok(None),
        Err(e) => return Err(e),
    };
    if v.get("error").and_then(Value::as_str) == Some("handle_required") {
        return Ok(None);
    }
    let nodes = v
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for node in nodes {
        let has = node
            .get("tags")
            .and_then(Value::as_array)
            .is_some_and(|t| t.iter().any(|x| x.as_str() == Some(tag)));
        if let Some(id) = node.get("id").and_then(Value::as_str)
            && has
            && is_memory_id(id)
        {
            return Ok(Some(id.to_string()));
        }
    }
    Ok(None)
}

async fn ingest(ctx: &Ctx<'_>, item: Value) -> Result<String, String> {
    let out = super::smart_ingest::execute(
        ctx.storage,
        ctx.cognitive,
        Some(json!({"items": [item], "outputProfile": "compact"})),
    )
    .await?;
    let res = out
        .get("results")
        .and_then(Value::as_array)
        .and_then(|r| r.first())
        .cloned()
        .unwrap_or(out);
    for key in ["nodeId", "echoNodeId", "duplicateOf"] {
        if let Some(id) = res.get(key).and_then(Value::as_str)
            && is_memory_id(id)
        {
            return Ok(id.to_string());
        }
    }
    Err(format!(
        "smart_ingest saved nothing: {}",
        res.to_string().chars().take(300).collect::<String>()
    ))
}

fn source_of(row: &GateRow) -> String {
    format!(
        "operator-gate receipt {}, vestige reflex {VERSION}",
        row.digest
    )
}

fn or_unknown(s: &str) -> &str {
    if s.is_empty() { "unknown" } else { s }
}

fn rule_item(row: &GateRow) -> Value {
    let rule = &row.rule;
    let name = if row.rule_name.is_empty() {
        String::new()
    } else {
        format!(" {}", row.rule_name)
    };
    json!({
        "node_type": "pattern",
        "tags": [rule, format!("op-rule-{rule}"), "operator", "operator-rule"],
        "source": source_of(row),
        "content": format!(
            "OPERATOR RULE {rule}{name}. Recorded the first time this rule stopped a call: {} UTC, host {}, \
             session {}. The gate's reason that time: {}. Every stop under this rule is a memory tagged \
             operator-stop and {rule} that is derived_from this memory, so a causal walk from any of those \
             stops reaches the rule.",
            row.ts,
            or_unknown(&row.source),
            or_unknown(&row.session.chars().take(8).collect::<String>()),
            if row.detail.is_empty() { "not given" } else { &row.detail },
        ),
    })
}

fn stop_item(row: &GateRow, rule_node: &str, manifest: &[String], command: Option<&str>) -> Value {
    let d16 = row.d16();
    let name = if row.rule_name.is_empty() {
        String::new()
    } else {
        format!(" {}", row.rule_name)
    };
    let mut content = format!(
        "OPERATOR STOP, recorded from the gate's own receipt. When: {} UTC. Host: {}, session {}. \
         Working directory: {}. Tool: {}. Rule: {}{name}. Command, as the gate's redacted preview: {}. \
         Why: {}. Receipt digest {d16}, chain prev {}, gate version {}, action id {}. Nothing was run. \
         Derived from the rule memory {rule_node}. The way forward is the compliant path the stop named, or a \
         one-time permit for that action id, which only the owner grants from their own terminal.",
        row.ts,
        or_unknown(&row.source),
        or_unknown(&row.session.chars().take(8).collect::<String>()),
        or_unknown(&row.cwd),
        or_unknown(&row.tool),
        row.rule,
        if row.preview.is_empty() {
            "none"
        } else {
            &row.preview
        },
        if row.detail.is_empty() {
            "not given"
        } else {
            &row.detail
        },
        if row.prev.is_empty() {
            "start".to_string()
        } else {
            row.prev.chars().take(16).collect()
        },
        or_unknown(&row.gate_version),
        if row.action.is_empty() {
            "none"
        } else {
            &row.action
        },
    );
    if !manifest.is_empty() {
        let shown: Vec<&str> = manifest
            .iter()
            .rev()
            .take(MANIFEST_IN_STOP)
            .map(String::as_str)
            .collect();
        content.push_str(&format!(
            " Memories in this agent's context at that tool call, newest first, {} of {}: {}.",
            shown.len(),
            manifest.len(),
            shown.join(", ")
        ));
    }
    if let Some(c) = command {
        content.push_str(&format!(" Command fingerprint {c}."));
    }
    json!({
        "node_type": "event",
        "tags": [row.rule, format!("op-receipt-{d16}"), "operator", "operator-stop"],
        "source": source_of(row),
        "links": [{"kind": "derived_from", "to": rule_node}],
        "content": content,
    })
}

fn resolution_item(row: &GateRow, stop: &OpenStop, rule_node: Option<&str>) -> Value {
    let d16 = row.d16();
    let mut links = vec![json!({"kind": "closes", "to": stop.node})];
    if let Some(r) = rule_node {
        links.push(json!({"kind": "derived_from", "to": r}));
    }
    json!({
        "node_type": "event",
        "tags": [stop.rule, format!("op-resolution-{d16}"), "operator", "operator-resolution"],
        "source": source_of(row),
        "links": links,
        "content": format!(
            "OPERATOR RESOLUTION, recorded from the gate's own receipt. When: {} UTC. Host: {}, session {}. \
             How the stop was answered: {}. This closes the stop {} (receipt {}) under {}. \
             Receipt digest {d16}, chain prev {}. The gate decided; this records how its stop was answered.",
            row.ts,
            or_unknown(&row.source),
            or_unknown(&row.session.chars().take(8).collect::<String>()),
            how_of(row),
            stop.node,
            stop.d16,
            stop.rule,
            if row.prev.is_empty() { "start".to_string() } else { row.prev.chars().take(16).collect() },
        ),
    })
}

async fn run_job(ctx: &Ctx<'_>, job: Job) -> Result<Outcome, (String, String)> {
    match job {
        Job::Stop {
            row,
            rule_node,
            manifest,
            command,
            prior,
        } => {
            let digest = row.digest.clone();
            let fail = |e: String| (digest.clone(), e);
            if let Some(node) = tagged(ctx, &format!("op-receipt-{}", row.d16()))
                .await
                .map_err(fail)?
            {
                let rule_node = match rule_node {
                    Some(r) => r,
                    None => tagged(ctx, &format!("op-rule-{}", row.rule))
                        .await
                        .map_err(fail)?
                        .unwrap_or_default(),
                };
                return Ok(Outcome::Stop {
                    row,
                    node,
                    rule_node,
                    command,
                    prior,
                    wrote: false,
                });
            }
            let rule_node = match rule_node {
                Some(r) => r,
                None => match tagged(ctx, &format!("op-rule-{}", row.rule))
                    .await
                    .map_err(fail)?
                {
                    Some(r) => r,
                    None => ingest(ctx, rule_item(&row)).await.map_err(fail)?,
                },
            };
            let node = ingest(
                ctx,
                stop_item(&row, &rule_node, &manifest, command.as_deref()),
            )
            .await
            .map_err(fail)?;
            Ok(Outcome::Stop {
                row,
                node,
                rule_node,
                command,
                prior,
                wrote: true,
            })
        }
        Job::Resolve {
            row,
            stop,
            rule_node,
        } => {
            let digest = row.digest.clone();
            let fail = |e: String| (digest.clone(), e);
            if let Some(node) = tagged(ctx, &format!("op-resolution-{}", row.d16()))
                .await
                .map_err(fail)?
            {
                return Ok(Outcome::Resolve {
                    row,
                    node,
                    stop,
                    wrote: false,
                });
            }
            let node = ingest(ctx, resolution_item(&row, &stop, rule_node.as_deref()))
                .await
                .map_err(fail)?;
            Ok(Outcome::Resolve {
                row,
                node,
                stop,
                wrote: true,
            })
        }
    }
}

fn job_row(job: &Job) -> &GateRow {
    match job {
        Job::Stop { row, .. } | Job::Resolve { row, .. } => row,
    }
}

/// Write other sessions' rows off the hook's path. At most one task per store;
/// rows it cannot take now are released for a later firing.
fn spawn_background(
    global: &Arc<std::sync::Mutex<Global>>,
    storage: &Arc<Storage>,
    cognitive: &Arc<Mutex<CognitiveEngine>>,
    output_config: &Arc<OutputConfig>,
    jobs: Vec<Job>,
) {
    if jobs.is_empty() {
        return;
    }
    {
        let Ok(mut g) = global.lock() else {
            return;
        };
        if g.bg_running || tokio::runtime::Handle::try_current().is_err() {
            for j in &jobs {
                let d = job_row(j).digest.clone();
                g.in_flight.retain(|x| *x != d);
            }
            return;
        }
        g.bg_running = true;
    }
    let global = Arc::clone(global);
    let storage = Arc::clone(storage);
    let cognitive = Arc::clone(cognitive);
    let output_config = Arc::clone(output_config);
    tokio::spawn(async move {
        let started = Instant::now();
        let ctx = Ctx {
            storage: &storage,
            cognitive: &cognitive,
            output_config: &output_config,
        };
        let mut outcomes = Vec::new();
        for job in jobs {
            match run_job(&ctx, job).await {
                Ok(o) => outcomes.push(o),
                Err((digest, e)) => {
                    tracing::warn!(error = %e, "reflex background write waits for a later firing");
                    outcomes.push(Outcome::Wait { digest });
                }
            }
        }
        let Ok(mut g) = global.lock() else {
            return;
        };
        let mut wrote = Vec::new();
        for o in outcomes {
            if let Some(entry) = apply(&mut g, o) {
                wrote.push(entry);
            }
        }
        if !wrote.is_empty() {
            let inp = Input {
                event: "background".into(),
                host: "reflex".into(),
                ..Input::default()
            };
            append_ledger(&mut g, &inp, 0, &[], &wrote, started.elapsed().as_millis());
            g.persist(true);
        }
        g.bg_running = false;
    });
}

/// Fold one finished job back into the shared state; returns the ledger entry.
fn apply(g: &mut Global, outcome: Outcome) -> Option<Value> {
    match outcome {
        Outcome::Wait { digest } => {
            g.in_flight.retain(|d| *d != digest);
            None
        }
        Outcome::Stop {
            row,
            node,
            rule_node,
            command,
            prior,
            wrote,
        } => {
            let d16 = row.d16();
            g.in_flight.retain(|d| *d != row.digest);
            g.p.pending.retain(|r| r.digest != row.digest);
            upsert(&mut g.p.done, &d16, node.clone(), MAX_DONE);
            if !rule_node.is_empty() {
                upsert(&mut g.p.rules, &row.rule, rule_node.clone(), 1000);
            }
            if let Some(c) = &command {
                upsert(&mut g.p.commands, c, d16.clone(), MAX_COMMANDS);
            }
            g.p.open.retain(|s| s.d16 != d16);
            g.p.open.push(OpenStop {
                d16: d16.clone(),
                node: node.clone(),
                session: row.session.clone(),
                rule: row.rule.clone(),
                action: row.action.clone(),
                ts: row.ts.clone(),
            });
            if g.p.open.len() > MAX_OPEN {
                let drop = g.p.open.len() - MAX_OPEN;
                g.p.open.drain(..drop);
            }
            g.p.recent.retain(|r| r.d16 != d16);
            g.p.recent.push(Recent {
                d16: d16.clone(),
                node: node.clone(),
                rule: row.rule.clone(),
                ts: row.ts.clone(),
                preview: row.preview.chars().take(120).collect(),
            });
            if g.p.recent.len() > RECENT_KEEP {
                let drop = g.p.recent.len() - RECENT_KEEP;
                g.p.recent.drain(..drop);
            }
            if !row.session.is_empty() {
                let note = StopNote {
                    d16: d16.clone(),
                    node: node.clone(),
                    rule: row.rule.clone(),
                    rule_node: rule_node.clone(),
                    preview: row.preview.chars().take(160).collect(),
                    detail: row.detail.chars().take(200).collect(),
                    ts: row.ts.clone(),
                    resolution: prior,
                };
                let s = g.session(&row.session);
                if !s.stops.iter().any(|x| x.d16 == d16) {
                    s.stops.push(note);
                }
            }
            g.dirty = true;
            Some(
                json!({"kind": "stop", "receipt": d16, "memory": node, "rule": row.rule, "new": wrote}),
            )
        }
        Outcome::Resolve {
            row,
            node,
            stop,
            wrote,
        } => {
            g.in_flight.retain(|d| *d != row.digest);
            g.p.pending.retain(|r| r.digest != row.digest);
            g.p.open.retain(|s| s.d16 != stop.d16);
            upsert(
                &mut g.p.resolutions,
                &stop.rule,
                Resolution {
                    node: node.clone(),
                    how: how_of(&row),
                    ts: row.ts.clone(),
                    stop_d16: stop.d16.clone(),
                },
                1000,
            );
            g.dirty = true;
            Some(
                json!({"kind": "resolution", "receipt": row.d16(), "memory": node, "closes": stop.node, "new": wrote}),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// What the agent is told
// ---------------------------------------------------------------------------

fn command_hash(command: &str) -> String {
    let normalized = command.split_whitespace().collect::<Vec<_>>().join(" ");
    sha2::Sha256::digest(normalized.as_bytes())
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn hhmm(ts: &str) -> String {
    ts.get(11..16)
        .map(|t| format!("{t}Z"))
        .unwrap_or_else(|| ts.to_string())
}

fn stop_line(n: &StopNote) -> String {
    let mut line = format!(
        "Operator stopped `{}` under {} at {} (receipt {}). Saved as {}{}.",
        if n.preview.is_empty() {
            "(no preview)"
        } else {
            &n.preview
        },
        n.rule,
        hhmm(&n.ts),
        n.d16,
        n.node,
        if n.rule_node.is_empty() {
            String::new()
        } else {
            format!(", derived from its rule {}", n.rule_node)
        },
    );
    if !n.detail.is_empty() {
        line.push_str(&format!(" Why: {}.", n.detail.trim_end_matches('.')));
    }
    if let Some(r) = &n.resolution {
        line.push_str(&format!(
            " Last time {} stopped a call it was answered by {} ({}).",
            n.rule,
            r.how.trim_end_matches('.'),
            r.node
        ));
    }
    line.push_str(" Take the safe path the stop named, or ask the owner for a one-time permit.");
    line
}

/// Compose this firing's context within the event and turn budgets, and mark
/// what it tells. Returns the text and the receipts told.
fn compose(g: &mut Global, inp: &Input) -> (String, Vec<String>) {
    let budget = event_budget(&inp.event);
    if budget == 0 || inp.session.is_empty() {
        return (String::new(), Vec::new());
    }
    let recent = g.p.recent.clone();
    let commands = if inp.event == "PreToolUse" && !inp.command.is_empty() {
        let h = command_hash(&inp.command);
        lookup(&g.p.commands, &h).and_then(|d16| {
            let node = lookup(&g.p.done, d16)?.clone();
            let r = recent.iter().find(|r| r.d16 == *d16).cloned();
            Some((d16.clone(), node, r))
        })
    } else {
        None
    };
    let s = g.session(&inp.session);
    let cap = budget.min(TURN_BUDGET.saturating_sub(s.turn_chars));
    if cap == 0 {
        return (String::new(), Vec::new());
    }
    let mut lines: Vec<(String, Option<String>)> = Vec::new();
    match inp.event.as_str() {
        "SessionStart" if inp.source == "compact" => {
            for n in s.stops.iter().filter(|n| s.told.contains(&n.d16)) {
                lines.push((format!("Before compaction: {}", stop_line(n)), None));
            }
        }
        "SessionStart" => {
            let now = Utc::now().timestamp();
            let fresh: Vec<&Recent> = recent
                .iter()
                .rev()
                .filter(|r| {
                    DateTime::parse_from_rfc3339(&r.ts)
                        .map(|t| now - t.timestamp() <= RECENT_DAYS * 86_400)
                        .unwrap_or(false)
                })
                .take(RECENT_SHOWN)
                .collect();
            if !fresh.is_empty() {
                lines.push((
                    format!(
                        "Operator stopped {} action{} on this machine in the last {RECENT_DAYS} days. Each stop is a Vestige memory; recall its handle to see why, and take the safe path it names instead of trying the same thing again:",
                        fresh.len(),
                        if fresh.len() == 1 { "" } else { "s" }
                    ),
                    None,
                ));
                for r in fresh {
                    lines.push((
                        format!(
                            "- {} {}: {} -> recall(handle=\"op-receipt-{}\")",
                            r.rule,
                            r.ts,
                            if r.preview.is_empty() {
                                "(no preview)"
                            } else {
                                &r.preview
                            },
                            r.d16
                        ),
                        None,
                    ));
                }
            }
        }
        "SubagentStart" => {
            let told: Vec<&StopNote> = s.stops.iter().filter(|n| s.told.contains(&n.d16)).collect();
            if !told.is_empty() {
                lines.push((
                    "Operator stops in this session so far, which hold for you too:".to_string(),
                    None,
                ));
                for n in told.iter().rev().take(3) {
                    lines.push((format!("- {}", stop_line(n)), None));
                }
            }
        }
        "PreToolUse" => {
            // A stop the agent has not heard about yet goes first: this is the
            // last moment before its next tool call runs.
            for n in s.stops.iter().filter(|n| !s.told.contains(&n.d16)) {
                lines.push((stop_line(n), Some(n.d16.clone())));
            }
            if let Some((d16, node, r)) = commands {
                let (rule, ts) = r.map(|r| (r.rule, r.ts)).unwrap_or_default();
                lines.push((
                    format!(
                        "Operator already stopped this exact command{} (receipt {d16}, memory {node}). It will be stopped again. Take the safe path it named, or ask the owner for a one-time permit.",
                        if rule.is_empty() { String::new() } else { format!(" under {rule} at {}", hhmm(&ts)) }
                    ),
                    None,
                ));
            }
        }
        _ => {
            for n in s.stops.iter().filter(|n| !s.told.contains(&n.d16)) {
                lines.push((stop_line(n), Some(n.d16.clone())));
            }
        }
    }
    let mut text = String::new();
    let mut told = Vec::new();
    for (line, d16) in lines {
        let add = if text.is_empty() {
            line.len()
        } else {
            line.len() + 1
        };
        if text.len() + add > cap {
            break;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&line);
        if let Some(d) = d16 {
            told.push(d);
        }
    }
    s.turn_chars += text.len();
    for d in &told {
        if !s.told.contains(d) {
            s.told.push(d.clone());
        }
    }
    let told_nodes: Vec<String> = s
        .stops
        .iter()
        .filter(|n| told.contains(&n.d16))
        .map(|n| n.node.clone())
        .collect();
    for id in told_nodes {
        s.remember(id);
    }
    (text, told)
}

// ---------------------------------------------------------------------------
// The Hook Ledger
// ---------------------------------------------------------------------------

fn ledger_digest(row: &Value) -> String {
    let mut body = row.clone();
    if let Some(o) = body.as_object_mut() {
        o.remove("digest");
    }
    sha2::Sha256::digest(body.to_string().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn last_ledger_digest(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.lines().rev().find(|l| !l.trim().is_empty())?;
    serde_json::from_str::<Value>(line)
        .ok()?
        .get("digest")?
        .as_str()
        .map(str::to_string)
}

fn append_ledger(
    g: &mut Global,
    inp: &Input,
    chars: usize,
    told: &[String],
    wrote: &[Value],
    ms: u128,
) {
    if std::fs::create_dir_all(&g.dir).is_err() {
        return;
    }
    let path = g.dir.join("ledger.jsonl");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LEDGER_ROTATE_BYTES) {
        let _ = std::fs::rename(&path, g.dir.join("ledger.jsonl.1"));
    }
    let mut row = json!({
        "v": 1,
        "ts": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "event": inp.event,
        "session": inp.session,
        "host": inp.host,
        "tool": inp.tool,
        "call": inp.call,
        "chars": chars,
        "told": told,
        "wrote": wrote,
        "pending": g.p.pending.len(),
        "chainBreaks": g.p.chain_breaks,
        "ms": ms as u64,
        "prev": g.p.ledger_tip.clone().unwrap_or_else(|| "0".repeat(64)),
    });
    let digest = ledger_digest(&row);
    row["digest"] = json!(digest);
    let line = format!("{row}\n");
    let ok = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(line.as_bytes()))
        .is_ok();
    if ok {
        g.p.ledger_tip = Some(digest);
    }
}

fn parse_since(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() {
        return Some(86_400);
    }
    let unit = t.chars().last()?;
    let num = &t[..t.len() - unit.len_utf8()];
    let mult = match unit {
        'm' => 60,
        'h' => 3600,
        'd' => 86_400,
        'w' => 604_800,
        _ => return None,
    };
    num.trim().parse::<i64>().ok().map(|n| n * mult)
}

fn ledger_view(storage: &Arc<Storage>, inp: &Input) -> Value {
    ledger_at(&storage.data_dir().join("reflex"), &inp.since)
}

/// What reflex has recorded for the store at `data_dir`: the Hook Ledger for a
/// window, checked link by link, and the receipt reader's own counters. Reads
/// only `<data_dir>/reflex/`, so it works while a server owns the store.
pub fn report(data_dir: &Path, since: &str) -> Value {
    let dir = data_dir.join("reflex");
    let mut view = ledger_at(&dir, since);
    let state = std::fs::read_to_string(dir.join("state.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Persisted>(&t).ok());
    let heartbeat = std::fs::read_to_string(dir.join("heartbeat"))
        .ok()
        .and_then(|t| t.trim().parse::<i64>().ok());
    if let Some(o) = view.as_object_mut() {
        o.insert(
            "receipts".into(),
            match state {
                Some(p) => json!({
                    "linesRead": p.lines_read,
                    "chainBreaks": p.chain_breaks,
                    "filesTracked": p.files.len(),
                    "stopsRecorded": p.done.len(),
                    "rules": p.rules.len(),
                    "openStops": p.open.len(),
                    "pending": p.pending.len(),
                }),
                None => json!({"note": "reflex has not run against this store yet"}),
            },
        );
        o.insert(
            "lastFiringSecondsAgo".into(),
            json!(heartbeat.map(|h| Utc::now().timestamp() - h)),
        );
    }
    view
}

fn ledger_at(dir: &Path, since: &str) -> Value {
    let path = dir.join("ledger.jsonl");
    let since_label = if since.is_empty() { "24h" } else { since };
    let Some(window) = parse_since(since) else {
        return json!({"error": "invalid since: use forms like 30m, 24h, 7d"});
    };
    let cutoff = Utc::now().timestamp() - window;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut prev: Option<String> = None;
    let mut checked = 0usize;
    let mut breaks = 0usize;
    let mut firings = 0usize;
    let mut chars = 0u64;
    let mut by_event: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    let mut stops = 0u64;
    let mut resolutions = 0u64;
    let mut told = 0u64;
    let mut sessions: Vec<String> = Vec::new();
    let mut last: VecDeque<Value> = VecDeque::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(row) = serde_json::from_str::<Value>(line) else {
            breaks += 1;
            prev = None;
            continue;
        };
        checked += 1;
        let stored = row
            .get("digest")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let linked = prev
            .as_deref()
            .is_none_or(|p| row.get("prev").and_then(Value::as_str) == Some(p));
        if !linked || ledger_digest(&row) != stored {
            breaks += 1;
        }
        prev = Some(stored);
        let ts = row
            .get("ts")
            .and_then(Value::as_str)
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.timestamp())
            .unwrap_or(0);
        if ts < cutoff {
            continue;
        }
        firings += 1;
        chars += row.get("chars").and_then(Value::as_u64).unwrap_or(0);
        *by_event
            .entry(
                row.get("event")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            )
            .or_default() += 1;
        told += row
            .get("told")
            .and_then(Value::as_array)
            .map_or(0, |a| a.len() as u64);
        for w in row
            .get("wrote")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            if w.get("new").and_then(Value::as_bool) == Some(true) {
                match w.get("kind").and_then(Value::as_str) {
                    Some("stop") => stops += 1,
                    Some("resolution") => resolutions += 1,
                    _ => {}
                }
            }
        }
        if let Some(s) = row.get("session").and_then(Value::as_str)
            && !s.is_empty()
            && !sessions.iter().any(|x| x == s)
        {
            sessions.push(s.to_string());
        }
        last.push_back(row);
        if last.len() > 10 {
            last.pop_front();
        }
    }
    json!({
        "ledger": path.display().to_string(),
        "since": since_label,
        "firings": firings,
        "byEvent": by_event,
        "charsTold": chars,
        "stopsTold": told,
        "stopsRecorded": stops,
        "resolutionsRecorded": resolutions,
        "sessions": sessions.len(),
        "chain": {
            "status": if checked == 0 { "empty" } else if breaks == 0 { "ok" } else { "broken" },
            "linesChecked": checked,
            "breaks": breaks,
            "integrity": "reference_digest_not_signature",
        },
        "recent": last,
    })
}

fn status_view(g: &Global, home: &Path) -> Value {
    json!({
        "gateHome": home.display().to_string(),
        "receiptFilesTracked": g.p.files.len(),
        "receiptLinesRead": g.p.lines_read,
        "receiptChainBreaks": g.p.chain_breaks,
        "pending": g.p.pending.len(),
        "stopsRecorded": g.p.done.len(),
        "rules": g.p.rules.len(),
        "openStops": g.p.open.len(),
        "resolutions": g.p.resolutions.len(),
        "sessions": g.sessions.len(),
        "version": VERSION,
    })
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

fn shape(inp: &Input, text: &str) -> Value {
    if text.is_empty() {
        return json!({});
    }
    if inp.format == "plain" {
        return json!({"context": text, "event": inp.event});
    }
    json!({"hookSpecificOutput": {"hookEventName": inp.event, "additionalContext": text}})
}

/// One hook firing, against the gate home at `$OPERATOR_HOME` or `~/.operator`.
pub async fn execute(
    storage: &Arc<Storage>,
    cognitive: &Arc<Mutex<CognitiveEngine>>,
    output_config: &Arc<OutputConfig>,
    conn: &ConnectionState,
    args: Option<Value>,
) -> Result<Value, String> {
    let home = super::precedent::operator_home();
    execute_with(storage, cognitive, output_config, conn, args, &home).await
}

/// One hook firing against an explicit gate home. Never fails the hook: an
/// internal error is logged and answered with an empty object.
pub async fn execute_with(
    storage: &Arc<Storage>,
    cognitive: &Arc<Mutex<CognitiveEngine>>,
    output_config: &Arc<OutputConfig>,
    conn: &ConnectionState,
    args: Option<Value>,
    home: &Path,
) -> Result<Value, String> {
    let inp = Input::parse(args.as_ref());
    if inp.action == "ledger" {
        return Ok(ledger_view(storage, &inp));
    }
    let global = global_for(storage);
    if inp.action == "status" {
        let g = global.lock().map_err(|e| e.to_string())?;
        return Ok(status_view(&g, home));
    }
    let started = Instant::now();
    if !inp.session.is_empty()
        && let Ok(mut c) = conn.lock()
    {
        c.session = Some(inp.session.clone());
    }
    let jobs = {
        let mut g = global.lock().map_err(|e| e.to_string())?;
        if !inp.session.is_empty() {
            let s = g.session(&inp.session);
            if inp.event == "UserPromptSubmit" {
                s.turn_chars = 0;
            }
            if inp.event == "PreToolUse" && !inp.call.is_empty() {
                let snap = Snapshot {
                    call: inp.call.clone(),
                    command: (!inp.command.is_empty()).then(|| command_hash(&inp.command)),
                    manifest: s.manifest.iter().cloned().collect(),
                };
                s.snapshots.push_back(snap);
                while s.snapshots.len() > SNAPSHOT_KEEP {
                    s.snapshots.pop_front();
                }
            }
        }
        scan(&mut g, home);
        take_jobs(&mut g, &inp.session)
    };
    // Only the calling session's rows are written before this hook answers.
    // Everything else drains in one background task per store, so a backlog
    // never slows a tool call.
    let (urgent, background): (Vec<Job>, Vec<Job>) = jobs
        .into_iter()
        .partition(|j| !inp.session.is_empty() && job_row(j).session == inp.session);
    let ctx = Ctx {
        storage,
        cognitive,
        output_config,
    };
    let mut outcomes = Vec::new();
    for job in urgent {
        if started.elapsed().as_millis() > WRITE_BUDGET_MS {
            outcomes.push(Outcome::Wait {
                digest: job_row(&job).digest.clone(),
            });
            continue;
        }
        match run_job(&ctx, job).await {
            Ok(o) => outcomes.push(o),
            Err((digest, e)) => {
                tracing::warn!(error = %e, "reflex could not record a gate receipt; it waits for the next firing");
                outcomes.push(Outcome::Wait { digest });
            }
        }
    }
    spawn_background(&global, storage, cognitive, output_config, background);
    let mut g = global.lock().map_err(|e| e.to_string())?;
    let mut wrote = Vec::new();
    for o in outcomes {
        if let Some(entry) = apply(&mut g, o) {
            wrote.push(entry);
        }
    }
    let (text, told) = compose(&mut g, &inp);
    append_ledger(
        &mut g,
        &inp,
        text.len(),
        &told,
        &wrote,
        started.elapsed().as_millis(),
    );
    let force = wrote
        .iter()
        .any(|w| w.get("new").and_then(Value::as_bool) == Some(true));
    g.dirty = g.dirty || !wrote.is_empty();
    g.persist(force);
    g.heartbeat();
    Ok(shape(&inp, &text))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn cognitive() -> Arc<Mutex<CognitiveEngine>> {
        Arc::new(Mutex::new(CognitiveEngine::new()))
    }

    struct Fixture {
        storage: Arc<Storage>,
        home: PathBuf,
        _store_dir: tempfile::TempDir,
        _home_dir: tempfile::TempDir,
        tip: String,
        conn: ConnectionState,
        cog: Arc<Mutex<CognitiveEngine>>,
        oc: Arc<OutputConfig>,
    }

    impl Fixture {
        fn new() -> Fixture {
            let store_dir = tempfile::TempDir::new().unwrap();
            let home_dir = tempfile::TempDir::new().unwrap();
            let storage =
                vestige_core::open_storage(Some(store_dir.path().join("test.db"))).unwrap();
            std::fs::create_dir_all(home_dir.path().join("receipts")).unwrap();
            Fixture {
                storage,
                home: home_dir.path().to_path_buf(),
                _store_dir: store_dir,
                _home_dir: home_dir,
                tip: "0".repeat(64),
                conn: new_connection(),
                cog: cognitive(),
                oc: Arc::new(OutputConfig::default()),
            }
        }

        fn day(&self) -> PathBuf {
            self.home
                .join("receipts")
                .join(format!("{}.jsonl", Utc::now().format("%Y-%m-%d")))
        }

        /// Append one receipt, chained and digested the way the gate does it.
        fn receipt(&mut self, mut rec: Value) -> String {
            rec["prev"] = json!(self.tip);
            rec["ts"] = json!(Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
            let digest = super::super::precedent::gate_digest(&rec).unwrap();
            rec["digest"] = json!(digest);
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.day())
                .unwrap();
            writeln!(f, "{rec}").unwrap();
            self.tip = digest.clone();
            digest
        }

        fn forged(&mut self, mut rec: Value) {
            rec["prev"] = json!(self.tip);
            rec["ts"] = json!(Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
            rec["digest"] = json!("f".repeat(64));
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.day())
                .unwrap();
            writeln!(f, "{rec}").unwrap();
            self.tip = "f".repeat(64);
        }

        async fn fire(&self, args: Value) -> Value {
            execute_with(
                &self.storage,
                &self.cog,
                &self.oc,
                &self.conn,
                Some(args),
                &self.home,
            )
            .await
            .unwrap()
        }

        /// Wait for the background drain to finish.
        async fn settle(&self) {
            let global = global_for(&self.storage);
            for _ in 0..1000 {
                if !global.lock().unwrap().bg_running {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            panic!("background drain did not finish");
        }

        async fn tag(&self, tag: &str) -> Option<String> {
            let ctx = Ctx {
                storage: &self.storage,
                cognitive: &self.cog,
                output_config: &self.oc,
            };
            tagged(&ctx, tag).await.unwrap()
        }
    }

    fn stop(session: &str, rule: &str, call: &str, preview: &str) -> Value {
        json!({
            "v": "1", "decision": "STOP", "commitment": rule, "rule": "no-blind-recursive-delete",
            "session": session, "source": "claude", "tool": "Bash", "call": call, "cwd": "/repo",
            "action_preview": preview, "detail": "recursive delete outside the project",
            "action_digest": format!("act-{call}"), "gate_version": "test"
        })
    }

    fn context(v: &Value) -> String {
        v.pointer("/hookSpecificOutput/additionalContext")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }

    #[tokio::test]
    async fn a_stop_becomes_a_memory_linked_to_its_rule_and_is_told_once() {
        let mut fx = Fixture::new();
        let digest = fx.receipt(stop("s1", "OP-003", "toolu_1", "rm -rf ~/work"));
        let out = fx
            .fire(json!({"event": "PostToolUse", "session": "s1", "tool": "Bash"}))
            .await;
        let text = context(&out);
        assert!(text.contains("rm -rf ~/work"), "{out}");
        assert!(text.contains(&digest[..16]), "{out}");
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        let stop_node = fx.tag(&format!("op-receipt-{}", &digest[..16])).await;
        let rule_node = fx.tag("op-rule-OP-003").await;
        assert!(stop_node.is_some() && rule_node.is_some());
        assert!(text.contains(stop_node.as_deref().unwrap()));
        let again = fx
            .fire(json!({"event": "PostToolUse", "session": "s1"}))
            .await;
        assert_eq!(again, json!({}), "a stop is told once");
    }

    #[tokio::test]
    async fn another_session_is_not_told_but_the_stop_is_still_recorded() {
        let mut fx = Fixture::new();
        let digest = fx.receipt(stop("s1", "OP-003", "toolu_1", "rm -rf ~/work"));
        let out = fx
            .fire(json!({"event": "PostToolUse", "session": "other"}))
            .await;
        assert_eq!(out, json!({}));
        fx.settle().await;
        assert!(
            fx.tag(&format!("op-receipt-{}", &digest[..16]))
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn a_line_that_breaks_the_chain_never_becomes_a_memory() {
        let mut fx = Fixture::new();
        fx.forged(stop("s1", "OP-003", "toolu_9", "rm -rf ~/forged"));
        let out = fx
            .fire(json!({"event": "PostToolUse", "session": "s1"}))
            .await;
        assert_eq!(out, json!({}));
        let status = fx.fire(json!({"action": "status"})).await;
        assert_eq!(status["receiptChainBreaks"], 1);
        assert_eq!(status["stopsRecorded"], 0);
    }

    #[tokio::test]
    async fn an_existing_stop_memory_is_reused_not_written_twice() {
        let mut fx = Fixture::new();
        let digest = fx.receipt(stop("s1", "OP-003", "toolu_1", "rm -rf ~/work"));
        let ctx = Ctx {
            storage: &fx.storage,
            cognitive: &fx.cog,
            output_config: &fx.oc,
        };
        let existing = ingest(
            &ctx,
            json!({"content": "stop written by the python hook", "node_type": "event",
                   "tags": ["OP-003", format!("op-receipt-{}", &digest[..16]), "operator", "operator-stop"]}),
        )
        .await
        .unwrap();
        let out = fx
            .fire(json!({"event": "PostToolUse", "session": "s1"}))
            .await;
        assert!(context(&out).contains(&existing), "{out}");
        let ledger = fx.fire(json!({"action": "ledger"})).await;
        assert_eq!(ledger["stopsRecorded"], 0, "{ledger}");
    }

    #[tokio::test]
    async fn a_permit_closes_its_stop_and_the_next_stop_names_that_answer() {
        let mut fx = Fixture::new();
        let first = fx.receipt(stop("s1", "OP-011", "toolu_1", "git reset --hard"));
        fx.fire(json!({"event": "PostToolUse", "session": "s1"}))
            .await;
        let permit = fx.receipt(json!({
            "v": "1", "decision": "PERMIT_GRANTED", "source": "owner-tty", "session": "s1",
            "action_digest": "act-toolu_1", "stop_receipt": first, "single_use": "True"
        }));
        fx.fire(json!({"event": "UserPromptSubmit", "session": "s1"}))
            .await;
        assert!(
            fx.tag(&format!("op-resolution-{}", &permit[..16]))
                .await
                .is_some()
        );
        fx.receipt(stop("s1", "OP-011", "toolu_2", "git reset --hard HEAD~3"));
        let out = fx
            .fire(json!({"event": "PostToolUse", "session": "s1"}))
            .await;
        assert!(
            context(&out).contains("one-time permit from their own terminal"),
            "{out}"
        );
    }

    #[tokio::test]
    async fn a_law_lift_allow_resolves_the_newest_stop_of_that_rule() {
        let mut fx = Fixture::new();
        fx.receipt(stop(
            "s1",
            "recall-before-grep",
            "toolu_1",
            "grep -rn foo .",
        ));
        fx.fire(json!({"event": "PostToolUse", "session": "s1"}))
            .await;
        let allow = fx.receipt(json!({
            "v": "1", "decision": "ALLOW", "commitment": "recall-before-grep", "via": "law_lift",
            "session": "s1", "source": "claude", "tool": "Bash", "action_preview": "grep -rn foo src"
        }));
        fx.fire(json!({"event": "PostToolUse", "session": "s1"}))
            .await;
        assert!(
            fx.tag(&format!("op-resolution-{}", &allow[..16]))
                .await
                .is_some()
        );
        let status = fx.fire(json!({"action": "status"})).await;
        assert_eq!(status["openStops"], 0, "{status}");
    }

    #[tokio::test]
    async fn the_exact_command_stopped_before_is_flagged_on_its_next_attempt() {
        let mut fx = Fixture::new();
        fx.fire(json!({"event": "PreToolUse", "session": "s1", "call": "toolu_1", "command": "rm -rf  ~/work"})).await;
        fx.receipt(stop("s1", "OP-003", "toolu_1", "rm -rf ~/work"));
        fx.fire(json!({"event": "PostToolUse", "session": "s1"}))
            .await;
        let warn = fx
            .fire(json!({"event": "PreToolUse", "session": "s1", "call": "toolu_2", "command": "rm -rf ~/work"}))
            .await;
        assert!(
            context(&warn).contains("already stopped this exact command"),
            "{warn}"
        );
        let other = fx
            .fire(json!({"event": "PreToolUse", "session": "s1", "call": "toolu_3", "command": "ls ~/work"}))
            .await;
        assert_eq!(other, json!({}));
    }

    #[tokio::test]
    async fn a_stop_records_the_memories_the_agent_held_at_that_call() {
        let mut fx = Fixture::new();
        fx.fire(json!({"event": "SessionStart", "session": "s1", "source": "startup"}))
            .await;
        observe(
            &fx.conn,
            &fx.storage,
            "recall",
            &json!({"nodes": [{"id": "mem-00000000000000aa"}]}),
        );
        fx.fire(json!({"event": "PreToolUse", "session": "s1", "call": "toolu_1", "command": "rm -rf ~/work"})).await;
        let digest = fx.receipt(stop("s1", "OP-003", "toolu_1", "rm -rf ~/work"));
        fx.fire(json!({"event": "PostToolUse", "session": "s1"}))
            .await;
        let id = fx
            .tag(&format!("op-receipt-{}", &digest[..16]))
            .await
            .unwrap();
        let ctx = Ctx {
            storage: &fx.storage,
            cognitive: &fx.cog,
            output_config: &fx.oc,
        };
        let got = super::super::recall::execute(
            ctx.storage,
            ctx.cognitive,
            ctx.output_config,
            Some(json!({"handle": id, "detail_level": "full"})),
        )
        .await
        .unwrap();
        assert!(got.to_string().contains("mem-00000000000000aa"), "{got}");
    }

    #[tokio::test]
    async fn context_stays_inside_the_event_budget() {
        let mut fx = Fixture::new();
        for i in 0..8 {
            fx.receipt(stop(
                "s1",
                "OP-003",
                &format!("toolu_{i}"),
                &"x".repeat(150),
            ));
        }
        let out = fx
            .fire(json!({"event": "PostToolUse", "session": "s1"}))
            .await;
        assert!(
            context(&out).len() <= event_budget("PostToolUse"),
            "{}",
            context(&out).len()
        );
        let next = fx
            .fire(json!({"event": "PostToolUse", "session": "s1"}))
            .await;
        assert!(
            !context(&next).is_empty(),
            "the rest are told on later firings"
        );
    }

    #[tokio::test]
    async fn the_hook_ledger_chains_every_firing_and_detects_an_edit() {
        let mut fx = Fixture::new();
        fx.receipt(stop("s1", "OP-003", "toolu_1", "rm -rf ~/work"));
        fx.fire(json!({"event": "PostToolUse", "session": "s1"}))
            .await;
        fx.fire(json!({"event": "Stop", "session": "s1"})).await;
        let view = fx.fire(json!({"action": "ledger"})).await;
        assert_eq!(view["chain"]["status"], "ok", "{view}");
        assert_eq!(view["firings"], 2);
        assert_eq!(view["stopsRecorded"], 1);
        let path = fx.storage.data_dir().join("reflex").join("ledger.jsonl");
        let edited = std::fs::read_to_string(&path)
            .unwrap()
            .replacen("\"Stop\"", "\"Stip\"", 1);
        std::fs::write(&path, edited).unwrap();
        let view = fx.fire(json!({"action": "ledger"})).await;
        assert_eq!(view["chain"]["status"], "broken", "{view}");
    }

    #[tokio::test]
    async fn events_without_context_and_unfilled_substitutions_answer_empty() {
        let fx = Fixture::new();
        assert_eq!(
            fx.fire(json!({"event": "Stop", "session": "s1"})).await,
            json!({})
        );
        assert_eq!(
            fx.fire(json!({"event": "PostToolUse", "session": "${session_id}"}))
                .await,
            json!({})
        );
        assert_eq!(fx.fire(json!({})).await, json!({}));
    }

    #[tokio::test]
    async fn plain_format_answers_for_the_command_shim() {
        let mut fx = Fixture::new();
        fx.receipt(stop("s1", "OP-003", "toolu_1", "rm -rf ~/work"));
        let out = fx
            .fire(json!({"event": "PostToolUse", "session": "s1", "format": "plain"}))
            .await;
        assert!(
            out["context"].as_str().unwrap().contains("rm -rf ~/work"),
            "{out}"
        );
        assert!(out.get("hookSpecificOutput").is_none());
    }

    #[tokio::test]
    async fn pre_tool_use_tells_an_untold_stop_before_the_next_call() {
        let mut fx = Fixture::new();
        let digest = fx.receipt(stop("s1", "OP-003", "toolu_1", "rm -rf ~/work"));
        let out = fx
            .fire(json!({"event": "PreToolUse", "session": "s1", "call": "toolu_2", "command": "echo next"}))
            .await;
        assert!(context(&out).contains(&digest[..16]), "{out}");
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    }

    #[tokio::test]
    async fn other_sessions_backlog_drains_in_the_background_and_is_ledgered() {
        let mut fx = Fixture::new();
        let a = fx.receipt(stop("other-1", "OP-003", "toolu_1", "rm -rf ~/a"));
        let b = fx.receipt(stop(
            "other-2",
            "OP-004",
            "toolu_2",
            "git reflog expire --all",
        ));
        let out = fx
            .fire(json!({"event": "PostToolUse", "session": "s1"}))
            .await;
        assert_eq!(out, json!({}), "nothing to tell this session");
        fx.settle().await;
        assert!(fx.tag(&format!("op-receipt-{}", &a[..16])).await.is_some());
        assert!(fx.tag(&format!("op-receipt-{}", &b[..16])).await.is_some());
        let view = fx.fire(json!({"action": "ledger"})).await;
        assert_eq!(view["stopsRecorded"], 2, "{view}");
        assert_eq!(view["byEvent"]["background"], 1, "{view}");
        assert_eq!(view["chain"]["status"], "ok", "{view}");
    }

    #[test]
    fn memory_ids_and_rules_are_recognized_exactly() {
        assert!(is_memory_id("mem-00000000000acd2b"));
        assert!(is_memory_id("68540393-203d-428b-9a1a-854da288e655"));
        assert!(!is_memory_id("mem-123"));
        assert!(!is_memory_id("op-receipt-b2c87261ed002a83"));
        assert!(is_rule("OP-003") && is_rule("recall-before-grep"));
        assert!(!is_rule("-x") && !is_rule("bad rule"));
    }

    #[test]
    fn rows_that_answer_no_stop_are_not_queued() {
        let d = "a".repeat(64);
        assert!(GateRow::from_receipt(&json!({"decision": "PASS", "digest": d})).is_none());
        assert!(GateRow::from_receipt(&json!({"decision": "ALLOW", "digest": d})).is_none());
        assert!(
            GateRow::from_receipt(&json!({"decision": "ALLOW", "digest": d, "via": "law_lift"}))
                .is_some()
        );
        assert!(
            GateRow::from_receipt(&json!({"decision": "PERMIT_GRANTED", "digest": d})).is_none()
        );
        assert!(
            GateRow::from_receipt(
                &json!({"decision": "STOP", "digest": d, "commitment": "bad rule"})
            )
            .is_none()
        );
    }
}
