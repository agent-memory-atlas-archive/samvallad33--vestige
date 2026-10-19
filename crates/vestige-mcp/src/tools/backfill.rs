//! # Retroactive Salience Backfill — MCP tool
//!
//! On Strata, backward causal reach follows recorded edges via `causal_walk`;
//! heuristic entity-overlap backfill returns `pending_strata`.

use serde_json::{Value, json};
use std::sync::Arc;

use vestige_core::advanced::retroactive_backfill;
use vestige_core::{KnowledgeNode, Storage};

pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "scope": {"type": "string", "default": "user", "description": "Exact project namespace for failure and candidates. Defaults to user; cross-project inference is not performed."},
            "failure_id": {
                "type": "string",
                "description": "Failure memory to backfill from. Omitted: the most recent failure-like memory."
            },
            "manual": {
                "type": "boolean",
                "description": "Force the backfill when the event is not auto-detected as salient. Default false.",
                "default": false
            },
            "lookback_days": {
                "type": "integer",
                "description": "How many days back to reach for the cause. Default 30.",
                "minimum": 1,
                "maximum": 365,
                "default": 30
            },
            "promote": {
                "type": "boolean",
                "description": "Explicitly reinforce candidates and record candidate edges after review. Default false: preview only, with no graph or strength mutation. Promotion does not verify causality.",
                "default": false
            },
            "scan_limit": {
                "type": "integer",
                "description": "Max memories to scan as candidate causes. Default 500.",
                "minimum": 10,
                "maximum": 5000,
                "default": 500
            },
            "why_not": {
                "type": "string",
                "description": "Memory id (or commit sha prefix): report the exact rule that excluded this record, or its rank among the surfaced causes."
            },
            "git_repo": {
                "type": "string",
                "description": "Path to the git repository backing this scope's commit records. Enables version-range mapping: 'broke in X, worked in Y' in the failure text (or explicit worked_in/broke_in tags) restricts commit candidates to that range."
            },
            "worked_in": {
                "type": "string",
                "description": "Last-known-good tag for the version range. Requires git_repo."
            },
            "broke_in": {
                "type": "string",
                "description": "First-bad tag for the version range. Requires git_repo."
            }
        }
    })
}

/// Thin `&KnowledgeNode` adapter over [`retroactive_backfill::looks_like_failure`].
pub fn looks_like_failure(node: &KnowledgeNode) -> bool {
    retroactive_backfill::looks_like_failure(&node.content, &node.tags)
}

pub async fn execute(_storage: &Arc<Storage>, _args: Option<Value>) -> Result<Value, String> {
    Err("pending_strata: backfill entity overlap is not a recorded edge".into())
}
