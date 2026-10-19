//! `suppress` MCP Tool (v2.0.5) — Top-Down Active Forgetting
//!
//! Actively suppress a memory via top-down inhibitory control. Distinct from
//! `memory.delete` (which removes the row) and `memory.demote` (which is a
//! one-shot thumb-down). Each call compounds: suppression_count increments,
//! FSRS state is dealt a strong blow, and a background Rac1 cascade worker
//! (in the existing consolidation loop) will fade co-activated neighbors.
//!
//! Reversible within a 24-hour labile window via `reverse: true`.
//!
//! References:
//! - Anderson et al. (2025). Brain mechanisms underlying the inhibitory
//!   control of thought. Nat Rev Neurosci. DOI 10.1038/s41583-025-00929-y
//! - Cervantes-Sandoval & Davis (2020). Rac1 Impairs Forgetting-Induced
//!   Cellular Plasticity. Front Cell Neurosci. PMC7477079

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

use vestige_core::Storage;
use vestige_core::neuroscience::active_forgetting::{ActiveForgettingSystem, DEFAULT_LABILE_HOURS};

/// Input schema for the `suppress` tool.
pub fn schema() -> Value {
    json!({
        "type": "object",
        "description": "Hide a memory from every read; the log keeps its bytes. On Strata in 4.0 it cannot be undone and is not erasure. Legacy engine only: faster decay that compounds per call, accelerated decay of co-activated neighbours over 72 hours, and reversal within 24 hours when its snapshot still matches (journaled cascade effects are reversed atomically when neighbor state still matches).",
        "properties": {
            "id": {
                "type": "string",
                "description": "Memory id to suppress (or reverse-suppress on the legacy engine)"
            },
            "reason": {
                "type": "string",
                "description": "Optional note on why; logged."
            },
            "reverse": {
                "type": "boolean",
                "default": false,
                "description": "Legacy engine only (Strata refuses it): reverse a previous suppression. Requires a matching snapshot within the 24-hour labile window; later state changes and legacy suppressions need review. Includes journaled neighbor cascades when their state still matches."
            },
            "cascade_derived_from": {
                "type": "boolean",
                "default": false,
                "description": "If true, extend this suppression along derived_from edges (blast traversal, depth cap 5, exact, cycle-safe) before suppressing. Each derived target is routed through the same Memory-PR review gate as a direct suppress: Fast mode suppresses them directly; Risk-Gated/Paranoid opens one pending Memory PR per target and suppresses NOTHING until approved. Ignored by reverse=true."
            }
        },
        "required": ["id"]
    })
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SuppressArgs {
    id: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    reverse: bool,
    // Primary wire name is camelCase like the siblings; the snake_case
    // spelling (the name used in the schema description) is accepted too.
    #[serde(default, alias = "cascade_derived_from")]
    cascade_derived_from: bool,
}

pub async fn execute(storage: &Arc<Storage>, args: Option<Value>) -> Result<Value, String> {
    let args: SuppressArgs = match args {
        Some(v) => serde_json::from_value(v).map_err(|e| format!("Invalid arguments: {}", e))?,
        None => return Err("Missing arguments".to_string()),
    };

    if args.id.trim().is_empty() {
        return Err("'id' must not be empty".to_string());
    }
    // Basic UUID sanity check — don't reject if missing, but warn
    if !crate::tools::memory_unified::is_memory_id(&args.id) {
        return Err(format!("Invalid memory ID format: {}", args.id));
    }

    let sys = ActiveForgettingSystem::new();

    if args.reverse && crate::strata_memory::is_strata_backend(storage.as_ref()) {
        return Err("unavailable_in_4_0: suppress reverse is not available on Strata in Vestige 4.0: a Strata suppression cannot be undone. The memory's bytes stay on the log; it is only hidden from reads.".to_string());
    }
    if args.reverse {
        // Reverse path — only allowed within labile window.
        match storage.reverse_suppression(&args.id, sys.labile_hours) {
            Ok(node) => {
                let still_suppressed = node.suppression_count > 0;
                Ok(json!({
                    "success": true,
                    "action": "reverse",
                    "id": args.id,
                    "suppressionCount": node.suppression_count,
                    "stillSuppressed": still_suppressed,
                    "reversalScope": "local_state_and_journaled_cascades",
                    "journaledCascadeReversal": "atomic",
                    "unrecordedEffectsReversed": false,
                    "retentionStrength": node.retention_strength,
                    "retrievalStrength": node.retrieval_strength,
                    "stability": node.stability,
                    "message": if still_suppressed {
                        format!(
                            "Reversal applied. {} suppression(s) remain on this memory.",
                            node.suppression_count
                        )
                    } else {
                        "Local suppression reversed. Journaled cascade effects are reversed atomically when neighbor state still matches.".to_string()
                    },
                }))
            }
            Err(e) => Err(format!("Reverse failed: {}", e)),
        }
    } else {
        // Forward path — suppress + log reason + tell the user what will happen.
        let strata = crate::strata_memory::is_strata_backend(storage.as_ref());
        let before_count = if strata {
            0
        } else {
            storage
                .get_node(&args.id)
                .map_err(|e| format!("Failed to load memory: {}", e))?
                .map(|n| n.suppression_count)
                .unwrap_or(0)
        };

        // Optional derived_from cascade: compute the exact blast set FIRST
        // (scope before effect — the traversal happens before ANY
        // suppression), then route every derived target through the same
        // Memory-PR review gate a direct suppress hits (#296 purge
        // discipline). The primary id below is gated by the server pre-gate;
        // the cascade ids are new victims and are gated here, per id:
        // Fast mode executes directly, Risk-Gated/Paranoid holds each behind
        // a pending Memory PR and suppresses nothing until approved.
        let cascade = if args.cascade_derived_from {
            Some(derive_and_gate_cascade(storage, &args.id, args.reason.as_deref()).await?)
        } else {
            None
        };

        let node = storage
            .suppress_memory(&args.id)
            .map_err(|e| format!("Suppress failed: {}", e))?;

        if strata {
            let receipt_id = node
                .source
                .clone()
                .filter(|id| id.starts_with("eff-"))
                .ok_or_else(|| "suppress admitted but the retire receipt is missing".to_string())?;
            tracing::info!(
                id = %node.id,
                receipt_id = %receipt_id,
                reason = args.reason.as_deref().unwrap_or(""),
                "Memory suppressed"
            );
            let mut response = json!({
                "success": true,
                "action": "suppress",
                "id": node.id,
                "nodeId": node.id,
                "receiptId": receipt_id,
                "rule": "suppress",
                "message": "Retired; can't be retrieved.",
                "reason": args.reason,
            });
            if let Some(cascade) = cascade {
                response["cascadeDerivedFrom"] = cascade;
            }
            return Ok(response);
        }

        // Count how many neighbors will be cascaded over the coming 72h.
        // We don't run the cascade synchronously — it happens in the
        // background consolidation loop via `run_rac1_cascade_sweep`. But we
        // can give the user an estimate.
        let edges = storage
            .get_connections_for_memory(&args.id)
            .unwrap_or_default();
        let estimated_cascade = edges.len().min(100);

        let reversible_until = node
            .suppressed_at
            .map(|t| sys.reversible_until(t))
            .unwrap_or_else(chrono::Utc::now);
        let retrieval_penalty = sys.retrieval_penalty(node.suppression_count);

        tracing::info!(
            id = %args.id,
            count = node.suppression_count,
            reason = args.reason.as_deref().unwrap_or(""),
            "Memory suppressed"
        );

        Ok(json!({
            "success": true,
            "action": "suppress",
            "id": args.id,
            "suppressionCount": node.suppression_count,
            "priorCount": before_count,
            "retrievalPenalty": retrieval_penalty,
            "retentionStrength": node.retention_strength,
            "retrievalStrength": node.retrieval_strength,
            "stability": node.stability,
            "estimatedCascadeNeighbors": estimated_cascade,
            "cascadeDerivedFrom": cascade,
            "reversibleUntil": reversible_until.to_rfc3339(),
            "labileWindowHours": DEFAULT_LABILE_HOURS,
            "reason": args.reason,
            "message": format!(
                "Actively forgetting. Suppression #{} applied. ~{} co-activated neighbors will fade over the next 72h via Rac1 cascade. Reversible for {}h.",
                node.suppression_count, estimated_cascade, DEFAULT_LABILE_HOURS
            ),
            "citation": "Anderson et al. 2025, Nat Rev Neurosci, DOI: 10.1038/s41583-025-00929-y"
        }))
    }
}

/// Exact derived_from blast from `id`, each target gated like a direct
/// suppress. Returns a JSON summary for the tool response; suppression of
/// held targets does NOT happen until their Memory PR is approved.
async fn derive_and_gate_cascade(
    storage: &Arc<Storage>,
    id: &str,
    reason: Option<&str>,
) -> Result<Value, String> {
    let report = storage
        .blast_radius_with_link_types(id, false, &["derived_from"])
        .map_err(|e| format!("cascade traversal failed: {}", e))?;
    let targets: Vec<_> = report.affected.into_iter().filter(|a| a.id != id).collect();

    let mode = crate::trace_recorder::read_review_mode(storage);
    let mut entries = Vec::with_capacity(targets.len());
    let mut allowed: Vec<String> = Vec::new();
    let mut held = 0usize;

    for target in targets {
        let gate_args = Some(json!({"id": target.id, "reason": reason}));
        match crate::trace_recorder::gate_pending_memory_mutation(
            storage,
            None,
            "suppress_cascade_derived_from",
            "suppress",
            &gate_args,
            mode,
        ) {
            Ok(None) => {
                allowed.push(target.id.clone());
                entries.push(json!({
                    "id": target.id,
                    "via": target.via,
                    "depth": target.depth,
                    "outcome": "suppressed",
                }));
            }
            Ok(Some(pending)) => {
                held += 1;
                entries.push(json!({
                    "id": target.id,
                    "via": target.via,
                    "depth": target.depth,
                    "outcome": "pending_review",
                    "requiresReview": true,
                    "memoryPr": pending["memoryPrsOpened"][0]["id"],
                }));
            }
            Err(error) => {
                // Fail closed: a broken gate must not degrade into an
                // ungated cascade suppression.
                held += 1;
                entries.push(json!({
                    "id": target.id,
                    "via": target.via,
                    "depth": target.depth,
                    "outcome": "gate_error",
                    "requiresReview": true,
                    "error": error,
                }));
            }
        }
    }

    if !allowed.is_empty() {
        let refs: Vec<&str> = allowed.iter().map(String::as_str).collect();
        for outcome in storage.retire_affected(&refs, reason.unwrap_or("derived_from cascade")) {
            let entry = entries
                .iter_mut()
                .find(|e| e["id"] == json!(outcome.id))
                .expect("outcome id came from allowed set");
            if !outcome.suppressed {
                entry["outcome"] = json!("failed");
                entry["error"] = json!(outcome.error);
            }
        }
    }

    Ok(json!({
        "linkType": "derived_from",
        "targets": entries,
        "suppressed": entries.len() - held,
        "heldForReview": held,
        "note": if held > 0 {
            "Held targets were NOT suppressed; approve their Memory PRs (forget) or keep them (promote)."
        } else {
            "All derived_from targets suppressed through the existing suppression mechanism (no deletions)."
        },
    }))
}
