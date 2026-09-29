//! Actor provenance storage (#252 Phase A).
//!
//! Owns the operator-controlled versioned role/weight policy, the
//! process-actor binding, and the actor-attributed endorsement events.
//!
//! Transactional contract (acceptance gate 3): a memory mutation, its
//! provenance event, and its receipt commit in ONE immediate transaction, so
//! an evidence failure rolls the mutation back — the store can never hold a
//! strength change that lost its evidence.
//!
//! Idempotence contract (acceptance gate 2): endorsement event ids are
//! deterministic over (actor, memory, revision digest, stance), so the same
//! actor retrying — or switching hats within the same process — records ONE
//! stance. Self-support (an actor endorsing the revision it authored) is
//! recorded with `independent_prior = 0.0`.

use super::*;

use crate::actor::{ActorPolicySnapshot, RoleResolution, endorsement_event_id, revision_digest};
use crate::trace::{ActorProvenance, Receipt, ReceiptMutation};
use std::collections::{BTreeMap, BTreeSet};

// `EndorsementEventRecord` and `ActorMutationOutcome` are defined in (and
// re-exported from) `crate::storage::types`.
pub use crate::storage::types::{ActorMutationOutcome, EndorsementEventRecord};

const ACTOR_FEEDBACK_OPERATION: &str = "actor_feedback_mutation";

impl SqliteMemoryStore {
    // ========================================================================
    // Process actor binding
    // ========================================================================

    /// Bind the stable process actor did:key to this store. Every local node
    /// write stamps it as `author_actor_did`; endorsement paths require it.
    /// Call once at startup, after loading/minting the identity from
    /// `<data_dir>/actor.key`.
    pub fn set_process_actor(&self, did: &str) -> Result<()> {
        crate::actor::ed25519_public_key_from_did_key(did).map_err(|error| {
            StorageError::Init(format!(
                "process actor is not a valid Ed25519 did:key: {error}"
            ))
        })?;
        *self
            .process_actor_did
            .write()
            .map_err(|_| StorageError::Init("Process actor lock poisoned".into()))? =
            Some(did.to_string());
        Ok(())
    }

    /// The bound process actor did, if any.
    pub fn process_actor_did(&self) -> Option<String> {
        self.process_actor_did
            .read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    // ========================================================================
    // Operator-controlled policy (no MCP tool writes these in Phase A)
    // ========================================================================

    fn read_policy_snapshot(conn: &Connection) -> Result<ActorPolicySnapshot> {
        let policy_version: i64 = conn
            .query_row(
                "SELECT policy_version FROM actor_policy_state WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(crate::actor::INITIAL_POLICY_VERSION as i64);

        let mut weights = BTreeMap::new();
        {
            let mut stmt = conn.prepare("SELECT role, weight_prior FROM actor_role_weights")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            })?;
            for row in rows {
                let (role, weight) = row?;
                weights.insert(normalize_role(&role), weight);
            }
        }

        let mut memberships: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        {
            let mut stmt = conn.prepare("SELECT actor_did, role FROM actor_role_membership")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (actor_did, role) = row?;
                memberships
                    .entry(actor_did)
                    .or_default()
                    .insert(normalize_role(&role));
            }
        }

        Ok(ActorPolicySnapshot {
            policy_version: policy_version.max(0) as u64,
            weights,
            memberships,
        })
    }

    /// Freeze the current operator policy (weights, memberships, version).
    pub fn actor_policy_snapshot(&self) -> Result<ActorPolicySnapshot> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        Self::read_policy_snapshot(&reader)
    }

    /// Operator action: set one role's weight prior and bump the policy
    /// version transactionally. Bounds follow the issue's initial policy —
    /// positive and capped at 1.5, reserved for human operator authority.
    /// There is deliberately no MCP surface for this in Phase A.
    pub fn set_role_weight(&self, role: &str, weight_prior: f64) -> Result<u64> {
        let role = normalize_role(role);
        if role.is_empty() {
            return Err(StorageError::Init("role name must not be empty".into()));
        }
        if !(weight_prior > 0.0 && weight_prior <= crate::actor::MAX_ROLE_WEIGHT) {
            return Err(StorageError::Init(format!(
                "weight_prior must be in (0, {}]; the maximum is reserved for operator authority",
                crate::actor::MAX_ROLE_WEIGHT
            )));
        }
        let writer = self
            .writer
            .lock()
            .map_err(|_| StorageError::Init("Writer lock poisoned".into()))?;
        let tx = Self::begin_write_transaction(&writer, "actor_policy_weight")?;
        tx.execute(
            "INSERT INTO actor_role_weights (role, weight_prior, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(role) DO UPDATE SET weight_prior = ?2, updated_at = ?3",
            params![role, weight_prior, Utc::now().to_rfc3339()],
        )?;
        let version = Self::bump_policy_version(&tx)?;
        tx.commit()?;
        Ok(version)
    }

    /// Operator action: grant an actor a role. Membership ONLY flows through
    /// this store — ordinary callers cannot grant roles.
    pub fn grant_actor_role(&self, actor_did: &str, role: &str, note: Option<&str>) -> Result<u64> {
        let role = normalize_role(role);
        if role.is_empty() {
            return Err(StorageError::Init("role name must not be empty".into()));
        }
        let writer = self
            .writer
            .lock()
            .map_err(|_| StorageError::Init("Writer lock poisoned".into()))?;
        let tx = Self::begin_write_transaction(&writer, "actor_policy_grant")?;
        tx.execute(
            "INSERT INTO actor_role_membership (actor_did, role, granted_at, note)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(actor_did, role) DO UPDATE SET granted_at = ?3, note = ?4",
            params![actor_did, role, Utc::now().to_rfc3339(), note],
        )?;
        let version = Self::bump_policy_version(&tx)?;
        tx.commit()?;
        Ok(version)
    }

    /// Operator action: revoke an actor's role and bump the policy version.
    pub fn revoke_actor_role(&self, actor_did: &str, role: &str) -> Result<u64> {
        let role = normalize_role(role);
        let writer = self
            .writer
            .lock()
            .map_err(|_| StorageError::Init("Writer lock poisoned".into()))?;
        let tx = Self::begin_write_transaction(&writer, "actor_policy_revoke")?;
        tx.execute(
            "DELETE FROM actor_role_membership WHERE actor_did = ?1 AND role = ?2",
            params![actor_did, role],
        )?;
        let version = Self::bump_policy_version(&tx)?;
        tx.commit()?;
        Ok(version)
    }

    /// Bump `actor_policy_state.policy_version` inside an open transaction.
    fn bump_policy_version(tx: &rusqlite::Transaction<'_>) -> Result<u64> {
        tx.execute(
            "UPDATE actor_policy_state
             SET policy_version = policy_version + 1, updated_at = ?1
             WHERE id = 1",
            params![Utc::now().to_rfc3339()],
        )?;
        let version: i64 = tx.query_row(
            "SELECT policy_version FROM actor_policy_state WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(version.max(0) as u64)
    }

    /// Resolve the process actor's claimed role against the frozen policy.
    /// Returns the actor did plus the resolution. Fails when no process
    /// actor is bound — endorsement paths never guess an identity.
    pub fn resolve_actor_role(
        &self,
        claimed_role: Option<&str>,
    ) -> Result<(String, RoleResolution)> {
        let actor_did = self
            .process_actor_did()
            .ok_or_else(|| StorageError::Init("no process actor is bound to this store".into()))?;
        let snapshot = self.actor_policy_snapshot()?;
        let resolution = snapshot.resolve(&actor_did, claimed_role);
        Ok((actor_did, resolution))
    }

    // ========================================================================
    // Actor-attributed mutations (transactional: mutation + event + receipt)
    // ========================================================================

    /// Promote a memory as the process actor. The strength mutation, the
    /// endorsement event, and the receipt commit atomically; an evidence
    /// failure rolls the mutation back. Same-actor retries keep their single
    /// recorded stance (`already_recorded = true`) — no accumulated votes.
    pub fn promote_memory_as_actor(
        &self,
        id: &str,
        claimed_role: Option<&str>,
        tool: &str,
    ) -> Result<ActorMutationOutcome> {
        self.commit_actor_feedback(id, claimed_role, FeedbackKind::Promote, tool)
    }

    /// Demote a memory as the process actor (negative feedback). Same
    /// transactional and idempotence contract as promotion.
    pub fn demote_memory_as_actor(
        &self,
        id: &str,
        claimed_role: Option<&str>,
        tool: &str,
    ) -> Result<ActorMutationOutcome> {
        self.commit_actor_feedback(id, claimed_role, FeedbackKind::Demote, tool)
    }

    /// Record an endorsement for a duplicate/reinforce ingestion (the
    /// decision is that the existing revision is already correct). The
    /// reinforce strength bump, the event, and the receipt commit
    /// transactionally, exactly like promote/demote.
    pub fn record_reinforce_endorsement(
        &self,
        id: &str,
        claimed_role: Option<&str>,
        tool: &str,
    ) -> Result<ActorMutationOutcome> {
        self.commit_actor_feedback(id, claimed_role, FeedbackKind::Reinforce, tool)
    }

    /// Every recorded endorsement event, newest first, optionally filtered
    /// by memory or actor. Read path for "who said what".
    pub fn list_endorsement_events(
        &self,
        memory_id: Option<&str>,
        actor_did: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EndorsementEventRecord>> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        // Positional parameters must be contiguous, so the placeholders are
        // numbered per filter combination.
        let (memory_clause, actor_clause) = match (memory_id, actor_did) {
            (Some(_), Some(_)) => (" AND memory_id = ?1", " AND actor_did = ?2"),
            (Some(_), None) => (" AND memory_id = ?1", ""),
            (None, Some(_)) => ("", " AND actor_did = ?1"),
            (None, None) => ("", ""),
        };
        let sql = format!(
            "SELECT event_id, memory_id, actor_did, claimed_role, effective_role,
                    resolved_weight, resolution_disposition, policy_version,
                    endorsement_kind, revision_digest, independent_prior, tool,
                    receipt_id, created_at
             FROM actor_endorsement_events WHERE 1=1{}{}
             ORDER BY created_at DESC, event_id DESC LIMIT {}",
            memory_clause,
            actor_clause,
            limit.clamp(1, 500),
        );
        let mut stmt = reader.prepare(&sql)?;
        let map_row = |row: &rusqlite::Row| -> rusqlite::Result<EndorsementEventRecord> {
            Ok(EndorsementEventRecord {
                event_id: row.get(0)?,
                memory_id: row.get(1)?,
                actor_did: row.get(2)?,
                claimed_role: row.get(3)?,
                effective_role: row.get(4)?,
                resolved_weight: row.get(5)?,
                resolution_disposition: row.get(6)?,
                policy_version: row.get::<_, i64>(7)?.max(0) as u64,
                endorsement_kind: row.get(8)?,
                revision_digest: row.get(9)?,
                independent_prior: row.get(10)?,
                tool: row.get(11)?,
                receipt_id: row.get(12)?,
                created_at: row.get(13)?,
            })
        };
        let rows = match (memory_id, actor_did) {
            (Some(memory), Some(actor)) => stmt.query_map(params![memory, actor], map_row)?,
            (Some(memory), None) => stmt.query_map(params![memory], map_row)?,
            (None, Some(actor)) => stmt.query_map(params![actor], map_row)?,
            (None, None) => stmt.query_map([], map_row)?,
        };
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.into())
    }

    /// The bounded aggregate independent prior for one memory revision:
    /// each actor contributes its best event, the total is capped at
    /// [`crate::actor::MAX_AGGREGATE_ENDORSEMENT_WEIGHT`].
    pub fn endorsement_aggregate(&self, memory_id: &str, digest: &str) -> Result<f64> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        let mut stmt = reader.prepare(
            "SELECT actor_did, MAX(independent_prior)
             FROM actor_endorsement_events
             WHERE memory_id = ?1 AND revision_digest = ?2
             GROUP BY actor_did",
        )?;
        let rows = stmt.query_map(params![memory_id, digest], |row| row.get::<_, f64>(1))?;
        let priors: Vec<f64> = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(crate::actor::bounded_aggregate(&priors))
    }

    // ========================================================================
    // Transactional core
    // ========================================================================

    fn commit_actor_feedback(
        &self,
        id: &str,
        claimed_role: Option<&str>,
        kind: FeedbackKind,
        tool: &str,
    ) -> Result<ActorMutationOutcome> {
        let actor_did = self
            .process_actor_did()
            .ok_or_else(|| StorageError::Init("no process actor is bound to this store".into()))?;
        let now = Utc::now();
        let writer = self
            .writer
            .lock()
            .map_err(|_| StorageError::Init("Writer lock poisoned".into()))?;
        let tx = Self::begin_write_transaction(&writer, ACTOR_FEEDBACK_OPERATION)?;

        // Node before (fails closed: an unknown id rolls back with NotFound).
        let before = Self::read_node_in_tx(&tx, id)?;

        // Freeze the operator policy INSIDE the same transaction that mutates.
        let snapshot = Self::read_policy_snapshot(&tx)?;
        let resolution = snapshot.resolve(&actor_did, claimed_role);

        // Apply the mutation with the exact SQL the non-actor paths use, so
        // strength outcomes are byte-identical to the historical behavior.
        match kind {
            FeedbackKind::Promote => {
                tx.execute(
                    "UPDATE knowledge_nodes SET
                        last_accessed = ?1,
                        retrieval_strength = MIN(1.0, retrieval_strength + 0.20),
                        retention_strength = MIN(1.0, retention_strength + 0.10),
                        stability = stability * 1.5,
                        times_useful = COALESCE(times_useful, 0) + 1,
                        utility_score = CASE
                            WHEN COALESCE(times_retrieved, 0) > 0
                            THEN MIN(1.0, CAST(COALESCE(times_useful, 0) + 1 AS REAL) / COALESCE(times_retrieved, 0))
                            ELSE 1.0
                        END
                    WHERE id = ?2",
                    params![now.to_rfc3339(), id],
                )?;
                // v1.9.0 waking SWR tag rides in the same transaction.
                tx.execute(
                    "UPDATE knowledge_nodes SET waking_tag = TRUE, waking_tag_at = ?1 WHERE id = ?2",
                    params![now.to_rfc3339(), id],
                )?;
            }
            FeedbackKind::Demote => {
                tx.execute(
                    "UPDATE knowledge_nodes SET
                        retrieval_strength = MAX(0.05, retrieval_strength - 0.30),
                        retention_strength = MAX(0.05, retention_strength - 0.15),
                        stability = stability * 0.5
                    WHERE id = ?1",
                    params![id],
                )?;
            }
            FeedbackKind::Reinforce => {
                tx.execute(
                    "UPDATE knowledge_nodes SET
                        last_accessed = ?1,
                        retrieval_strength = MIN(1.0, retrieval_strength + 0.05),
                        retention_strength = MIN(1.0, retention_strength + 0.02),
                        times_retrieved = COALESCE(times_retrieved, 0) + 1,
                        utility_score = CASE
                            WHEN COALESCE(times_retrieved, 0) + 1 > 0
                            THEN CAST(COALESCE(times_useful, 0) AS REAL) / (COALESCE(times_retrieved, 0) + 1)
                            ELSE 0.0
                        END
                    WHERE id = ?2",
                    params![now.to_rfc3339(), id],
                )?;
            }
        }
        // Access-log audit line inside the same transaction.
        tx.execute(
            "INSERT INTO memory_access_log (node_id, access_type, accessed_at)
             VALUES (?1, ?2, ?3)",
            params![id, kind.access_type(), now.to_rfc3339()],
        )?;

        let node = Self::read_node_in_tx(&tx, id)?;

        // Revision binding + authorship, read from the same transaction.
        let digest = revision_digest(&before.content);
        let author: Option<String> = tx.query_row(
            "SELECT author_actor_did FROM knowledge_nodes WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        let is_self_support = matches!(kind, FeedbackKind::Promote | FeedbackKind::Reinforce)
            && author.as_deref() == Some(actor_did.as_str());
        let endorsement_kind = match (kind, is_self_support) {
            (FeedbackKind::Demote, _) => "oppose",
            (_, true) => "self_support",
            (_, false) => "support",
        };
        // Self-support contributes exactly zero independent prior (gate 2).
        let independent_prior = if is_self_support {
            0.0
        } else {
            resolution.resolved_weight
        };

        let event_id = endorsement_event_id(&actor_did, id, &digest, endorsement_kind);
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT revision_digest, endorsement_kind FROM actor_endorsement_events
                 WHERE event_id = ?1",
                params![event_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let mut already_recorded = false;
        match existing {
            Some((existing_digest, existing_kind))
                if existing_digest == digest && existing_kind == endorsement_kind =>
            {
                // The same stance on the same revision from the same actor:
                // one actor, one vote. The mutation still applies (matching
                // the historical retry behavior), but no second vote lands.
                already_recorded = true;
            }
            Some((existing_digest, existing_kind)) => {
                // A deterministic-id collision with different evidence is a
                // corrupted or hostile store: fail the evidence write so the
                // whole mutation rolls back (gate 3).
                tx.rollback()?;
                return Err(StorageError::Init(format!(
                    "endorsement evidence mismatch for event {event_id}: recorded ({existing_digest}, {existing_kind}), attempted ({digest}, {endorsement_kind})"
                )));
            }
            None => {}
        }

        // Build the mutation receipt with the provenance block, deterministic
        // id, and the resolution recorded verbatim.
        let mutation_kind = match kind {
            FeedbackKind::Promote => "promoted",
            FeedbackKind::Demote => "demoted",
            FeedbackKind::Reinforce => "reinforced",
        };
        let receipt = Receipt::build_with_unique(
            now,
            "actor",
            &event_id[..event_id.len().min(6)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &[node.retrieval_strength],
            vec![ReceiptMutation {
                id: id.to_string(),
                kind: mutation_kind.to_string(),
                note: Some(format!(
                    "actor {} ({}) resolved_weight {:.2} policy v{}{}",
                    resolution.effective_role,
                    resolution.disposition.as_str(),
                    resolution.resolved_weight,
                    resolution.policy_version,
                    if already_recorded {
                        "; stance already recorded"
                    } else {
                        ""
                    }
                )),
            }],
        )
        .with_actor_provenance(ActorProvenance::from_resolution(&actor_did, &resolution));
        let receipt_id = receipt.receipt_id.clone();

        if !already_recorded {
            tx.execute(
                "INSERT INTO actor_endorsement_events (
                    event_id, memory_id, actor_did, claimed_role, effective_role,
                    resolved_weight, resolution_disposition, policy_version,
                    endorsement_kind, revision_digest, independent_prior, tool,
                    receipt_id, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    event_id,
                    id,
                    actor_did,
                    resolution.claimed_role,
                    resolution.effective_role,
                    resolution.resolved_weight,
                    resolution.disposition.as_str(),
                    resolution.policy_version as i64,
                    endorsement_kind,
                    digest,
                    independent_prior,
                    tool,
                    receipt.receipt_id,
                    now.to_rfc3339(),
                ],
            )?;
        }

        // The receipt persists in the SAME transaction (gate 3): INSERT OR
        // REPLACE keeps a retry idempotent, consistent with save_receipt.
        let payload = serde_json::to_string(&receipt)
            .map_err(|e| StorageError::Init(format!("receipt serialize: {e}")))?;
        tx.execute(
            "INSERT OR REPLACE INTO memory_receipts
                 (receipt_id, run_id, tool, query, retrieved_count, suppressed_count,
                  trust_floor, decay_risk, payload, created_at)
             VALUES (?1, NULL, ?2, NULL, 0, 0, ?3, ?4, ?5, ?6)",
            params![
                receipt.receipt_id,
                tool,
                receipt.trust_floor,
                receipt.decay_risk.as_str(),
                payload,
                now.to_rfc3339(),
            ],
        )?;

        tx.commit()?;
        Ok(ActorMutationOutcome {
            before,
            node,
            receipt,
            endorsement: EndorsementEventRecord {
                event_id,
                memory_id: id.to_string(),
                actor_did,
                claimed_role: resolution.claimed_role.clone(),
                effective_role: resolution.effective_role,
                resolved_weight: resolution.resolved_weight,
                resolution_disposition: resolution.disposition.as_str().to_string(),
                policy_version: resolution.policy_version,
                endorsement_kind: endorsement_kind.to_string(),
                revision_digest: digest,
                independent_prior,
                tool: tool.to_string(),
                receipt_id: Some(receipt_id),
                created_at: now.to_rfc3339(),
            },
            already_recorded,
        })
    }

    /// Read one node through an open write transaction (the writer's own
    /// uncommitted view — the reader connection cannot see it).
    fn read_node_in_tx(tx: &rusqlite::Transaction<'_>, id: &str) -> Result<KnowledgeNode> {
        tx.query_row(
            "SELECT * FROM knowledge_nodes WHERE id = ?1",
            params![id],
            |row| Self::row_to_node(row),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => StorageError::NotFound(id.to_string()),
            other => other.into(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeedbackKind {
    Promote,
    Demote,
    Reinforce,
}

impl FeedbackKind {
    fn access_type(self) -> &'static str {
        match self {
            Self::Promote => "promote",
            Self::Demote => "demote",
            Self::Reinforce => "reinforce",
        }
    }
}

/// Normalize a role name: trim, lowercase. Role identity is case-insensitive
/// so `QA` and `qa` resolve identically.
fn normalize_role(role: &str) -> String {
    role.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IngestInput;
    use crate::actor::{ProcessActor, ResolutionDisposition};

    fn test_storage() -> (SqliteMemoryStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage =
            SqliteMemoryStore::new(Some(dir.path().join("actor-test.db"))).expect("open store");
        (storage, dir)
    }

    fn ingest_node(storage: &SqliteMemoryStore, content: &str) -> KnowledgeNode {
        storage
            .ingest(IngestInput {
                content: content.to_string(),
                node_type: "fact".to_string(),
                source: None,
                sentiment_score: 0.0,
                sentiment_magnitude: 0.0,
                tags: vec![],
                valid_from: None,
                valid_until: None,
                validity_inferred: false,
                source_envelope: None,
            })
            .expect("ingest")
    }

    /// Bind a freshly minted process actor, as the MCP server does at startup.
    fn bind_actor(storage: &SqliteMemoryStore) -> String {
        let actor = ProcessActor::mint();
        storage.set_process_actor(actor.did()).expect("bind actor");
        actor.did().to_string()
    }

    // ========================================================================
    // Gate 1: env-only startup, neutral unregistered actors, identity immune
    // to claimed roles.
    // ========================================================================

    #[test]
    fn gate1_claimed_roles_cannot_override_identity_or_grant_authority() {
        let (storage, _dir) = test_storage();
        let did = bind_actor(&storage);
        let node = ingest_node(&storage, "the deploy window is 14:00-15:00 UTC");

        // A caller claims the most privileged role it has no membership for.
        let outcome = storage
            .promote_memory_as_actor(&node.id, Some("operator"), "promote_memory")
            .expect("promote with an unearned claim still mutates");
        let resolution = &outcome.receipt.actor.as_ref().expect("provenance block");
        assert_eq!(resolution.actor_id, did, "identity comes from the process");
        assert_eq!(resolution.claimed_role.as_deref(), Some("operator"));
        assert_eq!(resolution.effective_role, "unattributed");
        assert_eq!(resolution.resolved_weight, 1.0);
        assert_eq!(resolution.resolution_disposition, "unregistered_claim");
        assert_eq!(resolution.policy_version, 1);

        // The event records the same neutral resolution.
        assert_eq!(outcome.endorsement.effective_role, "unattributed");
        assert_eq!(outcome.endorsement.resolved_weight, 1.0);
        assert_eq!(
            outcome.endorsement.resolution_disposition,
            "unregistered_claim"
        );
    }

    #[test]
    fn gate1_legacy_and_unbound_nodes_default_to_unattributed() {
        let (storage, _dir) = test_storage();
        // Ingest BEFORE any actor is bound: the legacy shape, NULL author.
        let node = ingest_node(&storage, "pre-actor memory");
        let author: Option<String> = {
            let reader = storage.reader.lock().unwrap();
            reader
                .query_row(
                    "SELECT author_actor_did FROM knowledge_nodes WHERE id = ?1",
                    params![node.id],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert_eq!(author, None, "no actor bound -> unattributed, no backfill");

        bind_actor(&storage);
        let created = ingest_node(&storage, "post-actor memory");
        let author: Option<String> = {
            let reader = storage.reader.lock().unwrap();
            reader
                .query_row(
                    "SELECT author_actor_did FROM knowledge_nodes WHERE id = ?1",
                    params![created.id],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert_eq!(
            author.as_deref(),
            Some(storage.process_actor_did().unwrap().as_str())
        );
    }

    // ========================================================================
    // Gate 2: same-actor hats, retries, and self-support are not votes.
    // ========================================================================

    #[test]
    fn gate2_same_actor_retries_and_hats_create_no_independent_votes() {
        let (storage, _dir) = test_storage();
        // Ingest before any actor is bound: the node is unattributed, so a
        // later support is a plain "support", not self-support.
        let node = ingest_node(&storage, "retry and hat target");
        bind_actor(&storage);

        // First support: records the stance.
        let first = storage
            .promote_memory_as_actor(&node.id, Some("qa"), "promote_memory")
            .expect("first promote");
        assert!(!first.already_recorded);
        // Retry under a DIFFERENT claimed hat: same actor, one stance.
        let second = storage
            .promote_memory_as_actor(&node.id, Some("dev"), "promote_memory")
            .expect("retry under another hat");
        assert!(second.already_recorded, "the hat switch is the same actor");

        let events = storage
            .list_endorsement_events(Some(&node.id), None, 50)
            .expect("list");
        assert_eq!(
            events.len(),
            1,
            "same-actor retries/hats must not accumulate votes, got {events:?}"
        );
    }

    #[test]
    fn gate2_self_support_is_marked_zero_independent_prior() {
        let (storage, _dir) = test_storage();
        let did = bind_actor(&storage);
        // Authored by this process actor.
        let node = ingest_node(&storage, "my own earlier claim");
        let outcome = storage
            .promote_memory_as_actor(&node.id, Some("qa"), "promote_memory")
            .expect("promote own memory");
        assert_eq!(outcome.endorsement.endorsement_kind, "self_support");
        assert_eq!(outcome.endorsement.independent_prior, 0.0);

        // A DIFFERENT process actor (shared-store second process) supporting
        // the same revision is genuinely independent: full resolved prior.
        let other = ProcessActor::mint();
        storage.set_process_actor(other.did()).expect("rebind");
        let independent = storage
            .promote_memory_as_actor(&node.id, None, "promote_memory")
            .expect("independent promote");
        assert_eq!(independent.endorsement.endorsement_kind, "support");
        assert_eq!(independent.endorsement.independent_prior, 1.0);
        assert_ne!(independent.endorsement.actor_did, did);
    }

    #[test]
    fn gate2_support_is_revision_bound() {
        let (storage, _dir) = test_storage();
        let node = ingest_node(&storage, "revision one content");
        bind_actor(&storage);
        let first = storage
            .promote_memory_as_actor(&node.id, None, "promote_memory")
            .expect("support revision one");
        assert_eq!(
            first.endorsement.revision_digest,
            crate::actor::revision_digest("revision one content")
        );

        // The content is edited: the old verification must NOT inherit.
        storage
            .update_node_content(&node.id, "revision two content")
            .expect("edit");
        let second = storage
            .promote_memory_as_actor(&node.id, None, "promote_memory")
            .expect("support after edit is a NEW stance");
        assert!(!second.already_recorded);
        assert_eq!(
            second.endorsement.revision_digest,
            crate::actor::revision_digest("revision two content")
        );
        let events = storage
            .list_endorsement_events(Some(&node.id), None, 50)
            .expect("list");
        assert_eq!(
            events.len(),
            2,
            "different revisions bind different stances"
        );
    }

    // ========================================================================
    // Gate 3: mutation + evidence + receipt commit in one transaction.
    // ========================================================================

    #[test]
    fn gate3_evidence_failure_rolls_back_the_mutation() {
        let (storage, _dir) = test_storage();
        let node = ingest_node(&storage, "rollback target content");
        let did = bind_actor(&storage);
        let before = storage.get_node(&node.id).unwrap().unwrap();

        // Corrupt the evidence surface (external store tampering): a row
        // already occupies the exact deterministic event id but its payload
        // binds a DIFFERENT revision. The endorsement write must detect the
        // mismatch and undo the promote entirely.
        let tampered_digest = crate::actor::revision_digest("different revision entirely");
        let event_id = crate::actor::endorsement_event_id(
            &did,
            &node.id,
            &crate::actor::revision_digest("rollback target content"),
            "support",
        );
        {
            let writer = storage.writer.lock().unwrap();
            writer
                .execute(
                    "INSERT INTO actor_endorsement_events (
                        event_id, memory_id, actor_did, claimed_role, effective_role,
                        resolved_weight, resolution_disposition, policy_version,
                        endorsement_kind, revision_digest, independent_prior, tool,
                        receipt_id, created_at
                     ) VALUES (?1, ?2, ?3, NULL, 'unattributed', 1.0, 'unclaimed', 1,
                               'support', ?4, 1.0, 'tampered', NULL, datetime('now'))",
                    params![event_id, node.id, did, tampered_digest],
                )
                .unwrap();
        }

        let result = storage.promote_memory_as_actor(&node.id, None, "promote_memory");
        assert!(result.is_err(), "evidence mismatch must fail the call");

        let after = storage.get_node(&node.id).unwrap().unwrap();
        assert_eq!(
            after.retrieval_strength, before.retrieval_strength,
            "the strength mutation rolled back with the failed evidence"
        );
        assert_eq!(
            after.times_useful, before.times_useful,
            "no partially-applied bookkeeping survives the rollback"
        );
        // The tampered row is the only event; nothing new was committed.
        let events = storage
            .list_endorsement_events(Some(&node.id), None, 50)
            .expect("list");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].revision_digest, tampered_digest);
        // And no receipt leaked out of the aborted transaction.
        let receipts: i64 = {
            let reader = storage.reader.lock().unwrap();
            reader
                .query_row("SELECT COUNT(*) FROM memory_receipts", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(receipts, 0, "no receipt from a rolled-back transaction");
    }

    #[test]
    fn gate3_success_commits_mutation_event_and_receipt_together() {
        let (storage, _dir) = test_storage();
        let node = ingest_node(&storage, "happy path target");
        bind_actor(&storage);
        // Fresh nodes start at retrieval 1.0; demote first so the promote's
        // +0.20 is actually observable.
        storage.demote_memory(&node.id).expect("pre-demote");
        let weakened = storage.get_node(&node.id).unwrap().unwrap();
        let outcome = storage
            .promote_memory_as_actor(&node.id, Some("qa"), "promote_memory")
            .expect("promote");

        // Membership is operator-controlled: nothing granted yet, so the
        // neutral resolution is recorded — but all three artifacts exist.
        let stored = storage.get_receipt(&outcome.receipt.receipt_id).unwrap();
        assert!(stored.is_some(), "receipt persisted in the same commit");
        assert!(stored.unwrap().actor.is_some());
        let after = storage.get_node(&node.id).unwrap().unwrap();
        assert!(
            after.retrieval_strength > weakened.retrieval_strength,
            "the strength mutation committed with its evidence"
        );
        assert_eq!(
            storage
                .list_endorsement_events(Some(&node.id), None, 10)
                .unwrap()
                .len(),
            1
        );
    }

    // ========================================================================
    // Gate 11: versioned operator policy, bounded aggregation, neutral
    // fallback.
    // ========================================================================

    #[test]
    fn gate11_policy_changes_are_versioned_and_recorded() {
        let (storage, _dir) = test_storage();
        assert_eq!(storage.actor_policy_snapshot().unwrap().policy_version, 1);
        let node = ingest_node(&storage, "policy versioning target");
        let did = bind_actor(&storage);
        let _ = ResolutionDisposition::Unclaimed;

        // Operator grants the qa role at the seeded 1.10 prior.
        let v2 = storage
            .grant_actor_role(&did, "QA", Some("sam reviewed this actor"))
            .expect("grant");
        assert_eq!(v2, 2, "membership changes bump the policy version");

        let outcome = storage
            .promote_memory_as_actor(&node.id, Some("qa"), "promote_memory")
            .expect("granted promote");
        assert_eq!(outcome.endorsement.effective_role, "qa");
        assert!((outcome.endorsement.resolved_weight - 1.10).abs() < 1e-9);
        assert_eq!(outcome.endorsement.policy_version, 2);
        assert_eq!(outcome.endorsement.independent_prior, 1.10);
        let receipt_actor = outcome.receipt.actor.as_ref().unwrap();
        assert_eq!(receipt_actor.policy_version, 2);

        // Operator retunes the prior: the next resolution records v3.
        let v3 = storage.set_role_weight("qa", 1.20).expect("reweight");
        assert_eq!(v3, 3);
        let resolution = storage.resolve_actor_role(Some("qa")).unwrap().1;
        assert!((resolution.resolved_weight - 1.20).abs() < 1e-9);
        assert_eq!(resolution.policy_version, 3);

        // The earlier event keeps its v2 history — versions never rewrite.
        let events = storage
            .list_endorsement_events(Some(&node.id), None, 10)
            .unwrap();
        assert_eq!(events[0].policy_version, 2);

        // Out-of-bounds weights are rejected: the max is reserved.
        assert!(storage.set_role_weight("operator", 1.51).is_err());
        assert!(storage.set_role_weight("operator", 0.0).is_err());
    }

    #[test]
    fn gate11_unknown_roles_and_unregistered_actors_fall_back_neutral() {
        let (storage, _dir) = test_storage();
        let _did = bind_actor(&storage);
        // Unregistered role name.
        let resolution = storage
            .resolve_actor_role(Some("shadow-emperor"))
            .unwrap()
            .1;
        assert_eq!(resolution.effective_role, "unattributed");
        assert_eq!(resolution.resolved_weight, 1.0);
        assert_eq!(
            resolution.disposition,
            ResolutionDisposition::UnregisteredClaim
        );
        // No claim at all.
        let unclaimed = storage.resolve_actor_role(None).unwrap().1;
        assert_eq!(unclaimed.effective_role, "unattributed");
        assert_eq!(unclaimed.resolved_weight, 1.0);
        // Membership granted, then the weight row is removed by the operator
        // (direct store edit — a real operator action on the store).
        let did = storage.process_actor_did().unwrap();
        storage.grant_actor_role(&did, "ghost", None).unwrap();
        {
            let writer = storage.writer.lock().unwrap();
            writer
                .execute("DELETE FROM actor_role_weights WHERE role = 'ghost'", [])
                .unwrap();
        }
        let ghost = storage.resolve_actor_role(Some("ghost")).unwrap().1;
        assert_eq!(ghost.resolved_weight, 1.0);
        assert_eq!(ghost.disposition, ResolutionDisposition::UnknownRoleNeutral);
    }

    #[test]
    fn gate11_aggregate_is_bounded_across_independent_actors() {
        let (storage, _dir) = test_storage();
        let node = ingest_node(&storage, "bounded aggregate target");
        let digest = crate::actor::revision_digest("bounded aggregate target");
        let did = bind_actor(&storage);

        storage
            .promote_memory_as_actor(&node.id, None, "promote_memory")
            .expect("first actor");
        // A second and third process on the shared store.
        for _ in 0..2 {
            let other = ProcessActor::mint();
            storage.set_process_actor(other.did()).unwrap();
            storage
                .promote_memory_as_actor(&node.id, None, "promote_memory")
                .expect("independent actor");
        }
        // One of them retries: still one stance per actor.
        storage
            .promote_memory_as_actor(&node.id, None, "promote_memory")
            .expect("retry");

        let aggregate = storage.endorsement_aggregate(&node.id, &digest).unwrap();
        assert!(
            (aggregate - 3.0).abs() < 1e-9,
            "three independent actors aggregate to 3.0, got {aggregate}"
        );
        let _ = did;
    }

    #[test]
    fn promote_without_a_bound_actor_fails_closed() {
        let (storage, _dir) = test_storage();
        let node = ingest_node(&storage, "no actor bound");
        let err = storage
            .promote_memory_as_actor(&node.id, None, "promote_memory")
            .unwrap_err();
        assert!(err.to_string().contains("no process actor"));
        // Nothing changed.
        let after = storage.get_node(&node.id).unwrap().unwrap();
        assert_eq!(after.retrieval_strength, node.retrieval_strength);
    }

    #[test]
    fn demote_records_opposition_with_the_same_transactional_contract() {
        let (storage, _dir) = test_storage();
        bind_actor(&storage);
        let node = ingest_node(&storage, "demotion target");
        let outcome = storage
            .demote_memory_as_actor(&node.id, None, "memory")
            .expect("demote");
        assert_eq!(outcome.endorsement.endorsement_kind, "oppose");
        assert_eq!(outcome.receipt.mutations[0].kind, "demoted");
        assert!(
            outcome.node.retrieval_strength < outcome.before.retrieval_strength,
            "the demote mutation applied"
        );
        // Retried demotion: still one stance.
        let retry = storage
            .demote_memory_as_actor(&node.id, None, "memory")
            .expect("retry");
        assert!(retry.already_recorded);
    }

    #[test]
    fn reinforce_endorsement_uses_the_reinforce_strength_path() {
        let (storage, _dir) = test_storage();
        bind_actor(&storage);
        let node = ingest_node(&storage, "reinforce target");
        storage.demote_memory(&node.id).expect("pre-demote");
        let weakened = storage.get_node(&node.id).unwrap().unwrap();
        let outcome = storage
            .record_reinforce_endorsement(&node.id, None, "smart_ingest")
            .expect("reinforce");
        assert_eq!(outcome.endorsement.tool, "smart_ingest");
        assert_eq!(outcome.receipt.mutations[0].kind, "reinforced");
        // +0.05 retrieval matches strengthen_on_access, not the +0.20 promote.
        assert!(
            (outcome.node.retrieval_strength - weakened.retrieval_strength - 0.05).abs() < 1e-9
        );
    }
}
