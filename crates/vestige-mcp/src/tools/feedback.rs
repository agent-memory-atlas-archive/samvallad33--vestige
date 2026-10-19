//! Feedback Tools
//!
//! Promote and demote memories based on outcome quality.
//! Implements preference learning for Vestige.
//!
//! v1.5.0: Enhanced with cognitive pipeline:
//!   - Reward signal recording (4-channel importance)
//!   - Importance tracking (retrieval outcome)
//!   - Reconsolidation modification (labile window boost)
//!   - Activation network reinforcement

use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::cognitive::CognitiveEngine;
use vestige_core::{Modification, OutcomeType, Storage};

/// Input schema for promote_memory tool
pub fn promote_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "description": "The ID of the memory to promote"
            },
            "reason": {
                "type": "string",
                "description": "Why this memory was helpful (optional, for logging)"
            },
            "role": {
                "type": "string",
                "description": "CLAIMED role for provenance; resolved against the operator-controlled policy and never self-granting."
            }
        },
        "required": ["id"]
    })
}

/// Input schema for demote_memory tool
pub fn demote_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "description": "The ID of the memory to demote"
            },
            "reason": {
                "type": "string",
                "description": "Why this memory was unhelpful or wrong (optional, for logging)"
            },
            "role": {
                "type": "string",
                "description": "CLAIMED role for provenance; resolved against the operator-controlled policy and never self-granting."
            }
        },
        "required": ["id"]
    })
}

#[derive(Debug, Deserialize)]
struct FeedbackArgs {
    id: String,
    reason: Option<String>,
    /// #252 Phase A: claimed role for provenance. Never overrides the
    /// process identity and never self-grants authority.
    #[serde(alias = "claimed_role")]
    role: Option<String>,
}

/// Promote a memory (thumbs up) - it led to a good outcome
pub async fn execute_promote(
    storage: &Arc<Storage>,
    cognitive: &Arc<Mutex<CognitiveEngine>>,
    args: Option<Value>,
) -> Result<Value, String> {
    let args: FeedbackArgs = match args {
        Some(v) => serde_json::from_value(v).map_err(|e| format!("Invalid arguments: {}", e))?,
        None => return Err("Missing arguments".to_string()),
    };

    // SQLite ids are UUIDs. Strata ids are `mem-` + 16 hex chars.
    if uuid::Uuid::parse_str(&args.id).is_err() && !super::review::is_strata_memory_id(&args.id) {
        return Err("Invalid node ID format".to_string());
    }

    // Get node before for comparison
    let before = storage
        .get_node(&args.id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Node not found: {}", args.id))?;

    // #252 Phase A: with a bound process actor, the promote is an
    // actor-attributed endorsement (one transaction: mutation + evidence +
    // receipt). Without one, the historical path runs unchanged.
    let claimed_role = args
        .role
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    let endorsement = if storage.process_actor_did().is_some() {
        Some(
            storage
                .promote_memory_as_actor(&args.id, claimed_role, "promote_memory")
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let node = match &endorsement {
        Some(outcome) => outcome.node.clone(),
        None => storage
            .promote_memory(&args.id)
            .map_err(|e| e.to_string())?,
    };

    // ====================================================================
    // COGNITIVE FEEDBACK PIPELINE (promote)
    // ====================================================================
    if let Ok(mut cog) = cognitive.try_lock() {
        // 5A. Reward signal — record positive outcome
        cog.reward_signal
            .record_outcome(&args.id, OutcomeType::Helpful);

        // 5B. Importance tracking — mark as helpful retrieval
        cog.importance_tracker.on_retrieved(&args.id, true);

        // 5C. Reconsolidation — boost retrieval if memory is labile
        if cog.reconsolidation.is_labile(&args.id) {
            cog.reconsolidation.apply_modification(
                &args.id,
                Modification::StrengthenConnection {
                    target_memory_id: args.id.clone(),
                    boost: 0.2,
                },
            );
        }
    }

    let mut response = serde_json::json!({
        "success": true,
        "action": "promoted",
        "nodeId": node.id,
        "reason": args.reason,
        "changes": {
            "retrievalStrength": {
                "before": before.retrieval_strength,
                "after": node.retrieval_strength,
                "delta": "+0.20"
            },
            "retentionStrength": {
                "before": before.retention_strength,
                "after": node.retention_strength,
                "delta": "+0.10"
            },
            "stability": {
                "before": before.stability,
                "after": node.stability,
                "multiplier": "1.5x"
            }
        },
        "message": format!("Memory promoted. It will now surface more often in searches. Retrieval: {:.2} -> {:.2}",
            before.retrieval_strength, node.retrieval_strength),
    });
    if let Some(outcome) = &endorsement {
        response["actor"] = crate::actor_surface::actor_block(&outcome.endorsement);
        response["endorsement"] = crate::actor_surface::endorsement_block(outcome);
    }
    Ok(response)
}

/// Demote a memory (thumbs down) - it led to a bad outcome
pub async fn execute_demote(
    storage: &Arc<Storage>,
    cognitive: &Arc<Mutex<CognitiveEngine>>,
    args: Option<Value>,
) -> Result<Value, String> {
    let args: FeedbackArgs = match args {
        Some(v) => serde_json::from_value(v).map_err(|e| format!("Invalid arguments: {}", e))?,
        None => return Err("Missing arguments".to_string()),
    };

    // SQLite ids are UUIDs. Strata ids are `mem-` + 16 hex chars.
    if uuid::Uuid::parse_str(&args.id).is_err() && !super::review::is_strata_memory_id(&args.id) {
        return Err("Invalid node ID format".to_string());
    }

    // Get node before for comparison
    let before = storage
        .get_node(&args.id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Node not found: {}", args.id))?;

    let claimed_role = args
        .role
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    let endorsement = if storage.process_actor_did().is_some() {
        Some(
            storage
                .demote_memory_as_actor(&args.id, claimed_role, "demote_memory")
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let node = match &endorsement {
        Some(outcome) => outcome.node.clone(),
        None => storage.demote_memory(&args.id).map_err(|e| e.to_string())?,
    };

    // ====================================================================
    // COGNITIVE FEEDBACK PIPELINE (demote)
    // ====================================================================
    if let Ok(mut cog) = cognitive.try_lock() {
        // 5A. Reward signal — record negative outcome
        cog.reward_signal
            .record_outcome(&args.id, OutcomeType::NotHelpful);

        // 5B. Importance tracking — mark as unhelpful retrieval
        cog.importance_tracker.on_retrieved(&args.id, false);

        // 5C. Reconsolidation — weaken if memory is labile
        if cog.reconsolidation.is_labile(&args.id) {
            cog.reconsolidation.apply_modification(
                &args.id,
                Modification::AddContext {
                    context: "User reported this memory was wrong/unhelpful".to_string(),
                },
            );
        }
    }

    let mut response = serde_json::json!({
        "success": true,
        "action": "demoted",
        "nodeId": node.id,
        "reason": args.reason,
        "changes": {
            "retrievalStrength": {
                "before": before.retrieval_strength,
                "after": node.retrieval_strength,
                "delta": "-0.30"
            },
            "retentionStrength": {
                "before": before.retention_strength,
                "after": node.retention_strength,
                "delta": "-0.15"
            },
            "stability": {
                "before": before.stability,
                "after": node.stability,
                "multiplier": "0.5x"
            }
        },
        "message": format!("Memory demoted. Better alternatives will now surface instead. Retrieval: {:.2} -> {:.2}",
            before.retrieval_strength, node.retrieval_strength),
        "note": "Memory is NOT deleted - it remains searchable but ranks lower."
    });
    if let Some(outcome) = &endorsement {
        response["actor"] = crate::actor_surface::actor_block(&outcome.endorsement);
        response["endorsement"] = crate::actor_surface::endorsement_block(outcome);
    }
    Ok(response)
}

/// Input schema for request_feedback tool
pub fn request_feedback_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "description": "The ID of the memory to request feedback on"
            },
            "context": {
                "type": "string",
                "description": "What the memory was used for (e.g., 'error handling advice')"
            }
        },
        "required": ["id"]
    })
}

#[derive(Debug, Deserialize)]
struct RequestFeedbackArgs {
    id: String,
    context: Option<String>,
}

/// Request feedback from the user about a memory's usefulness
/// Returns a structured prompt for Claude to ask the user
pub async fn execute_request_feedback(
    storage: &Arc<Storage>,
    args: Option<Value>,
) -> Result<Value, String> {
    let args: RequestFeedbackArgs = match args {
        Some(v) => serde_json::from_value(v).map_err(|e| format!("Invalid arguments: {}", e))?,
        None => return Err("Missing arguments".to_string()),
    };

    // SQLite ids are UUIDs. Strata ids are `mem-` + 16 hex chars.
    if uuid::Uuid::parse_str(&args.id).is_err() && !super::review::is_strata_memory_id(&args.id) {
        return Err("Invalid node ID format".to_string());
    }

    let node = storage
        .get_node(&args.id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Node not found: {}", args.id))?;

    // Truncate content for display
    let preview: String = node.content.chars().take(100).collect();
    let preview = if node.content.len() > 100 {
        format!("{}...", preview)
    } else {
        preview
    };

    Ok(serde_json::json!({
        "action": "request_feedback",
        "nodeId": node.id,
        "memoryPreview": preview,
        "context": args.context,
        "prompt": "Was this memory helpful?",
        "options": [
            {
                "key": "A",
                "label": "Yes, helpful",
                "action": "promote",
                "description": "Memory will surface more often"
            },
            {
                "key": "B",
                "label": "No, wrong/outdated",
                "action": "demote",
                "description": "Better alternatives will surface instead"
            },
            {
                "key": "C",
                "label": "Ask Claude...",
                "action": "custom",
                "description": "Give Claude a custom instruction (e.g., 'update this memory', 'merge with X', 'add tag Y')"
            }
        ],
        "instruction": "PRESENT THESE OPTIONS TO THE USER. If they choose A, call memory(action='promote'). If B, call memory(action='demote'). If C, they will provide a custom instruction - execute it (could be: update the memory content, delete it, merge it, add tags, research something, etc.)."
    }))
}
