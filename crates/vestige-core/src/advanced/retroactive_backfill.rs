//! # Retroactive Salience Backfill
//!
//! Memory with hindsight. When a salient *failure* event lands (a bug, crash,
//! regression — the "aversive event"), this reaches **backward in time** and
//! promotes the quiet earlier memory that secretly caused it — the one a pure
//! semantic search will never surface because it isn't *similar* to the failure,
//! only *causally upstream* of it.
//!
//! ## Scientific basis
//!
//! Faithful port of Zaki, Cai et al. (2024), *Nature* 637:145-155, "Offline
//! ensemble co-reactivation links memories across days." Key findings ported:
//!
//! - A **neutral** memory formed earlier is retroactively promoted to important
//!   only when a **salient** event later co-reactivates the two ensembles
//!   offline. (Here: the dream/consolidation pass is the offline window.)
//! - **The asymmetry is backward-only**: "fear links retrospectively, but not
//!   prospectively." A failure promotes the *past* cause, never a future memory.
//!   This is also exactly correct for software: a root cause is always upstream
//!   in time. The biological directionality earns its keep, it is not decorative.
//! - Linking flows along the **overlap ensemble** — memories that share entities
//!   (same file, env var, service, symbol). That shared-entity edge is the join
//!   key the backward scan follows; semantic similarity is deliberately NOT a
//!   ranking signal IN EITHER DIRECTION (that is the whole point — RAG already
//!   covers similarity, and an inverted-similarity bonus would still make the
//!   ranking a function of the embedding space, i.e. vector search with the
//!   sign flipped). Similarity appears in the OUTPUT only, as the reported
//!   similarity_rank proving the cause is not what a vector search surfaces.
//!
//! Honesty note for callers: this is scoped to *failure → backward causal
//! backfill*, not a universal "all salience flows backward" law. The Cai paper
//! is an aversive→neutral paradigm; we mirror that scope intentionally.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// ============================================================================
// CONSTANTS
// ============================================================================

/// A memory must be at least this surprising (prediction error, 0..1) to count
/// as a salient "aversive event" that can trigger a backfill. Mirrors the gate's
/// own surprise scale. Manual triggers bypass this.
pub const DEFAULT_SALIENCE_THRESHOLD: f32 = 0.55;

/// How far back in time the backward reach scans, in days. The Cai paradigm
/// linked across ~2 days; software causes can be older, so we default wider.
pub const DEFAULT_LOOKBACK_DAYS: i64 = 30;

/// A candidate must share at least this many entities with the failure to be
/// considered causally upstream (1 shared file/env-var/service is enough).
pub const MIN_SHARED_ENTITIES: usize = 1;

/// Words that mark a memory as a failure/"aversive" event when auto-detecting.
/// Lowercased substring match against content + tags.
pub const FAILURE_MARKERS: &[&str] = &[
    "error",
    "bug",
    "crash",
    "crashed",
    "regression",
    "broke",
    "broken",
    "failure",
    "failed",
    "panic",
    "exception",
    "fault",
    "outage",
    "incident",
    // NOTE: bare "500" was removed — it matched benign content like "$500",
    // "500 users", or "line 500" and wrongly flagged a quiet CAUSE memory as a
    // failure, excluding it from the backward reach. The specific HTTP error
    // codes 502/503/504 below stay; a genuine "HTTP 500" is still caught by
    // "error"/"failed"/"exception" in any real incident note.
    "timeout",
    "deadlock",
    "leak",
    "corrupt",
    "stack overflow",
    // performance/degradation failures (an agent should backfill from these too)
    "spiked",
    "latency",
    "degraded",
    "slow",
    "hang",
    "hung",
    "throttled",
    "oom",
    "502",
    "503",
    "504",
    "rejected",
    "denied",
    "flaky",
    // real-incident vocabulary (CauseBench found these missing — postmortems often
    // describe failures without the classic crash words above)
    // NOTE: bare "pinned" was removed — dependency pinning ("pinned rails to
    // 5.2") is classic quiet-CAUSE vocabulary; flagging such notes as failures
    // excludes them from the backward reach, the same failure mode that got
    // bare "500" removed above.
    "saturated",
    "saturation",
    "stalled",
    "exhausted",
    "exhaustion",
    "overload",
    "overloaded",
    "backlog",
    "fell behind",
    "lag",
    "lagging",
    "unavailable",
    "down",
    "dropped",
    "reset",
    "refused",
    "stampede",
    "thrashing",
    "starved",
    "starvation",
    "expired",
    "expiry",
    "overflow",
];

/// How strongly to promote the backfilled cause: multiply its stability by this
/// (capped). A real boost so the cause stops decaying and surfaces in future
/// recalls — without overwriting the FSRS history.
pub const PROMOTION_STABILITY_FACTOR: f64 = 2.5;

/// Bonus for candidates that ARE a change record (a commit) rather than a
/// description of one. Real histories are full of reports quoting the same
/// changelog line; when evidence is close, the change outranks the chatter.
/// Small on purpose: shared-entity evidence still dominates.
pub const CHANGE_RECORD_BONUS: f64 = 0.25;

// ============================================================================
// INPUT TYPES
// ============================================================================

/// The minimal view of a memory the backfill needs. Built from a KnowledgeNode
/// by the caller (keeps this module storage-agnostic + trivially testable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackfillCandidate {
    pub id: String,
    pub content: String,
    /// Entities this memory mentions: files, env vars, services, symbols.
    pub entities: Vec<String>,
    /// Age in days relative to the failure event (older = larger). Negative or
    /// zero means it is NOT in the past relative to the failure → excluded.
    pub age_days_before_failure: f64,
    /// Current FSRS stability (we promote by boosting this).
    pub stability: f64,
    /// Set when this candidate stands in for an older record that was
    /// bitemporally superseded: the trail follows the supersession link to the
    /// current belief, dated by the superseded record (the fact's origin).
    #[serde(default)]
    pub via_supersession_of: Option<String>,
    /// True when this candidate is itself a change (a commit record), not a
    /// report about one. Wins ties against reports.
    #[serde(default)]
    pub is_change_record: bool,
}

/// The salient failure event that triggers the backward reach.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureEvent {
    pub id: String,
    pub content: String,
    pub entities: Vec<String>,
    /// The failure's tags — failure markers can live in a tag, so salience
    /// detection must see them too.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Prediction error / surprise of this event (0..1).
    pub prediction_error: f32,
    /// True if a caller explicitly marked this salient (manual override path).
    pub manual: bool,
}

/// Pull shared-entity join keys from content + tags (single source of truth used
/// by the MCP tool, CLI, and offline pass so they never diverge).
/// Does this token look like a real identifier -- an env var, a path, a
/// filename, a dotted symbol -- as opposed to an ordinary English word?
///
/// The backward reach joins a failure to its cause on SHARED ENTITIES. That only
/// works if an "entity" is something specific. Anything that admits common
/// vocabulary turns the causal join into "these two memories used the same
/// word", which is precisely what a vector search already does and what backfill
/// exists to complement. Public so `git_records` emits only tokens that pass
/// this same test (record entities are decided at write time, extracted at
/// query time — both sides must agree).
pub fn is_identifier_shaped(tok: &str) -> bool {
    identifier_tier(tok).is_some()
}

/// How much a shared name of this shape is worth as causal evidence. Tiers,
/// not booleans: a shared file path is strong evidence, a shared English word
/// is almost none (rarity weighting via IDF does the rest).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifierTier {
    /// Dotted/slashed paths and file names: `events/local.py`, `pyvenv.cfg`.
    Path,
    /// Code-shaped names: UPPER_SNAKE env vars, lower_snake identifiers,
    /// camelCase normalized to snake_case: `API_TIMEOUT`, `is_abort_error`.
    Code,
    /// Bare semver-ish versions: `0.8.5`, `1.42`.
    Version,
    /// Bare English words >= 4 chars (`purge`). Cheapest tier: admitted so a
    /// named suspect is never invisible, but rarity weighting plus the 0.3
    /// multiplier keeps vocabulary from out-ranking real anchors.
    Word,
}

impl IdentifierTier {
    pub fn weight(self) -> f64 {
        match self {
            IdentifierTier::Path => 1.0,
            IdentifierTier::Code => 0.9,
            IdentifierTier::Version => 0.6,
            IdentifierTier::Word => 0.3,
        }
    }
}

/// True for `X.Y` / `X.Y.Z` with <=3 digit groups per segment.
fn is_version_token(tok: &str) -> bool {
    let segs: Vec<&str> = tok.split('.').collect();
    segs.len() >= 2
        && segs.len() <= 3
        && segs
            .iter()
            .all(|g| !g.is_empty() && g.len() <= 3 && g.chars().all(|c| c.is_ascii_digit()))
}

/// camelCase / PascalCase with a real case transition, ascii-alphabetic only.
fn is_camel_token(tok: &str) -> bool {
    tok.chars().all(|c| c.is_ascii_alphabetic())
        && tok.chars().any(|c| c.is_ascii_lowercase())
        && tok.chars().any(|c| c.is_ascii_uppercase())
        && tok
            .chars()
            .zip(tok.chars().skip(1))
            .any(|(a, b)| a.is_ascii_lowercase() && b.is_ascii_uppercase())
}

/// camelCase -> snake_case so `isAbortError` joins `is_abort_error`.
fn camel_to_snake(tok: &str) -> String {
    let mut out = String::with_capacity(tok.len() + 4);
    for (i, c) in tok.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

pub fn identifier_tier(tok: &str) -> Option<IdentifierTier> {
    if tok.len() < 3 {
        return None;
    }
    if !tok.contains('/') && is_version_token(tok) {
        return Some(IdentifierTier::Version);
    }
    if tok.contains('/') || tok.contains('.') {
        let segs: Vec<&str> = tok.split(['/', '.']).filter(|x| !x.is_empty()).collect();
        let is_path = segs.len() >= 2
            && segs.iter().any(|x| x.len() >= 3)
            && tok.chars().any(|c| c.is_ascii_alphabetic());
        return if is_path {
            Some(IdentifierTier::Path)
        } else {
            None
        };
    }
    // UPPER_SNAKE env var: underscore REQUIRED so emphasis (VERIFIED, SHIPPED)
    // is not harvested as an env var.
    let is_env = tok.contains('_')
        && tok
            .chars()
            .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
        && tok.chars().any(|c| c.is_ascii_uppercase());
    if is_env {
        return Some(IdentifierTier::Code);
    }
    let is_snake = tok.contains('_')
        && tok
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
        && tok.chars().any(|c| c.is_ascii_lowercase());
    if is_snake {
        return Some(IdentifierTier::Code);
    }
    if is_camel_token(tok) {
        return Some(IdentifierTier::Code);
    }
    if tok.chars().all(|c| c.is_ascii_alphabetic()) && tok.len() >= 4 {
        return Some(IdentifierTier::Word);
    }
    None
}

/// Tier of an already-normalized (lowercased, snake) entity string.
pub fn normalized_tier(entity: &str) -> IdentifierTier {
    if entity.contains('/') {
        return IdentifierTier::Path;
    }
    if is_version_token(entity) {
        return IdentifierTier::Version;
    }
    if entity.contains('.') {
        return IdentifierTier::Path;
    }
    if entity.contains('_') {
        return IdentifierTier::Code;
    }
    IdentifierTier::Word
}

pub fn extract_entities(content: &str, tags: &[String]) -> Vec<String> {
    use std::collections::HashSet;
    // Tags are NOT trusted verbatim. Ingest guidance encourages a topical tag on
    // every save, so tags are overwhelmingly broad vocabulary ("vestige",
    // "bug", "verified"). Inserting them raw made every memory sharing a topic a
    // "causal" match. Run them through the same shape test as content tokens.
    // Shape-test the tag AS WRITTEN, then lowercase for storage. Testing the
    // lowercased form would reject every uppercase env var (is_env requires an
    // ASCII uppercase char), which is the most valuable join key there is.
    let mut set: HashSet<String> = tags
        .iter()
        .map(|t| t.trim())
        .filter(|t| is_identifier_shaped(t))
        .map(|t| {
            if is_camel_token(t) {
                camel_to_snake(t)
            } else {
                t.to_lowercase()
            }
        })
        .collect();
    for raw in content
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.' || c == '/' || c == '-'))
    {
        let tok = raw.trim_matches(|c: char| c == '.' || c == '/' || c == '-');
        if is_identifier_shaped(tok) {
            set.insert(if is_camel_token(tok) {
                camel_to_snake(tok)
            } else {
                tok.to_lowercase()
            });
        }
    }
    set.into_iter().collect()
}

/// Whole-word marker match: `marker` must appear bounded by non-alphanumeric
/// chars, not embedded in a larger identifier. This is the difference between
/// matching "timeout" in "a request timeout" (a real failure) and NOT matching it
/// inside the config var `API_TIMEOUT` (a perfectly ordinary env var). Plain
/// substring over-fires: "timeout" hits "API_TIMEOUT", "leak" hits "leaky", "500"
/// hits "$500" — which wrongly flags a quiet CAUSE as a failure and excludes it
/// from the backward reach.
fn contains_marker_word(hay: &str, marker: &str) -> bool {
    let mut from = 0usize;
    while let Some(pos) = hay[from..].find(marker) {
        let start = from + pos;
        let end = start + marker.len();
        // Inspect the actual char before/after the match, not a raw byte cast to
        // char: for a multibyte UTF-8 boundary the raw byte is a continuation
        // byte (0x80-0xBF), which `as char` misreads as a non-alphanumeric and
        // wrongly passes the word-boundary check. char iteration is boundary-safe.
        let before_ok = hay[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        let after_ok = hay[end..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

/// Does this content/tags pair read like a failure? Whole-word marker match over
/// content + tags (see [`contains_marker_word`] for why whole-word, not substring).
/// Shared by every caller so failure detection never drifts.
pub fn looks_like_failure(content: &str, tags: &[String]) -> bool {
    let hay = content.to_lowercase();
    if FAILURE_MARKERS
        .iter()
        .any(|m| contains_marker_word(&hay, m))
    {
        return true;
    }
    tags.iter().any(|t| {
        let tl = t.to_lowercase();
        FAILURE_MARKERS.iter().any(|m| contains_marker_word(&tl, m))
    })
}

impl FailureEvent {
    /// Auto-detection: is this memory a salient "aversive event"? True when it
    /// is sufficiently surprising AND carries a failure marker — or when a caller
    /// manually flagged it. (The "both" trigger: auto-detect + manual override.)
    pub fn is_salient(&self, salience_threshold: f32) -> bool {
        if self.manual {
            return true;
        }
        if self.prediction_error < salience_threshold {
            return false;
        }
        looks_like_failure(&self.content, &self.tags)
    }
}

// ============================================================================
// OUTPUT TYPES
// ============================================================================

/// One promoted memory: a quiet earlier cause the failure reached back to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackfilledCause {
    pub memory_id: String,
    /// The entities it shares with the failure (the causal join).
    pub shared_entities: Vec<String>,
    /// Days before the failure this memory was formed.
    pub age_days: f64,
    /// Backfill score (higher = stronger candidate cause).
    pub score: f64,
    /// New stability after promotion (= old * factor, capped).
    pub promoted_stability: f64,
    /// Whether the surfaced cause is itself a change record (a commit).
    #[serde(default)]
    pub is_change_record: bool,
    /// Human-readable why.
    pub reason: String,
}

/// One high-ranked record that was rejected, with the rule that excluded it
/// ("why not X?"). Ranked by how many failure entities it shares, so the
/// reported rejections are the ones a skeptic would name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RejectedCandidate {
    pub memory_id: String,
    pub reason: String,
    /// Shared-entity count at rejection time (0 for no-entity rejections).
    #[serde(default)]
    pub shared_entities: usize,
    /// Age in days vs the failure (negative = newer); 0 for caller exclusions.
    #[serde(default)]
    pub age_days: f64,
}

/// Reported when the trail stops short: which entities nothing in the window
/// shares, and what record would close the chain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackfillGap {
    /// Failure entities no in-window candidate carries (empty when matches
    /// existed but were all excluded — see [`BackfillResult::rejected`]).
    pub missing_entities: Vec<String>,
    pub note: String,
}

/// A candidate the caller excluded before the reach (e.g. a commit outside the
/// mapped version range), carried into why-not output.
#[derive(Debug, Clone)]
pub struct ExcludedCandidate {
    pub candidate: BackfillCandidate,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackfillResult {
    pub triggered: bool,
    pub failure_id: String,
    pub causes: Vec<BackfilledCause>,
    /// Strongest rejected records, for "why not X?" questions.
    #[serde(default)]
    pub rejected: Vec<RejectedCandidate>,
    /// Set when the trail broke and nothing was surfaced.
    #[serde(default)]
    pub gap: Option<BackfillGap>,
    pub scanned: usize,
}

// ============================================================================
// THE BACKFILL
// ============================================================================

#[derive(Debug, Clone)]
pub struct RetroactiveBackfill {
    pub salience_threshold: f32,
    pub lookback_days: i64,
    pub min_shared_entities: usize,
    pub max_causes: usize,
    /// How many rejected records to report for why-not questions.
    pub max_rejections: usize,
}

impl Default for RetroactiveBackfill {
    fn default() -> Self {
        Self {
            salience_threshold: DEFAULT_SALIENCE_THRESHOLD,
            lookback_days: DEFAULT_LOOKBACK_DAYS,
            min_shared_entities: MIN_SHARED_ENTITIES,
            max_causes: 3,
            max_rejections: 3,
        }
    }
}

impl RetroactiveBackfill {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run the backward reach. Given a (possibly salient) failure and the pool of
    /// earlier candidate memories, return which past memories to promote and why.
    ///
    /// Backward-only by construction: candidates with `age_days_before_failure`
    /// <= 0 (i.e. concurrent or future) are never considered.
    pub fn run(&self, failure: &FailureEvent, candidates: &[BackfillCandidate]) -> BackfillResult {
        self.run_trail(failure, candidates, &[])
    }

    /// Like [`run`], plus: reports the strongest REJECTED records with the rule
    /// that excluded each (why-not-X), and — when nothing surfaces — where the
    /// trail broke and what record would close it (gap report). `excluded`
    /// carries caller-side exclusions (e.g. a commit outside the mapped version
    /// range) so they appear in the same why-not output.
    pub fn run_trail(
        &self,
        failure: &FailureEvent,
        candidates: &[BackfillCandidate],
        excluded: &[ExcludedCandidate],
    ) -> BackfillResult {
        if !failure.is_salient(self.salience_threshold) {
            return BackfillResult {
                triggered: false,
                failure_id: failure.id.clone(),
                causes: vec![],
                rejected: vec![],
                gap: None,
                scanned: 0,
            };
        }

        let failure_entities: HashSet<&str> = failure.entities.iter().map(|s| s.as_str()).collect();

        // Inverse document frequency over the IN-WINDOW pool: an entity that
        // nearly every record carries (issue-template paths, the product's own
        // package name) is boilerplate, not a clue. Replaces the raw shared
        // count, which let two boilerplate matches outrank one real one.
        // In-window only: post-failure records are excluded from the reach, so
        // they must not inflate df either — a hot file churned on both sides of
        // the failure would otherwise deflate the very cause being ranked.
        let in_window: Vec<&BackfillCandidate> = candidates
            .iter()
            .filter(|c| {
                c.age_days_before_failure > 0.0
                    && c.age_days_before_failure <= self.lookback_days as f64
            })
            .collect();
        let n = in_window.len();
        let mut df: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for c in &in_window {
            for e in &c.entities {
                *df.entry(e.as_str()).or_default() += 1;
            }
        }
        // probabilistic IDF * shape tier: a name every in-window record
        // carries weighs ~0, a rare one weighs ln((1+n)/2); a shared file path
        // is worth ~3x a shared English word before rarity is even considered.
        let idf = |e: &str| -> f64 {
            let d = df.get(e).copied().unwrap_or(0) as f64;
            ((1.0 + n as f64) / (1.0 + d)).ln().max(0.0) * normalized_tier(e).weight()
        };

        let mut rejected: Vec<RejectedCandidate> = Vec::new();
        let note_supersession = |c: &BackfillCandidate, reason: String| -> String {
            match &c.via_supersession_of {
                Some(orig) => format!("{reason} Trail followed the supersession of {orig}."),
                None => reason,
            }
        };

        let mut scored: Vec<BackfilledCause> = candidates
            .iter()
            .filter_map(|c| {
                let shared: Vec<String> = c
                    .entities
                    .iter()
                    .filter(|e| failure_entities.contains(e.as_str()))
                    .cloned()
                    .collect();
                let shared_n = shared.len();
                // rejection bookkeeping: keep the rule that excluded the record
                if c.age_days_before_failure <= 0.0 {
                    rejected.push(RejectedCandidate {
                        memory_id: c.id.clone(),
                        reason: "record is newer than the failure".into(),
                        shared_entities: shared_n,
                        age_days: c.age_days_before_failure,
                    });
                    return None;
                }
                if c.age_days_before_failure > self.lookback_days as f64 {
                    rejected.push(RejectedCandidate {
                        memory_id: c.id.clone(),
                        reason: format!("outside the {}d lookback window", self.lookback_days),
                        shared_entities: shared_n,
                        age_days: c.age_days_before_failure,
                    });
                    return None;
                }
                if shared_n < self.min_shared_entities {
                    rejected.push(RejectedCandidate {
                        memory_id: c.id.clone(),
                        reason: "shares no entity with the failure".into(),
                        shared_entities: shared_n,
                        age_days: c.age_days_before_failure,
                    });
                    return None;
                }
                let score = self.score(c, &shared, &idf);
                let promoted = (c.stability * PROMOTION_STABILITY_FACTOR).min(c.stability + 365.0);
                let reason = note_supersession(
                    c,
                    format!(
                        "Reached back {:.1}d to a quiet memory sharing {} entit{} ({}) with the failure; \
                         Shared entities support an association, not proof of cause.",
                        c.age_days_before_failure,
                        shared_n,
                        if shared_n == 1 { "y" } else { "ies" },
                        shared.join(", "),
                    ),
                );
                Some(BackfilledCause {
                    memory_id: c.id.clone(),
                    shared_entities: shared,
                    age_days: c.age_days_before_failure,
                    score,
                    promoted_stability: promoted,
                    is_change_record: c.is_change_record,
                    reason,
                })
            })
            .collect();

        for ex in excluded {
            let shared_n = ex
                .candidate
                .entities
                .iter()
                .filter(|e| failure_entities.contains(e.as_str()))
                .count();
            rejected.push(RejectedCandidate {
                memory_id: ex.candidate.id.clone(),
                reason: ex.reason.clone(),
                shared_entities: shared_n,
                age_days: 0.0,
            });
        }

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                // near-ties: the change, not the newest report about it
                .then(b.is_change_record.cmp(&a.is_change_record))
                .then(
                    a.age_days
                        .partial_cmp(&b.age_days)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
        });
        scored.truncate(self.max_causes);

        // strongest rejections first: most shared entities, then most recent
        rejected.sort_by(|a, b| {
            b.shared_entities.cmp(&a.shared_entities).then(
                a.age_days
                    .partial_cmp(&b.age_days)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });
        rejected.truncate(self.max_rejections);

        // gap report: the trail broke — name the missing link
        let gap = if scored.is_empty() {
            let matched_but_excluded = rejected
                .iter()
                .filter(|r| r.shared_entities >= self.min_shared_entities)
                .count();
            if matched_but_excluded > 0 {
                Some(BackfillGap {
                    missing_entities: vec![],
                    note: format!(
                        "{matched_but_excluded} record(s) share entities with the failure but were all excluded — see rejected. Widen the window/range or ingest the missing link records."
                    ),
                })
            } else {
                let window_entities: HashSet<&str> = candidates
                    .iter()
                    .filter(|c| {
                        c.age_days_before_failure > 0.0
                            && c.age_days_before_failure <= self.lookback_days as f64
                    })
                    .flat_map(|c| c.entities.iter().map(|s| s.as_str()))
                    .collect();
                let missing: Vec<String> = failure
                    .entities
                    .iter()
                    .filter(|e| !window_entities.contains(e.as_str()))
                    .cloned()
                    .collect();
                let shown = missing
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(BackfillGap {
                    missing_entities: missing,
                    note: format!(
                        "No record within {}d shares any of the failure's entities ({}). A commit or note touching one of those inside the window would close this trail.",
                        self.lookback_days,
                        if shown.is_empty() {
                            "(the failure names no entities)"
                        } else {
                            &shown
                        }
                    ),
                })
            }
        } else {
            None
        };

        BackfillResult {
            triggered: true,
            failure_id: failure.id.clone(),
            causes: scored,
            rejected,
            gap,
            scanned: candidates.len(),
        }
    }

    /// Score a candidate cause. Purely STRUCTURAL: IDF-weighted shared entities
    /// (boilerplate names count for almost nothing, rare identifiers a lot), a
    /// weak recency term, and the change-record bonus. Embedding similarity is
    /// NOT an input — not to pull lookalikes in, and not to push them away: a
    /// dissimilarity bonus would make this vector search with the sign flipped.
    fn score(&self, c: &BackfillCandidate, shared: &[String], idf: &impl Fn(&str) -> f64) -> f64 {
        let entity_term: f64 = shared.iter().map(|e| idf(e)).sum();
        // gentle recency-in-the-past: 1.0 at the failure, fading with age
        let recency_term =
            0.3 * (1.0 / (1.0 + c.age_days_before_failure / self.lookback_days as f64));
        let change_term = if c.is_change_record {
            CHANGE_RECORD_BONUS
        } else {
            0.0
        };
        entity_term + recency_term + change_term
    }
}

// ============================================================================
// TESTS — the receipt: plant a cause, inject a failure, assert backfill finds
// the cause that a similarity search ranks near the bottom.
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn failure() -> FailureEvent {
        FailureEvent {
            id: "fail-wed".into(),
            content: "Service crashed: 500 Internal Server Error on the auth endpoint".into(),
            entities: vec!["auth-service".into(), "API_TIMEOUT".into()],
            tags: vec![],
            prediction_error: 0.9,
            manual: false,
        }
    }

    /// The headline scenario: a quiet env-var change days ago caused a crash now.
    /// Semantic search ranks it LAST (it's not similar to "crash"); backfill
    /// promotes it because it shares the API_TIMEOUT entity, backward in time.
    #[test]
    fn backfill_surfaces_the_cause_rag_misses() {
        let candidates = vec![
            // the actual cause: a quiet config note from 3 days ago. Low similarity.
            BackfillCandidate {
                id: "cause-mon".into(),
                content: "Set API_TIMEOUT=2 in the deploy env to speed up cold starts".into(),
                entities: vec!["API_TIMEOUT".into(), "deploy-env".into()],
                age_days_before_failure: 3.0,
                stability: 5.0,
                via_supersession_of: None,
                is_change_record: false,
            },
            // a noisy distractor: semantically similar to the crash, but NOT causal
            // (shares no entity with the failure).
            BackfillCandidate {
                id: "noise-similar".into(),
                content: "Another 500 error happened in the billing service last month".into(),
                entities: vec!["billing-service".into()],
                age_days_before_failure: 20.0,
                stability: 3.0,
                via_supersession_of: None,
                is_change_record: false,
            },
            // a future memory — must never be backfilled (backward-only).
            BackfillCandidate {
                id: "future".into(),
                content: "Plan to add API_TIMEOUT retries next sprint".into(),
                entities: vec!["API_TIMEOUT".into()],
                age_days_before_failure: -1.0,
                stability: 2.0,
                via_supersession_of: None,
                is_change_record: false,
            },
        ];

        let result = RetroactiveBackfill::new().run(&failure(), &candidates);

        assert!(
            result.triggered,
            "high-PE failure with markers must trigger"
        );
        assert!(!result.causes.is_empty(), "must surface at least one cause");

        let top = &result.causes[0];
        // the promoted memory is the real cause, not the similar distractor
        assert_eq!(
            top.memory_id, "cause-mon",
            "must promote the causal env-var note"
        );
        assert!(top.shared_entities.contains(&"API_TIMEOUT".to_string()));
        // the similar distractor must not be surfaced at all: it shares no anchor
        assert!(
            !result.causes.iter().any(|c| c.memory_id == "noise-similar"),
            "a lookalike with no shared anchor must never be a cause"
        );
        // backward-only: the future memory is never promoted
        assert!(
            !result.causes.iter().any(|c| c.memory_id == "future"),
            "backward-only: a future memory must never be backfilled"
        );
        // it gets a real stability boost (stops decaying, will surface next time)
        assert!(
            top.promoted_stability > 5.0,
            "the cause must be promoted (boosted stability)"
        );
    }

    /// extract_entities had NO test exercising it on realistic prose -- every
    /// existing test hand-supplies a clean `entities` vec -- which is exactly how
    /// a vocabulary-harvesting extractor shipped. The causal join is only
    /// meaningful if an "entity" is specific.
    #[test]
    fn extract_entities_keeps_identifiers_and_rejects_prose() {
        let content = "VESTIGE BUG VERIFIED: this is a DANGEROUS FALSE NEGATIVE. \
                       Set RUST_LOG and VESTIGE_DATA_DIR before running, e.g. the \
                       store at com.vestige.core/backups, see sqlite.rs for detail.";
        let tags = vec![
            "vestige".to_string(),
            "bug".to_string(),
            "verified".to_string(),
            // TAG-ONLY identifier: deliberately absent from `content` so this
            // exercises the tag path in isolation. Shape-testing the LOWERCASED
            // tag silently drops every uppercase env var (is_env needs an ASCII
            // uppercase char) -- a regression the content path would otherwise
            // mask.
            "PAYMENTS_REDIS_URL".to_string(),
        ];
        let ents = extract_entities(content, &tags);

        // Real identifiers survive.
        for want in [
            "rust_log",
            "vestige_data_dir",
            "payments_redis_url", // tag-only: proves tags are not shape-tested post-lowercase
            "com.vestige.core/backups",
            "sqlite.rs",
        ] {
            assert!(
                ents.iter().any(|e| e == want),
                "missing entity {want:?}: {ents:?}"
            );
        }
        // Bare words >= 4 chars ARE now join keys (Word tier, 0.3x weight +
        // rarity-weighted): a named suspect like `purge` must never be
        // invisible. The noise guard moved from extraction into scoring —
        // two word-tier matches cannot out-rank one code/path anchor
        // (pinned by boilerplate_matches_lose_to_the_rare_one).
        for word in ["verified", "dangerous", "negative", "vestige"] {
            assert!(
                ents.iter().any(|e| e == word),
                "bare word {word:?} must extract at Word tier: {ents:?}"
            );
            assert_eq!(normalized_tier(word), IdentifierTier::Word);
        }
        // short words and prose abbreviations still never extract
        for junk in ["bug", "e.g"] {
            assert!(
                !ents.iter().any(|e| e == junk),
                "{junk:?} must not be an entity: {ents:?}"
            );
        }
        // code shapes land at the stronger tiers
        assert_eq!(normalized_tier("pyvenv.cfg"), IdentifierTier::Path);
        assert_eq!(normalized_tier("is_abort_error"), IdentifierTier::Code);
        assert_eq!(normalized_tier("api_timeout"), IdentifierTier::Code);
        assert_eq!(normalized_tier("0.8.5"), IdentifierTier::Version);
    }

    #[test]
    fn non_salient_event_does_not_trigger() {
        let calm = FailureEvent {
            id: "calm".into(),
            content: "Refactored the logging format for readability".into(),
            entities: vec!["logger".into()],
            tags: vec![],
            prediction_error: 0.2, // low surprise
            manual: false,
        };
        let result = RetroactiveBackfill::new().run(&calm, &[]);
        assert!(
            !result.triggered,
            "a calm, low-surprise note must not fire a backfill"
        );
    }

    #[test]
    fn manual_override_triggers_without_markers() {
        // No failure word, low PE — but the caller explicitly marked it salient.
        let manual = FailureEvent {
            id: "manual".into(),
            content: "Latency crept up on the checkout path".into(),
            entities: vec!["checkout".into()],
            tags: vec![],
            prediction_error: 0.1,
            manual: true,
        };
        let candidates = vec![BackfillCandidate {
            id: "cause".into(),
            content: "Disabled the checkout cache while debugging".into(),
            entities: vec!["checkout".into()],
            age_days_before_failure: 2.0,
            stability: 4.0,
            via_supersession_of: None,
            is_change_record: false,
        }];
        let result = RetroactiveBackfill::new().run(&manual, &candidates);
        assert!(
            result.triggered,
            "manual override must trigger regardless of markers/PE"
        );
        assert_eq!(result.causes[0].memory_id, "cause");
    }

    #[test]
    fn requires_a_shared_entity_no_spurious_links() {
        // A salient failure but the only past memory shares NO entity — we must
        // NOT invent a causal link (avoids the A-B,B-C spurious-edge failure mode).
        let candidates = vec![BackfillCandidate {
            id: "unrelated".into(),
            content: "Updated the README badges".into(),
            entities: vec!["README".into()],
            age_days_before_failure: 1.0,
            stability: 4.0,
            via_supersession_of: None,
            is_change_record: false,
        }];
        let result = RetroactiveBackfill::new().run(&failure(), &candidates);
        assert!(result.triggered);
        assert!(
            result.causes.is_empty(),
            "no shared entity => no backfill (don't fabricate a cause)"
        );
    }

    // ---- why-not-X and the gap report ----

    fn failure_with_entities() -> FailureEvent {
        FailureEvent {
            id: "fail".into(),
            content: "crash in events/local.py after upgrading".into(),
            entities: vec!["events/local.py".into(), "API_TIMEOUT".into()],
            tags: vec![],
            prediction_error: 0.9,
            manual: false,
        }
    }

    #[test]
    fn rejections_carry_the_rule_that_excluded_each() {
        let candidates = vec![
            // newer than the failure — excluded despite sharing an entity
            BackfillCandidate {
                id: "later-fix".into(),
                content: "commit f1 fixed local.py".into(),
                entities: vec!["events/local.py".into()],
                age_days_before_failure: -2.0,
                stability: 4.0,
                via_supersession_of: None,
                is_change_record: false,
            },
            // too old for the window
            BackfillCandidate {
                id: "ancient".into(),
                content: "touched local.py long ago".into(),
                entities: vec!["events/local.py".into()],
                age_days_before_failure: 90.0,
                stability: 4.0,
                via_supersession_of: None,
                is_change_record: false,
            },
            // in-window but shares nothing
            BackfillCandidate {
                id: "unrelated".into(),
                content: "readme badge churn".into(),
                entities: vec!["README".into()],
                age_days_before_failure: 1.0,
                stability: 4.0,
                via_supersession_of: None,
                is_change_record: false,
            },
        ];
        let result =
            RetroactiveBackfill::new().run_trail(&failure_with_entities(), &candidates, &[]);
        assert!(result.causes.is_empty());
        let reasons: Vec<(&str, &str)> = result
            .rejected
            .iter()
            .map(|r| (r.memory_id.as_str(), r.reason.as_str()))
            .collect();
        assert!(reasons.contains(&("later-fix", "record is newer than the failure")));
        assert!(
            reasons
                .iter()
                .any(|(id, r)| *id == "ancient" && r.contains("lookback"))
        );
        assert!(reasons.contains(&("unrelated", "shares no entity with the failure")));
        // the skeptic's first question gets the direct answer: the entity-sharing
        // record outranks the truly unrelated one in the why-not list
        assert_eq!(result.rejected[0].memory_id, "later-fix");
    }

    #[test]
    fn gap_report_names_the_missing_link() {
        // in-window candidates exist, but none carries a failure entity
        let candidates = vec![BackfillCandidate {
            id: "other".into(),
            content: "unrelated deploy note".into(),
            entities: vec!["deploy-env".into()],
            age_days_before_failure: 2.0,
            stability: 4.0,
            via_supersession_of: None,
            is_change_record: false,
        }];
        let result =
            RetroactiveBackfill::new().run_trail(&failure_with_entities(), &candidates, &[]);
        let gap = result.gap.expect("empty causes must produce a gap report");
        assert!(
            gap.missing_entities
                .contains(&"events/local.py".to_string())
        );
        assert!(
            gap.note.contains("local.py"),
            "note names the entity: {}",
            gap.note
        );
        assert!(gap.note.contains("would close this trail"));
    }

    #[test]
    fn boilerplate_matches_lose_to_the_rare_one() {
        // Issue-template paths land in every report; under raw shared counts,
        // two template matches beat the one real identifier. They must not.
        let failure = FailureEvent {
            id: "fail".into(),
            content: "crash after upgrade: T1 T3 RARE_SETTING".into(),
            entities: vec!["T1".into(), "T3".into(), "RARE_SETTING".into()],
            tags: vec![],
            prediction_error: 0.9,
            manual: false,
        };
        let mut candidates = Vec::new();
        // five template-carrying records; the distractor shares TWO of them
        for i in 0..5 {
            candidates.push(BackfillCandidate {
                id: format!("filler-{i}"),
                content: "template report".into(),
                entities: vec!["T1".into(), "T2".into(), "T3".into()],
                age_days_before_failure: 1.0,
                stability: 4.0,
                via_supersession_of: None,
                is_change_record: false,
            });
            if i == 0 {
                candidates[0].id = "distractor".into();
            }
        }
        // the real cause shares only the rare identifier
        candidates.push(BackfillCandidate {
            id: "cause".into(),
            content: "commit flipping RARE_SETTING default".into(),
            entities: vec!["RARE_SETTING".into()],
            age_days_before_failure: 1.0,
            stability: 4.0,
            via_supersession_of: None,
            is_change_record: true,
        });

        let result = RetroactiveBackfill::new().run(&failure, &candidates);
        assert_eq!(
            result.causes[0].memory_id, "cause",
            "the rare shared identifier must outrank two boilerplate matches"
        );
        assert!(result.causes[0].is_change_record);
    }

    #[test]
    fn ties_go_to_the_change_not_the_newest_report() {
        let failure = FailureEvent {
            id: "fail".into(),
            content: "crash in local.py".into(),
            entities: vec!["local.py".into()],
            tags: vec![],
            prediction_error: 0.9,
            manual: false,
        };
        let make = |id: &str, change: bool, age: f64| BackfillCandidate {
            id: id.into(),
            content: "touched local.py".into(),
            entities: vec!["local.py".into()],
            age_days_before_failure: age,
            stability: 4.0,
            via_supersession_of: None,
            is_change_record: change,
        };
        // equal evidence, equal age: the change record wins
        let result = RetroactiveBackfill::new().run(
            &failure,
            &[make("report", false, 5.0), make("change", true, 5.0)],
        );
        assert_eq!(result.causes[0].memory_id, "change");

        // the change may be considerably older and still win the near-tie
        let result = RetroactiveBackfill::new().run(
            &failure,
            &[make("report", false, 1.0), make("change", true, 8.0)],
        );
        assert_eq!(result.causes[0].memory_id, "change");
    }

    #[test]
    fn idf_ignores_post_failure_records() {
        // Post-failure records are excluded from the reach, so they must not
        // inflate an entity's document frequency either: a hot file churned on
        // BOTH sides of the failure would deflate the in-window cause's rare
        // clue. Under the pre-fix df (whole pool), the boilerplate pair won.
        let failure = FailureEvent {
            id: "fail".into(),
            content: "crash T1 T2 RARE_X".into(),
            entities: vec!["T1".into(), "T2".into(), "RARE_X".into()],
            tags: vec![],
            prediction_error: 0.9,
            manual: false,
        };
        let mut candidates = Vec::new();
        // five in-window template records carrying T1/T2 (the boilerplate pair
        // IS common in-window — that's why it must lose)
        for i in 0..5 {
            candidates.push(BackfillCandidate {
                id: format!("template-{i}"),
                content: "template noise".into(),
                entities: vec!["T1".into(), "T2".into()],
                age_days_before_failure: 1.0,
                stability: 4.0,
                via_supersession_of: None,
                is_change_record: false,
            });
        }
        // in-window cause: the rare identifier, itself a change record
        candidates.push(BackfillCandidate {
            id: "cause".into(),
            content: "commit flipping RARE_X".into(),
            entities: vec!["RARE_X".into()],
            age_days_before_failure: 1.0,
            stability: 4.0,
            via_supersession_of: None,
            is_change_record: true,
        });
        // eight POST-failure records all carrying RARE_X: under whole-pool df,
        // df(RARE_X)=9 made the rare clue look common and the template pair won
        for i in 0..8 {
            candidates.push(BackfillCandidate {
                id: format!("future-{i}"),
                content: "post-failure churn".into(),
                entities: vec!["RARE_X".into()],
                age_days_before_failure: -(i as f64 + 1.0),
                stability: 4.0,
                via_supersession_of: None,
                is_change_record: false,
            });
        }
        let result = RetroactiveBackfill::new().run(&failure, &candidates);
        assert_eq!(
            result.causes[0].memory_id, "cause",
            "future churn must not deflate the in-window rare clue"
        );
    }

    #[test]
    fn ranking_is_purely_structural_no_similarity_anywhere() {
        // The similarity field is DELETED from the candidate type: the engine
        // physically cannot consult embeddings, in any direction.
        let failure = FailureEvent {
            id: "fail".into(),
            content: "crash in local.py".into(),
            entities: vec!["local.py".into()],
            tags: vec![],
            prediction_error: 0.9,
            manual: false,
        };
        let make = |id: &str| BackfillCandidate {
            id: id.into(),
            content: "touched local.py".into(),
            entities: vec!["local.py".into()],
            age_days_before_failure: 5.0,
            stability: 4.0,
            via_supersession_of: None,
            is_change_record: false,
        };
        let pool = vec![make("a"), make("b"), make("c")];
        let r = RetroactiveBackfill::new().run(&failure, &pool);
        let ids: Vec<&str> = r.causes.iter().map(|c| c.memory_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
        // a candidate with the rarer anchor set outranks equal-count boilerplate
        let pool_c = vec![
            make("lookalike"),
            BackfillCandidate {
                id: "cause".into(),
                content: "commit touching local.py".into(),
                entities: vec!["local.py".into(), "RARE_KEY".into()],
                age_days_before_failure: 5.0,
                stability: 4.0,
                via_supersession_of: None,
                is_change_record: true,
            },
        ];
        let rc = RetroactiveBackfill::new().run(&failure, &pool_c);
        assert_eq!(rc.causes[0].memory_id, "cause");
    }

    #[test]
    fn dependency_pinning_is_not_a_failure_marker() {
        // "pinned" was removed from FAILURE_MARKERS: pinning a dependency is
        // quiet-CAUSE vocabulary and must stay reachable by the backward reach.
        assert!(!looks_like_failure(
            "pinned rails to 5.2 for the tz workaround",
            &[]
        ));
        // real incident vocabulary still detected
        assert!(looks_like_failure("connection pool saturated at 100%", &[]));
    }

    #[test]
    fn caller_exclusions_surface_in_why_not() {
        let candidate = BackfillCandidate {
            id: "commit-x".into(),
            content: "commit abc touched local.py".into(),
            entities: vec!["events/local.py".into()],
            age_days_before_failure: 2.0,
            stability: 4.0,
            via_supersession_of: None,
            is_change_record: false,
        };
        let excluded = vec![ExcludedCandidate {
            candidate,
            reason: "outside version range 1.41.0..1.42.1".into(),
        }];
        let result = RetroactiveBackfill::new().run_trail(&failure_with_entities(), &[], &excluded);
        assert!(result.causes.is_empty());
        assert_eq!(result.rejected.len(), 1);
        assert_eq!(result.rejected[0].memory_id, "commit-x");
        assert_eq!(
            result.rejected[0].reason,
            "outside version range 1.41.0..1.42.1"
        );
        // a match existed but was excluded: the gap says so instead of "missing"
        let gap = result
            .gap
            .expect("excluded match must still report the break");
        assert!(gap.missing_entities.is_empty());
        assert!(gap.note.contains("excluded"), "{}", gap.note);
    }
}
