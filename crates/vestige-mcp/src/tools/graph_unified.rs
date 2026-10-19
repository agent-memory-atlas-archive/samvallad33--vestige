//! Unified `graph` Tool (v2.2 — Tool Consolidation)
//!
//! Folds four graph/association/prediction tools into one action-dispatched
//! surface:
//!
//!   action ∈ {
//!     chain, associations, bridges,          // former explore_connections
//!     predict,                               // former predict
//!     memory_graph,                          // former memory_graph (viz subgraph)
//!     recent, get, memory, neighbors,        // former composed_graph
//!     never_composed, bounty_mode, label,    //   "
//!   }
//!
//! This is a transparent facade: each action forwards the *same* args envelope
//! to the existing handler, which re-reads its own discriminator/params. None of
//! the underlying arg structs use `deny_unknown_fields`, so unrelated fields are
//! ignored. All actions are read-only EXCEPT `label`, which writes a composition
//! outcome (the one mutator) and is logged for audit.
//!
//! Deprecated in 4.0: `ghostlink` is the advertised surface and `graph` a
//! hidden alias. On a Strata log each action runs the matching GhostLink
//! mode (see `tools::ghostlink::mode_for_graph_action`).

use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

use vestige_core::Storage;

use crate::cognitive::CognitiveEngine;
// Reuse composed_graph's canonical outcome-label vocabulary (do not re-list).
use super::composed_graph::OUTCOME_TYPES;

/// Discriminated-union schema for the unified `graph` tool.
pub fn schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": [
                    "chain", "associations", "bridges",
                    "predict", "memory_graph",
                    "recent", "get", "memory", "neighbors",
                    "never_composed", "bounty_mode", "label"
                ],
                "description": "Reasoning: 'chain' (from, to), 'associations' (from), 'bridges' (from, to), 'predict' (context), 'memory_graph' (center_id or query). Composition: 'recent', 'get' (event_id), 'memory', 'neighbors' (memory_id), 'never_composed', 'bounty_mode', 'label' (records an outcome; the only write)."
            },
            // --- explore (chain/associations/bridges) ---
            "from": { "type": "string", "description": "[chain/associations/bridges] Source memory ID." },
            "to": { "type": "string", "description": "[chain/bridges] Target memory ID." },
            // --- predict ---
            "context": { "type": "object", "description": "[predict] Context: current_file, current_topics, codebase." },
            // --- memory_graph (viz subgraph) ---
            "center_id": { "type": "string", "description": "[memory_graph] Center node id (or use 'query')." },
            "query": { "type": "string", "description": "[memory_graph] Pick a center node by search query." },
            "depth": { "type": "integer", "minimum": 1, "maximum": 3, "description": "[memory_graph] Traversal depth (1-3, default 2)." },
            "max_nodes": { "type": "integer", "description": "[memory_graph] Max nodes (default 50, capped 200)." },
            // --- composed_graph ---
            "event_id": { "type": "string", "description": "[get/label] Composition event id." },
            "memory_id": { "type": "string", "description": "[memory/neighbors] Memory id." },
            "tags": { "type": "array", "items": { "type": "string" }, "description": "[never_composed/bounty_mode] Optional tag filter." },
            "outcome_type": {
                "type": "string",
                "enum": OUTCOME_TYPES,
                "description": "[label] Outcome to record (the only write)."
            },
            "scope": { "type": "string", "default": "user", "description": "[never_composed only] Exact project namespace; filtered before candidate scan limits. Other graph actions do not accept this control." },
            "includeCrossScope": { "type": "boolean", "default": false, "description": "[never_composed only] Explicitly consider candidates across project namespaces." },
            // --- shared ---
            "limit": { "type": "integer", "description": "Max results (per-action defaults, clamped).", "minimum": 1, "maximum": 100 }
        },
        "required": ["action"]
    })
}

/// Unified dispatcher for `graph`. Routes on `action`.
pub async fn execute(
    storage: &Arc<Storage>,
    cognitive: &Arc<Mutex<CognitiveEngine>>,
    args: Option<Value>,
) -> Result<Value, String> {
    let action = args
        .as_ref()
        .and_then(|a| a.get("action"))
        .and_then(|v| v.as_str())
        .ok_or("Missing 'action'. Use chain|associations|bridges|predict|memory_graph|recent|get|memory|neighbors|never_composed|bounty_mode|label.")?
        .to_string();

    if action != "never_composed"
        && args
            .as_ref()
            .is_some_and(|a| a.get("scope").is_some() || a.get("includeCrossScope").is_some())
    {
        return Err("scope and includeCrossScope currently apply only to never_composed".into());
    }
    // 4.0: on a Strata log every graph action is the matching GhostLink
    // mode, answered by the same recorded-structure engine.
    if crate::strata_memory::is_strata_backend(storage.as_ref()) {
        return super::ghostlink::execute_graph_action_on_strata(storage, &action, args).await;
    }
    match action.as_str() {
        // explore_connections — re-reads its own `action` (chain/associations/bridges).
        "chain" | "associations" | "bridges" => {
            super::explore::execute(storage, cognitive, args).await
        }
        // predict — reads `context`, ignores `action`.
        "predict" => super::predict::execute(storage, cognitive, args).await,
        // memory_graph — reads center_id/query/depth, ignores `action`.
        "memory_graph" => super::graph::execute(storage, args).await,
        // composed_graph — re-reads its own `action`. `label` is the only write.
        "recent" | "get" | "memory" | "neighbors" | "never_composed" | "bounty_mode" | "label" => {
            if action == "label" {
                let event_id = args
                    .as_ref()
                    .and_then(|a| a.get("event_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let outcome = args
                    .as_ref()
                    .and_then(|a| a.get("outcome_type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                tracing::info!(
                    event_id = %event_id,
                    outcome_type = %outcome,
                    "graph: composition outcome labeled"
                );
            }
            super::composed_graph::execute(storage, args).await
        }
        other => Err(format!(
            "Unknown graph action '{other}'. Use chain|associations|bridges|predict|memory_graph|recent|get|memory|neighbors|never_composed|bounty_mode|label."
        )),
    }
}
