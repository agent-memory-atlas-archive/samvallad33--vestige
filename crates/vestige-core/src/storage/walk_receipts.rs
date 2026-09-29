//! Walk receipts: canonical backfill parameter envelopes for re-execution.
//!
//! A walk receipt freezes the exact parameter envelope of a Retroactive
//! Salience Backfill run (`tools::backfill` in the MCP layer) together with
//! the blake3 digest of its canonical JSON encoding. Replaying the receipt
//! re-executes the recorded parameters against the *current* store, so a
//! verdict can be re-derived and compared without trusting any cached
//! result. The table stores parameters only — never memory content.
//!
//! Canonicalization reuses the same helper the DSSE receipt chain already
//! uses (`serde_json_canonicalizer`, RFC 8785 / JCS): object keys sorted,
//! no insignificant whitespace, stable number formatting. Two envelopes
//! that differ only in key order or formatting canonicalize to identical
//! bytes and therefore identical digests and receipt ids.

use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use super::sqlite::{Result, SqliteMemoryStore, StorageError};

/// Stable schema URI for walk receipts.
pub const WALK_RECEIPT_SCHEMA_V1: &str = "https://vestige.dev/schemas/receipt/walk/v1";

// `WalkReceiptHandle`, `StoredWalkReceipt`, and `CoverageSnapshot` are
// defined in (and re-exported from) `crate::storage::types`.
pub use crate::storage::types::{CoverageSnapshot, StoredWalkReceipt, WalkReceiptHandle};

/// Canonicalize a walk parameter envelope with the codebase's canonical JSON
/// helper (RFC 8785 JCS via `serde_json_canonicalizer`, the same encoder the
/// receipt DSSE chain uses). Key order and whitespace of the input never
/// affect the output.
pub fn canonical_walk_json(params: &Value) -> Result<String> {
    if !params.is_object() {
        return Err(StorageError::Init(
            "walk receipt params must be a JSON object".into(),
        ));
    }
    let bytes = serde_json_canonicalizer::to_vec(params)
        .map_err(|error| StorageError::Init(format!("walk params canonicalization: {error}")))?;
    String::from_utf8(bytes)
        .map_err(|error| StorageError::Init(format!("canonical walk params not UTF-8: {error}")))
}

/// blake3 hex digest of canonical bytes — same primitive the anchor store
/// and the replay policy digests use.
fn walk_digest(canonical_json: &str) -> String {
    blake3::hash(canonical_json.as_bytes()).to_hex().to_string()
}

/// Deterministic receipt id derived from the digest: saving the same
/// canonical envelope twice addresses the same row.
fn walk_receipt_id(digest: &str) -> String {
    format!("wr_{}", &digest[..24])
}

impl SqliteMemoryStore {
    /// Persist one walk receipt. `canonical_json` must be exactly
    /// [`canonical_walk_json`]`(&params)`; passing anything else is a caller
    /// bug and is rejected, because the digest must always be computable
    /// from the stored bytes alone. Saving an already-stored envelope is
    /// idempotent: the existing row is kept and reported via
    /// `reused_existing`.
    pub fn save_walk_receipt(
        &self,
        canonical_json: &str,
        params: &Value,
    ) -> Result<WalkReceiptHandle> {
        let recomputed = canonical_walk_json(params)?;
        if recomputed != canonical_json {
            return Err(StorageError::Init(
                "canonical_json does not match the canonical form of params".into(),
            ));
        }
        // The parsed envelope must round-trip to the same bytes; this also
        // rejects a canonical_json that is not valid JSON.
        let parsed: Value = serde_json::from_str(canonical_json).map_err(|error| {
            StorageError::Init(format!("canonical_json is not valid JSON: {error}"))
        })?;
        if parsed != *params {
            return Err(StorageError::Init(
                "canonical_json and params must encode the same value".into(),
            ));
        }

        let digest = walk_digest(canonical_json);
        let receipt_id = walk_receipt_id(&digest);
        let writer = self
            .writer
            .lock()
            .map_err(|_| StorageError::Init("Writer lock poisoned".into()))?;
        let existing: Option<String> = writer
            .query_row(
                "SELECT receipt_id FROM walk_receipts WHERE digest = ?1",
                params![digest],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Ok(WalkReceiptHandle {
                receipt_id,
                digest,
                reused_existing: true,
            });
        }
        writer.execute(
            "INSERT INTO walk_receipts (receipt_id, digest, canonical_json, engine_version, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                receipt_id,
                digest,
                canonical_json,
                env!("CARGO_PKG_VERSION"),
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(WalkReceiptHandle {
            receipt_id,
            digest,
            reused_existing: false,
        })
    }

    /// Fetch one walk receipt by its deterministic id.
    pub fn get_walk_receipt(&self, receipt_id: &str) -> Result<Option<StoredWalkReceipt>> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        let row = reader
            .query_row(
                "SELECT receipt_id, digest, canonical_json, engine_version, created_at
                 FROM walk_receipts WHERE receipt_id = ?1",
                params![receipt_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((receipt_id, digest, canonical_json, engine_version, created_at)) = row else {
            return Ok(None);
        };
        let params: Value = serde_json::from_str(&canonical_json).map_err(|error| {
            StorageError::Init(format!("stored walk receipt is not valid JSON: {error}"))
        })?;
        Ok(Some(StoredWalkReceipt {
            receipt_id,
            digest,
            canonical_json,
            params,
            engine_version,
            created_at,
        }))
    }

    /// Exact SQL aggregates for the coverage view. Three aggregate queries
    /// plus two MAX probes, all against indexed columns; nothing scans
    /// content. Git-commit records are identified the same way the backfill
    /// identifies them: the `git-commit` tag on a knowledge node
    /// ([`crate::advanced::git_records::COMMIT_TAG`]).
    pub fn coverage_snapshot(&self) -> Result<CoverageSnapshot> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        let total_nodes: i64 =
            reader.query_row("SELECT COUNT(*) FROM knowledge_nodes", [], |row| row.get(0))?;
        let anchored_nodes: i64 = reader.query_row(
            "SELECT COUNT(DISTINCT node_id) FROM code_memory_anchors",
            [],
            |row| row.get(0),
        )?;

        let mut stmt = reader.prepare(
            "SELECT link_type, COUNT(*) FROM memory_connections
             GROUP BY link_type ORDER BY link_type ASC",
        )?;
        let edge_counts: Vec<(String, u64)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);

        let newest_git_commit_record: Option<String> = reader
            .query_row(
                "SELECT MAX(created_at) FROM knowledge_nodes
                 WHERE EXISTS (SELECT 1 FROM json_each(knowledge_nodes.tags) t
                               WHERE t.type = 'text' AND t.value = 'git-commit')",
                [],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let newest_agent_trace_at: Option<i64> = reader
            .query_row("SELECT MAX(at) FROM agent_traces", [], |row| row.get(0))
            .optional()?
            .flatten();

        let newest_git_commit_record_age_days = newest_git_commit_record
            .as_deref()
            .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
            .map(|parsed| {
                let age = Utc::now() - parsed.with_timezone(&Utc);
                age.num_days()
            });
        let newest_agent_trace_age_hours = newest_agent_trace_at.map(|millis| {
            let age_ms = Utc::now().timestamp_millis() - millis;
            ((age_ms as f64 / 3_600_000.0) * 10.0).round() / 10.0
        });

        let total_nodes = total_nodes.max(0) as u64;
        let anchored_nodes = anchored_nodes.max(0) as u64;
        let anchor_coverage_pct = if total_nodes == 0 {
            0.0
        } else {
            ((anchored_nodes as f64 / total_nodes as f64) * 100.0 * 100.0).round() / 100.0
        };

        Ok(CoverageSnapshot {
            total_nodes,
            anchored_nodes,
            anchor_coverage_pct,
            edge_counts_by_type: edge_counts,
            newest_git_commit_record,
            newest_git_commit_record_age_days,
            newest_agent_trace_at,
            newest_agent_trace_age_hours,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_store() -> (SqliteMemoryStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteMemoryStore::new(Some(dir.path().join("walk.db"))).unwrap();
        (store, dir)
    }

    #[test]
    fn canonicalization_is_stable_across_key_order_and_repeats() {
        let params = json!({
            "scope": "user",
            "failure_id": "mem_fail",
            "lookback_days": 30,
            "promote": false,
            "nested": {"b": 1, "a": [1, 2, {"z": true, "y": null}]}
        });
        let first = canonical_walk_json(&params).unwrap();
        let second = canonical_walk_json(&params).unwrap();
        assert_eq!(first, second, "same input must canonicalize identically");

        // Same value, different key order everywhere.
        let shuffled = json!({
            "nested": {"a": [1, 2, {"y": null, "z": true}], "b": 1},
            "promote": false,
            "lookback_days": 30,
            "failure_id": "mem_fail",
            "scope": "user"
        });
        let third = canonical_walk_json(&shuffled).unwrap();
        assert_eq!(
            first, third,
            "key-order shuffle must not change the digest input"
        );
        assert_eq!(walk_digest(&first), walk_digest(&third));

        // A genuinely different envelope must digest differently.
        let changed = json!({"scope": "user", "lookback_days": 31});
        assert_ne!(
            walk_digest(&first),
            walk_digest(&canonical_walk_json(&changed).unwrap())
        );

        // Non-object envelopes are refused.
        assert!(canonical_walk_json(&json!([1, 2])).is_err());
    }

    #[test]
    fn save_walk_receipt_is_idempotent_and_deterministic() {
        let (store, _dir) = test_store();
        let params = json!({"scope": "user", "failure_id": "f1", "lookback_days": 30});
        let canonical = canonical_walk_json(&params).unwrap();

        let first = store.save_walk_receipt(&canonical, &params).unwrap();
        assert!(!first.reused_existing);
        let second = store.save_walk_receipt(&canonical, &params).unwrap();
        assert_eq!(first.receipt_id, second.receipt_id);
        assert_eq!(first.digest, second.digest);
        assert!(second.reused_existing, "re-save must reuse the row");

        // Shuffled keys address the same row.
        let shuffled = json!({"lookback_days": 30, "failure_id": "f1", "scope": "user"});
        let third = store
            .save_walk_receipt(&canonical_walk_json(&shuffled).unwrap(), &shuffled)
            .unwrap();
        assert_eq!(third.receipt_id, first.receipt_id);
        assert!(third.reused_existing);

        // A mismatched (canonical_json, params) pair is rejected.
        let other = json!({"scope": "other"});
        assert!(store.save_walk_receipt(&canonical, &other).is_err());
        // Non-canonical bytes are rejected even when they encode params.
        let pretty = serde_json::to_string_pretty(&params).unwrap();
        assert!(store.save_walk_receipt(&pretty, &params).is_err());

        let stored = store.get_walk_receipt(&first.receipt_id).unwrap().unwrap();
        assert_eq!(stored.digest, first.digest);
        assert_eq!(stored.canonical_json, canonical);
        assert_eq!(stored.params, params);
        assert_eq!(stored.engine_version, env!("CARGO_PKG_VERSION"));
        assert!(store.get_walk_receipt("wr_nope").unwrap().is_none());
    }

    #[test]
    fn coverage_math_on_a_seeded_store() {
        let (store, _dir) = test_store();
        // Empty store: no NaN, no division by zero.
        let empty = store.coverage_snapshot().unwrap();
        assert_eq!(empty.total_nodes, 0);
        assert_eq!(empty.anchor_coverage_pct, 0.0);
        assert!(empty.edge_counts_by_type.is_empty());
        assert!(empty.newest_git_commit_record_age_days.is_none());
        assert!(empty.newest_agent_trace_age_hours.is_none());

        use crate::IngestInput;
        let anchored = store
            .ingest(IngestInput {
                content: "pattern: use blake3 for digests".into(),
                node_type: "pattern".into(),
                ..Default::default()
            })
            .unwrap();
        let mut facts = Vec::new();
        for i in 0..3 {
            facts.push(
                store
                    .ingest(IngestInput {
                        content: format!("plain fact {i}"),
                        ..Default::default()
                    })
                    .unwrap()
                    .id,
            );
        }
        let commit = store
            .ingest(IngestInput {
                content: "commit abcdef0 schema bump".into(),
                tags: vec!["git-commit".to_string()],
                ..Default::default()
            })
            .unwrap();

        use crate::codebase::CodeAnchor;
        store
            .record_code_anchors(&[CodeAnchor {
                id: "anc_1".into(),
                node_id: anchored.id.clone(),
                file_path: "src/lib.rs".into(),
                symbol: Some("digest".into()),
                symbol_kind: Some("function".into()),
                start_line: Some(1),
                end_line: Some(2),
                span_lines: Some(2),
                content_hash: Some("b3:abc".into()),
                captured_at: Utc::now(),
                last_verified_at: None,
                last_status: None,
            }])
            .unwrap();

        // memory_connections is keyed (source_id, target_id): distinct node
        // pairs are needed for per-type counts to accumulate.
        use crate::ConnectionRecord;
        let edges = [
            (commit.id.as_str(), "backfill_candidate"),
            (facts[0].as_str(), "backfill_candidate"),
            (facts[1].as_str(), "semantic"),
        ];
        for (i, (target, link)) in edges.into_iter().enumerate() {
            store
                .save_connection(&ConnectionRecord {
                    source_id: anchored.id.clone(),
                    target_id: target.to_string(),
                    strength: 0.5,
                    link_type: link.to_string(),
                    created_at: Utc::now(),
                    last_activated: Utc::now(),
                    activation_count: i as i32,
                })
                .unwrap();
        }

        // One Black Box trace event so the freshness probe has a row.
        store
            .append_trace_event(&crate::trace::MemoryTraceEvent::McpCall {
                run_id: "run_coverage_test".into(),
                tool: "memory_status".into(),
                args_hash: "0".into(),
                at: Utc::now().timestamp_millis(),
            })
            .unwrap();

        let snapshot = store.coverage_snapshot().unwrap();
        assert_eq!(snapshot.total_nodes, 5);
        assert_eq!(snapshot.anchored_nodes, 1);
        assert_eq!(snapshot.anchor_coverage_pct, 20.0);
        assert_eq!(
            snapshot.edge_counts_by_type,
            vec![
                ("backfill_candidate".to_string(), 2),
                ("semantic".to_string(), 1),
            ],
            "edge counts group by link_type and order ascending"
        );
        let age_days = snapshot
            .newest_git_commit_record_age_days
            .expect("git-commit record exists");
        assert!(
            age_days <= 1,
            "seeded commit is seconds old, got {age_days}"
        );
        let age_hours = snapshot
            .newest_agent_trace_age_hours
            .expect("ingest traces exist");
        assert!(
            age_hours < 1.0,
            "seeded traces are seconds old, got {age_hours}"
        );
    }
}
