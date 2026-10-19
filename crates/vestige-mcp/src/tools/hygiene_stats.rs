//! Full-store, agent-facing memory hygiene statistics.
//!
//! Aggregates the metadata-only snapshot supplied by `vestige-core`. Aggregate
//! counts always cover every stored row in the selected scope, including rows
//! outside their validity window and superseded rows. Only the two detail lists
//! are bounded.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use vestige_core::{
    ACCESS_LOG_RETENTION_DAYS, DEFAULT_MEMORY_SCOPE, HygieneNodeSummary, HygieneSnapshot, Storage,
    scan_secrets,
};

const DEFAULT_DETAIL_LIMIT: usize = 50;
const MAX_DETAIL_LIMIT: usize = 200;
const TAG_AUDIT_WINDOW: usize = 50;
/// The byTag aggregate is bounded to the top entries by count so a store with
/// tens of thousands of distinct tags cannot blow the 200k payload cap and
/// clip the entire stats JSON mid-document.
const MAX_TAG_COUNT_ENTRIES: usize = 200;

#[derive(Debug, Clone)]
struct TagAuditSummary {
    operation_id: String,
    operation_type: String,
    status: String,
    reason: Option<String>,
    affected_count: usize,
    created_at: String,
    reverted_at: Option<String>,
    scope: Option<String>,
    all_scopes: bool,
    source_tags: Vec<String>,
    target_tag: Option<String>,
}

#[derive(Debug, Clone)]
struct TagAuditWindow {
    operations: Vec<TagAuditSummary>,
    scanned_operations: usize,
    truncated: bool,
}

/// Return complete hygiene aggregates and bounded diagnostic lists.
pub async fn execute(storage: &Arc<Storage>, args: Option<Value>) -> Result<Value, String> {
    let args = args.unwrap_or_else(|| json!({}));
    let limit = parse_limit(&args)?;
    let all_scopes = parse_bool(&args, "all_scopes")?.unwrap_or(false);
    let requested_scope = parse_string(&args, "scope")?.map(str::trim);
    let selected_scope = if all_scopes {
        None
    } else {
        Some(match requested_scope {
            Some("") | None => DEFAULT_MEMORY_SCOPE,
            Some(scope) => scope,
        })
    };

    let snapshot = storage
        .hygiene_snapshot(selected_scope)
        .map_err(|error| format!("Failed to build hygiene snapshot: {error}"))?;
    let tag_audit = load_tag_audit(storage, selected_scope)?;

    Ok(build_response(
        snapshot,
        selected_scope,
        limit,
        Utc::now(),
        tag_audit,
    ))
}

fn parse_limit(args: &Value) -> Result<usize, String> {
    let Some(value) = args.get("limit") else {
        return Ok(DEFAULT_DETAIL_LIMIT);
    };
    let Some(limit) = value.as_u64() else {
        return Err("limit must be a positive integer".to_string());
    };
    if limit == 0 {
        return Err("limit must be at least 1".to_string());
    }
    Ok((limit as usize).min(MAX_DETAIL_LIMIT))
}

fn parse_bool(args: &Value, field: &str) -> Result<Option<bool>, String> {
    match args.get(field) {
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| format!("{field} must be a boolean")),
        None => Ok(None),
    }
}

fn parse_string<'a>(args: &'a Value, field: &str) -> Result<Option<&'a str>, String> {
    match args.get(field) {
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| format!("{field} must be a string")),
        None => Ok(None),
    }
}

fn load_tag_audit(
    storage: &Arc<Storage>,
    selected_scope: Option<&str>,
) -> Result<TagAuditWindow, String> {
    // Read tag operations directly (rather than filtering a general reflog
    // window) so unrelated merge activity cannot bury durable tag audits.
    let mut reflog = storage
        .list_tag_operations(TAG_AUDIT_WINDOW + 1, selected_scope)
        .map_err(|error| format!("Failed to read tag-operation audit log: {error}"))?;
    let truncated = reflog.len() > TAG_AUDIT_WINDOW;
    reflog.truncate(TAG_AUDIT_WINDOW);
    let scanned_operations = reflog.len();
    let operations = reflog
        .into_iter()
        .map(|operation| {
            let source_tags = operation
                .signals
                .as_ref()
                .and_then(|signals| signals.get("sourceTags"))
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let target_tag = operation
                .signals
                .as_ref()
                .and_then(|signals| signals.get("targetTag"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let scope = operation
                .signals
                .as_ref()
                .and_then(|signals| signals.get("scope"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let all_scopes = operation
                .signals
                .as_ref()
                .and_then(|signals| signals.get("allScopes"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            TagAuditSummary {
                operation_id: operation.id,
                operation_type: operation.op_type,
                status: operation.status,
                reason: operation.reason,
                affected_count: operation.affected_ids.len(),
                created_at: operation.created_at,
                reverted_at: operation.reverted_at,
                scope,
                all_scopes,
                source_tags,
                target_tag,
            }
        })
        .collect();

    Ok(TagAuditWindow {
        operations,
        scanned_operations,
        truncated,
    })
}

fn build_response(
    snapshot: HygieneSnapshot,
    selected_scope: Option<&str>,
    limit: usize,
    now: DateTime<Utc>,
    tag_audit: TagAuditWindow,
) -> Value {
    let nodes = snapshot.nodes;
    let total = nodes.len();
    let mut type_counts = BTreeMap::<String, usize>::new();
    let mut tag_counts = BTreeMap::<String, usize>::new();
    let mut age_counts = age_buckets();
    let mut retention_counts = retention_buckets();
    let mut untagged = 0usize;
    let mut within_validity_window = 0usize;
    let mut not_yet_valid = 0usize;
    let mut expired = 0usize;
    let mut invalid_temporal_bounds = 0usize;
    let mut superseded = 0usize;
    // Example entity IDs for the recommended actions below. Counts say how
    // many; these say WHICH, so an agent can act without re-scanning.
    const ACTION_ID_LIMIT: usize = 5;
    let mut expired_ids: Vec<&str> = Vec::new();
    let mut superseded_ids: Vec<&str> = Vec::new();
    let mut low_retention_ids: Vec<&str> = Vec::new();

    for node in &nodes {
        *type_counts.entry(node.node_type.clone()).or_default() += 1;

        if node.tags.is_empty() {
            untagged += 1;
        }
        // A count means "memories carrying this exact tag", not raw duplicate
        // array entries in a malformed legacy row.
        for tag in node.tags.iter().collect::<BTreeSet<_>>() {
            *tag_counts.entry(tag.clone()).or_default() += 1;
        }

        let age_days = now.signed_duration_since(node.created_at).num_days();
        let age_bucket = if node.created_at > now {
            "futureDated"
        } else if age_days <= 7 {
            "0-7d"
        } else if age_days <= 30 {
            "8-30d"
        } else if age_days <= 90 {
            "31-90d"
        } else if age_days <= 180 {
            "91-180d"
        } else {
            "181d+"
        };
        *age_counts.get_mut(age_bucket).expect("fixed age bucket") += 1;

        let retention_bucket = retention_bucket(node.retention_strength);
        *retention_counts
            .get_mut(retention_bucket)
            .expect("fixed retention bucket") += 1;
        if node.retention_strength <= 0.2 && low_retention_ids.len() < ACTION_ID_LIMIT {
            low_retention_ids.push(node.id.as_str());
        }

        if node
            .valid_from
            .zip(node.valid_until)
            .is_some_and(|(from, until)| from > until)
        {
            invalid_temporal_bounds += 1;
        }
        if node.valid_until.is_some_and(|until| until < now) {
            expired += 1;
            if expired_ids.len() < ACTION_ID_LIMIT {
                expired_ids.push(node.id.as_str());
            }
        } else if node.valid_from.is_some_and(|from| from > now) {
            not_yet_valid += 1;
        } else {
            within_validity_window += 1;
        }
        if node.superseded {
            superseded += 1;
            if superseded_ids.len() < ACTION_ID_LIMIT {
                superseded_ids.push(node.id.as_str());
            }
        }
    }

    let mut never_accessed: Vec<&HygieneNodeSummary> =
        nodes.iter().filter(|node| node.never_accessed).collect();
    never_accessed.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let never_accessed_total = never_accessed.len();
    let never_accessed_items: Vec<Value> = never_accessed
        .into_iter()
        .take(limit)
        .map(detail_item)
        .collect();

    // Memories created before the retained access-log window with zero
    // durable counters: their access history was pruned, so they are
    // reported separately, never claimed as never-accessed.
    let mut access_unknown: Vec<&HygieneNodeSummary> =
        nodes.iter().filter(|node| node.access_unknown).collect();
    access_unknown.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let access_unknown_total = access_unknown.len();
    let access_unknown_ids: Vec<&str> = access_unknown
        .into_iter()
        .take(limit)
        .map(|node| node.id.as_str())
        .collect();

    let mut largest: Vec<&HygieneNodeSummary> = nodes.iter().collect();
    largest.sort_by(|left, right| {
        right
            .content_bytes
            .cmp(&left.content_bytes)
            .then_with(|| left.id.cmp(&right.id))
    });
    let largest_items: Vec<Value> = largest.into_iter().take(limit).map(detail_item).collect();

    // Bound byTag to the top entries by count (ties broken by tag name asc)
    // so an unbounded tag vocabulary cannot clip the whole stats payload.
    let distinct_tag_total = tag_counts.len();
    let by_tag_truncated = distinct_tag_total > MAX_TAG_COUNT_ENTRIES;
    let tag_counts = if by_tag_truncated {
        let mut ranked: Vec<(String, usize)> = tag_counts.into_iter().collect();
        ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        ranked.truncate(MAX_TAG_COUNT_ENTRIES);
        ranked.into_iter().collect::<BTreeMap<String, usize>>()
    } else {
        tag_counts
    };

    let tag_operations: Vec<Value> = tag_audit
        .operations
        .into_iter()
        .map(|operation| {
            json!({
                "operationId": operation.operation_id,
                "operationType": operation.operation_type,
                "status": operation.status,
                "reason": operation.reason,
                "affectedCount": operation.affected_count,
                "createdAt": operation.created_at,
                "revertedAt": operation.reverted_at,
                "scope": operation.scope,
                "allScopes": operation.all_scopes,
                "sourceTags": operation.source_tags,
                "targetTag": operation.target_tag,
            })
        })
        .collect();

    // Actionable recommendations (upgrade/memory-status): every entry pairs a
    // hygiene count with the entity IDs to act on and a concrete next action
    // against an existing advertised tool. Only non-zero problems appear.
    let mut recommended_actions: Vec<Value> = Vec::new();
    {
        let mut push_action = |code: &str, count: usize, detail: &str, tool: &str, args: Value| {
            recommended_actions.push(json!({
                "code": code,
                "count": count,
                "detail": detail,
                "tool": tool,
                "args": args,
            }));
        };
        if expired > 0 {
            push_action(
                "expired_memories",
                expired,
                "Memories past their valid_until date remain stored; review and purge the ones with no audit value. Examples listed in memoryIds.",
                "purge",
                json!({ "exampleMemoryIds": expired_ids }),
            );
        }
        if superseded > 0 {
            push_action(
                "superseded_rows",
                superseded,
                "Superseded rows are kept intentionally for audit and undo; purge individual ids only when their history no longer matters. Examples listed in memoryIds.",
                "purge",
                json!({ "exampleMemoryIds": superseded_ids }),
            );
        }
        let low_retention = retention_counts["0-20%"] + retention_counts["below0"];
        if low_retention > 0 {
            push_action(
                "low_retention",
                low_retention,
                "Memories at or below 20% retention are decay candidates; run consolidation to replay and strengthen the ones still needed, or GC the rest. Examples listed in memoryIds.",
                "maintain",
                json!({ "action": "consolidate", "exampleMemoryIds": low_retention_ids }),
            );
        }
        if untagged > 0 {
            push_action(
                "untagged_memories",
                untagged,
                "Untagged memories are invisible to tag filters; tag new memories at ingest, and use dedup tag actions to consolidate tag vocabulary.",
                "smart_ingest",
                json!({ "hint": "pass tags on future ingests; dedup tag_rename/tag_merge for existing vocabulary" }),
            );
        }
        if snapshot.malformed_tag_rows > 0 {
            push_action(
                "malformed_tag_rows",
                snapshot.malformed_tag_rows,
                "Rows with unparseable tags JSON are treated as untagged; inspect the listed ids and re-save them to repair the column.",
                "memory",
                json!({ "hint": "memory get the listed ids, then re-save with valid tags" }),
            );
        }
        if never_accessed_total > 0 {
            push_action(
                "never_accessed",
                never_accessed_total,
                "Memories created inside the access-log window with zero retrievals; if they were predicted to matter, verify content and suppress or demote the dead weight. Examples in neverAccessed.memories.",
                "suppress",
                json!({ "hint": "review first; suppress is compounding and out of retrieval" }),
            );
        }
    }

    json!({
        "success": true,
        "view": "stats",
        "computedAt": now.to_rfc3339(),
        "scope": match selected_scope {
            Some(scope) => json!({ "mode": "single", "value": scope }),
            None => json!({ "mode": "all" }),
        },
        "semantics": {
            "population": "Every stored knowledge_nodes row in the selected scope, including expired, not-yet-valid, invalid-bound, and superseded rows.",
            "tagCounts": format!("Number of memories carrying each exact tag; duplicate entries within one memory count once. byTag is bounded to the top {MAX_TAG_COUNT_ENTRIES} tags by count (ties by tag name); distinctTagTotal and byTagTruncated report the full vocabulary size."),
            "age": "Whole elapsed days from createdAt to computedAt. Future-created rows are reported separately.",
            "validity": "Temporal buckets are mutually exclusive: expired first, then not-yet-valid, then within-window. Superseded and invalid-bound counts are independent overlays.",
            "neverAccessed": format!("No access recorded in the retained {ACCESS_LOG_RETENTION_DAYS}-day access log, durable retrieval counters (times_retrieved/times_useful) are zero, AND the memory was created inside that window — so the absence of evidence is meaningful. Access-log rows are pruned after {ACCESS_LOG_RETENTION_DAYS} days; they are not durable."),
            "accessUnknownPrunedLog": format!("Created before the retained {ACCESS_LOG_RETENTION_DAYS}-day access-log window with zero durable retrieval counters. Their pre-prune access history is unknowable, so they are reported here and never claimed as never-accessed. Do not suppress or purge them on access grounds."),
            "dataQuality": "malformedTagRows counts rows whose stored tags column is NULL or unparseable JSON (treated as untagged in aggregates); retentionDefaultedRows counts rows whose NULL retention_strength was reported at the schema default 1.0.",
            "contentSize": "UTF-8 bytes. Returned content is a bounded preview, never the full body. Blocking credential shapes are redacted without echoing the matched bytes."
        },
        "population": {
            "total": total,
            "includesInvalid": true,
            "includesSuperseded": true,
        },
        "counts": {
            "byMemoryType": type_counts,
            "byTag": tag_counts,
            "distinctTagTotal": distinct_tag_total,
            "byTagTruncated": by_tag_truncated,
            "untagged": untagged,
            "byAge": age_counts,
        },
        "dataQuality": {
            "malformedTagRows": {
                "count": snapshot.malformed_tag_rows,
                "memoryIds": snapshot.malformed_tag_row_ids,
                "memoryIdsTruncated": snapshot.malformed_tag_row_ids_truncated,
            },
            "retentionDefaultedRows": snapshot.defaulted_retention_rows,
        },
        "retentionDistribution": {
            "buckets": retention_counts,
        },
        "lifecycle": {
            "withinValidityWindow": within_validity_window,
            "notYetValid": not_yet_valid,
            "expired": expired,
            "invalidTemporalBounds": invalid_temporal_bounds,
            "superseded": superseded,
        },
        "neverAccessed": {
            "total": never_accessed_total,
            "returned": never_accessed_items.len(),
            "limit": limit,
            "truncated": never_accessed_total > never_accessed_items.len(),
            "memories": never_accessed_items,
        },
        "accessUnknownPrunedLog": {
            "total": access_unknown_total,
            "returned": access_unknown_ids.len(),
            "limit": limit,
            "truncated": access_unknown_total > access_unknown_ids.len(),
            "memoryIds": access_unknown_ids,
        },
        "largestNodes": {
            "total": total,
            "returned": largest_items.len(),
            "limit": limit,
            "truncated": total > largest_items.len(),
            "memories": largest_items,
        },
        "recommendedActions": {
            "note": "Only non-zero problems appear. memoryIds are bounded examples, not the full set; the matching count field is authoritative.",
            "actions": recommended_actions,
        },
        "recentTagOperations": {
            "source": "merge_operations memory reflog",
            "windowLimit": TAG_AUDIT_WINDOW,
            "scannedOperations": tag_audit.scanned_operations,
            "returned": tag_operations.len(),
            "truncated": tag_audit.truncated,
            "note": "Tag rename/merge operations are read directly from merge_operations so unrelated merge/supersede activity cannot bury them. Single-scope stats expose operations recorded for that exact scope plus every all-scopes operation (flagged allScopes=true), because an all-scopes mutation rewrote this scope's tags too; all-scopes stats expose every tag scope. A reverted operation remains visible with status and revertedAt.",
            "operations": tag_operations,
        },
    })
}

fn detail_item(node: &HygieneNodeSummary) -> Value {
    let redacted = scan_secrets(&node.content_preview)
        .iter()
        .any(vestige_core::SecretFinding::blocks_ingestion);
    json!({
        "id": node.id,
        "memoryType": node.node_type,
        "createdAt": node.created_at.to_rfc3339(),
        "contentBytes": node.content_bytes,
        "contentPreview": if redacted {
            "[redacted — probable credential in stored content]"
        } else {
            node.content_preview.as_str()
        },
        "contentPreviewRedacted": redacted,
        "tags": node.tags,
        "superseded": node.superseded,
    })
}

fn age_buckets() -> BTreeMap<&'static str, usize> {
    ["0-7d", "8-30d", "31-90d", "91-180d", "181d+", "futureDated"]
        .into_iter()
        .map(|bucket| (bucket, 0))
        .collect()
}

fn retention_buckets() -> BTreeMap<&'static str, usize> {
    [
        "below0",
        "0-20%",
        ">20-40%",
        ">40-60%",
        ">60-80%",
        ">80-100%",
        "above100%",
        "nonFinite",
    ]
    .into_iter()
    .map(|bucket| (bucket, 0))
    .collect()
}

fn retention_bucket(value: f64) -> &'static str {
    if !value.is_finite() {
        "nonFinite"
    } else if value < 0.0 {
        "below0"
    } else if value <= 0.2 {
        "0-20%"
    } else if value <= 0.4 {
        ">20-40%"
    } else if value <= 0.6 {
        ">40-60%"
    } else if value <= 0.8 {
        ">60-80%"
    } else if value <= 1.0 {
        ">80-100%"
    } else {
        "above100%"
    }
}
