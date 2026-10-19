//! Dream Compile — report and configuration structures for Strata edge replay.

use serde::Serialize;

/// Replay strengthens a co-hit edge by this much (capped at 1.0 by the
/// storage layer). Small by design: a single dream is one vote, not a verdict.
pub const REPLAY_STRENGTHEN_BOOST: f64 = 0.1;

/// An edge weaker than this is "weak" for synaptic downscaling. Strong edges
/// are protected even when unreplayed — downscaling targets unused synapses,
/// not established ones.
pub const WEAK_EDGE_FLOOR: f64 = 0.5;

/// Configuration for dream compile.
#[derive(Debug, Clone)]
pub struct DreamCompileConfig {
    /// Namespace to dream over.
    pub scope: String,
    /// How many top-retention memories enter the dream (5..=500).
    pub memory_count: usize,
    /// Review-PR budget per run. 0 runs the phases but files nothing.
    pub max_prs: usize,
}

impl Default for DreamCompileConfig {
    fn default() -> Self {
        Self {
            scope: "user".to_string(),
            memory_count: 50,
            max_prs: 20,
        }
    }
}

/// One phase of the executed dream, for the report.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamCompilePhase {
    pub phase: String,
    pub duration_ms: u64,
    pub memories_processed: usize,
    pub actions: Vec<String>,
}

/// A Memory PR filed by the run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamCompilePr {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub source_ids: Vec<String>,
}

/// What one dream compile did. Serialized straight onto the MCP wire.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamCompileReport {
    /// `"compiled"` or `"insufficient_memories"`.
    pub status: String,
    pub scope: String,
    pub memories_selected: usize,
    pub memories_replayed: usize,
    pub phases: Vec<DreamCompilePhase>,
    pub edges_strengthened: usize,
    pub edges_downscaled: usize,
    pub contradictions_found: usize,
    pub insights_generated: usize,
    pub prs_filed: Vec<DreamCompilePr>,
    pub dream_history_recorded: bool,
    pub duration_ms: u64,
}
