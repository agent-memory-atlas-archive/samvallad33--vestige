//! composed_graph tool — durable composition history and bounty-mode lane queue.
//!
//! On Strata, `composed_graph` delegates directly to `ghostlink`.

use serde_json::Value;
use std::sync::Arc;
use vestige_core::Storage;

/// Canonical outcome vocabulary (backend-neutral, in vestige-core).
pub(crate) const OUTCOME_TYPES: &[&str] = vestige_core::composition::OUTCOME_TYPES;

pub fn schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["recent", "get", "memory", "neighbors", "never_composed", "bounty_mode", "label"],
                "description": "ComposedGraph action to run."
            },
            "event_id": {
                "type": "string",
                "description": "Composition event id for get/label actions."
            },
            "memory_id": {
                "type": "string",
                "description": "Memory id for memory/neighbors actions."
            },
            "limit": {
                "type": "integer",
                "description": "Maximum rows to return (default 10, max 100).",
                "default": 10,
                "minimum": 1,
                "maximum": 100
            },
            "tags": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional tag filter for never_composed and bounty_mode."
            },
            "outcome_type": {
                "type": "string",
                "enum": ["helpful", "dead_end", "submitted", "accepted", "rejected", "duplicate_risk", "needs_poc", "bad_severity", "user_promoted", "user_demoted", "closed_by_scope", "closed_by_duplicate", "closed_by_false_assumption", "closed_by_user", "expired_lane"],
                "description": "Outcome label for label action."
            },
            "notes": {
                "type": "string",
                "description": "Optional outcome notes."
            },
            "label_source": {
                "type": "string",
                "description": "Where the outcome label came from (default: user)."
            },
            "confidence_delta": {
                "type": "number",
                "description": "Optional confidence adjustment for this outcome."
            }
        },
        "required": ["action"]
    })
}

pub async fn execute(storage: &Arc<Storage>, args: Option<Value>) -> Result<Value, String> {
    let action = args
        .as_ref()
        .and_then(|a| a.get("action"))
        .and_then(|v| v.as_str())
        .ok_or("Missing arguments")?
        .to_string();
    if !matches!(
        action.as_str(),
        "recent" | "get" | "memory" | "neighbors" | "never_composed" | "bounty_mode" | "label"
    ) {
        return Err(format!("Unknown composed_graph action: {action}"));
    }
    if action != "never_composed"
        && args
            .as_ref()
            .is_some_and(|a| a.get("scope").is_some() || a.get("includeCrossScope").is_some())
    {
        return Err("scope and includeCrossScope currently apply only to never_composed".into());
    }
    super::ghostlink::execute_graph_action_on_strata(storage, &action, args).await
}
