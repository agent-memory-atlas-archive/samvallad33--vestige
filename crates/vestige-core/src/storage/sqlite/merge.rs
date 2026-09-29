//! Merge storage operations, extracted from the integrated v3 implementation.

use super::*;

impl SqliteMemoryStore {
    // ========================================================================
    // Merge / Supersede controls (Phase 3 — v2.1.25)
    //
    // Diff-previewed, confidence-gated, reversible, self-explaining
    // combine/dedupe/supersede on a never-delete (bitemporal) store.
    // Pure scoring/plan/op types live in `advanced::merge_supersede`.
    // ========================================================================

    /// Mark a memory protected (pinned) or unprotected. A protected memory can
    /// never be auto-merged, superseded, or garbage-collected.
    pub fn set_protected(&self, id: &str, protected: bool) -> Result<()> {
        let writer = self
            .writer
            .lock()
            .map_err(|_| StorageError::Init("Writer lock poisoned".into()))?;
        let affected = writer.execute(
            "UPDATE knowledge_nodes SET protected = ?1 WHERE id = ?2",
            params![if protected { 1 } else { 0 }, id],
        )?;
        if affected == 0 {
            return Err(StorageError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Is this memory protected (pinned)?
    pub fn is_protected(&self, id: &str) -> Result<bool> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        let v: Option<i64> = reader
            .query_row(
                "SELECT protected FROM knowledge_nodes WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        match v {
            Some(p) => Ok(p != 0),
            None => Err(StorageError::NotFound(id.to_string())),
        }
    }

    /// Read the per-project merge policy (two Fellegi-Sunter thresholds +
    /// auto_apply). Persisted in `fsrs_config` so it survives restarts without a
    /// new table; falls back to defaults (env-overridable) when unset.
    pub fn get_merge_policy(&self) -> Result<crate::advanced::MergePolicy> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        let read_key = |key: &str| -> Option<f64> {
            reader
                .query_row(
                    "SELECT value FROM fsrs_config WHERE key = ?1",
                    params![key],
                    |row| row.get::<_, f64>(0),
                )
                .optional()
                .ok()
                .flatten()
        };
        let default = crate::advanced::MergePolicy::default();
        let env_f32 = |name: &str, fallback: f32| -> f32 {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(fallback)
        };
        let match_threshold = read_key("merge_match_threshold")
            .map(|v| v as f32)
            .unwrap_or_else(|| env_f32("VESTIGE_MERGE_MATCH_THRESHOLD", default.match_threshold));
        let possible_threshold = read_key("merge_possible_threshold")
            .map(|v| v as f32)
            .unwrap_or_else(|| {
                env_f32(
                    "VESTIGE_MERGE_POSSIBLE_THRESHOLD",
                    default.possible_threshold,
                )
            });
        let auto_apply = match read_key("merge_auto_apply") {
            Some(v) => v != 0.0,
            None => std::env::var("VESTIGE_MERGE_AUTO_APPLY")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(default.auto_apply),
        };
        Ok(crate::advanced::MergePolicy::new(
            match_threshold,
            possible_threshold,
            auto_apply,
        ))
    }

    /// Persist the per-project merge policy into `fsrs_config`.
    pub fn set_merge_policy(&self, policy: crate::advanced::MergePolicy) -> Result<()> {
        let writer = self
            .writer
            .lock()
            .map_err(|_| StorageError::Init("Writer lock poisoned".into()))?;
        let now = Utc::now().to_rfc3339();
        let put = |key: &str, value: f64| -> Result<()> {
            writer.execute(
                "INSERT OR REPLACE INTO fsrs_config (key, value, updated_at) VALUES (?1, ?2, ?3)",
                params![key, value, now],
            )?;
            Ok(())
        };
        put("merge_match_threshold", policy.match_threshold as f64)?;
        put("merge_possible_threshold", policy.possible_threshold as f64)?;
        put(
            "merge_auto_apply",
            if policy.auto_apply { 1.0 } else { 0.0 },
        )?;
        Ok(())
    }

    /// Surface duplicate/overlapping memory clusters with confidence
    /// scores and the signals behind each (Fellegi-Sunter classified).
    ///
    /// NOMINATION IS EXACT EQUALITY ONLY (owner decision 2026-09-28: no
    /// similarity anywhere in dedup/merge nomination). The former O(n²)
    /// embedding-cosine candidate scan is deleted. A pair of memories is
    /// nominated when any of these exact equalities holds:
    ///
    /// 1. **identical content hash** — the stored envelope `content_hash`
    ///    (SQL group-by on `COALESCE(content_hash, content)`; nodes without a
    ///    recorded hash use their byte-identical content as the identity);
    /// 2. **the same declared source key** `(source_system, source_id)` (SQL
    ///    group-by): the same upstream record ingested twice. Current schemas
    ///    enforce a UNIQUE index on the key, so this nominator mainly
    ///    catches stores written before that constraint existed;
    /// 3. **exactly equal non-empty entity sets** —
    ///    `advanced::retroactive_backfill::extract_entities`, compared as sets
    ///    in memory.
    ///
    /// Tag/token overlap (`advanced::score_pair`) NEVER nominates; it only
    /// orders the nominated clusters and labels them for review. A cluster
    /// nominated by a shared source key but with diverged contents is
    /// intentionally still surfaced (labelled `Possible`/`NonMatch`) instead
    /// of dropped — a repeated declared source is review-worthy on its own,
    /// and the label tells the reviewer how weak the lexical evidence is.
    ///
    /// Protected members are flagged so the caller never auto-merges a pin.
    pub fn merge_candidates(
        &self,
        policy: crate::advanced::MergePolicy,
        limit: usize,
        tag_filter: &[String],
    ) -> Result<Vec<crate::advanced::MergeCandidate>> {
        use crate::advanced::{MergeCandidate, score_pair};
        use std::collections::{BTreeSet, HashMap, HashSet};

        let superseded: HashSet<String> = self.superseded_node_ids()?;
        let protected: HashSet<String> = self.protected_node_ids()?;

        // Load nodes for metadata. Exclude already-superseded nodes — they are
        // historical and must not be re-offered for merge — and apply the
        // caller's tag filter.
        let mut nodes: Vec<KnowledgeNode> = Vec::new();
        let mut offset = 0;
        loop {
            let batch = self.get_all_nodes(500, offset)?;
            let n = batch.len();
            nodes.extend(batch);
            if n < 500 {
                break;
            }
            offset += 500;
        }
        let nodes: Vec<KnowledgeNode> = nodes
            .into_iter()
            .filter(|node| !superseded.contains(&node.id))
            .filter(|node| tag_filter.is_empty() || tag_filter.iter().any(|t| node.tags.contains(t)))
            .collect();
        if nodes.len() < 2 {
            return Ok(vec![]);
        }
        let index_of: HashMap<&str, usize> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect();

        let n = nodes.len();
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(parent: &mut [usize], x: usize) -> usize {
            let mut root = x;
            while parent[root] != root {
                root = parent[root];
            }
            let mut cur = x;
            while parent[cur] != root {
                let next = parent[cur];
                parent[cur] = root;
                cur = next;
            }
            root
        }
        let union = |parent: &mut Vec<usize>, a: usize, b: usize| {
            let ra = find(parent, a);
            let rb = find(parent, b);
            if ra != rb {
                parent[ra] = rb;
            }
        };

        // Nominator 1 (SQL group-by): identical content identity — the stored
        // envelope hash when present, else the exact content itself.
        {
            let reader = self
                .reader
                .lock()
                .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
            let mut stmt = reader.prepare(
                "SELECT COALESCE(content_hash, content) AS identity_key, id
                 FROM knowledge_nodes
                 WHERE superseded_by IS NULL AND COALESCE(content_hash, content) IS NOT NULL
                 ORDER BY identity_key",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
            for row in rows {
                let (key, id) = row?;
                if let Some(&idx) = index_of.get(id.as_str()) {
                    groups.entry(key).or_default().push(idx);
                }
            }
            for members in groups.into_values() {
                for pair in members.windows(2) {
                    union(&mut parent, pair[0], pair[1]);
                }
            }
        }

        // Nominator 2 (SQL group-by): the same declared source key, at the
        // same granularity the store's own UNIQUE index uses
        // (system, project, id) so two projects' "issue 42" stay separate.
        {
            let reader = self
                .reader
                .lock()
                .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
            let mut stmt = reader.prepare(
                "SELECT source_system || ':' || COALESCE(source_project, '') || ':' || source_id AS source_key, id
                 FROM knowledge_nodes
                 WHERE superseded_by IS NULL
                   AND source_system IS NOT NULL AND source_id IS NOT NULL
                 ORDER BY source_key",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
            for row in rows {
                let (key, id) = row?;
                if let Some(&idx) = index_of.get(id.as_str()) {
                    groups.entry(key).or_default().push(idx);
                }
            }
            for members in groups.into_values() {
                for pair in members.windows(2) {
                    union(&mut parent, pair[0], pair[1]);
                }
            }
        }

        // Nominator 3 (in-memory set compare): exactly equal, non-empty
        // extracted-entity sets.
        {
            let mut groups: HashMap<BTreeSet<String>, Vec<usize>> = HashMap::new();
            for (i, node) in nodes.iter().enumerate() {
                let entities: BTreeSet<String> =
                    crate::advanced::retroactive_backfill::extract_entities(
                        &node.content,
                        &node.tags,
                    )
                    .into_iter()
                    .collect();
                if entities.is_empty() {
                    continue;
                }
                groups.entry(entities).or_default().push(i);
            }
            for members in groups.into_values() {
                for pair in members.windows(2) {
                    union(&mut parent, pair[0], pair[1]);
                }
            }
        }

        // Group indices by root.
        let mut clusters: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in 0..n {
            let r = find(&mut parent, i);
            clusters.entry(r).or_default().push(i);
        }

        let mut out: Vec<MergeCandidate> = Vec::new();
        for members in clusters.into_values() {
            if members.len() < 2 {
                continue;
            }
            // Cluster confidence = weakest pairwise lexical score (the loosest
            // link); the best-scoring pair's signals are the explanation.
            let mut min_score = 1.0f32;
            let mut best_signals: Option<crate::advanced::MatchSignals> = None;
            for a in 0..members.len() {
                for b in (a + 1)..members.len() {
                    let (na, nb) = (&nodes[members[a]], &nodes[members[b]]);
                    let sig = score_pair(&na.tags, &nb.tags, &na.content, &nb.content);
                    if sig.combined_score < min_score {
                        min_score = sig.combined_score;
                    }
                    if best_signals
                        .as_ref()
                        .map(|s| sig.combined_score > s.combined_score)
                        .unwrap_or(true)
                    {
                        best_signals = Some(sig);
                    }
                }
            }
            let signals = match best_signals {
                Some(s) => s,
                None => continue,
            };

            // Survivor = highest retention member.
            let mut ranked: Vec<usize> = members.clone();
            ranked.sort_by(|a, b| {
                let ra = nodes[*a].retention_strength;
                let rb = nodes[*b].retention_strength;
                rb.partial_cmp(&ra).unwrap_or(std::cmp::Ordering::Equal)
            });
            let member_ids: Vec<String> = ranked.iter().map(|&idx| nodes[idx].id.clone()).collect();
            let survivor_id = member_ids[0].clone();
            let has_protected_member = member_ids.iter().any(|id| protected.contains(id));
            let previews: Vec<String> = ranked
                .iter()
                .map(|&idx| preview(&nodes[idx].content, 120))
                .collect();

            // Advisory label only. Nomination came from exact equality above,
            // so a low lexical score surfaces the cluster for review rather
            // than dropping it (the old cosine scan dropped NonMatch pairs
            // because its nominations were probabilistic; these are not).
            let classification = policy.classify(min_score);

            out.push(MergeCandidate {
                member_ids,
                previews,
                survivor_id,
                confidence: min_score,
                classification,
                signals,
                has_protected_member,
            });
        }

        out.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out.truncate(limit);
        Ok(out)
    }

    /// IDs of nodes that have been bitemporally superseded (kept, but invalid).
    pub fn superseded_node_ids(&self) -> Result<std::collections::HashSet<String>> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        let mut stmt =
            reader.prepare("SELECT id FROM knowledge_nodes WHERE superseded_by IS NOT NULL")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut set = std::collections::HashSet::new();
        for r in rows {
            set.insert(r?);
        }
        Ok(set)
    }

    /// (superseded_id, superseding_id) pairs, so a trail can follow the link
    /// to the current belief instead of stopping at the invalidated record.
    pub fn supersession_pairs(&self) -> Result<Vec<(String, String)>> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        let mut stmt = reader
            .prepare("SELECT id, superseded_by FROM knowledge_nodes WHERE superseded_by IS NOT NULL")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// IDs of protected (pinned) nodes.
    pub fn protected_node_ids(&self) -> Result<std::collections::HashSet<String>> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        let mut stmt = reader.prepare("SELECT id FROM knowledge_nodes WHERE protected = 1")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut set = std::collections::HashSet::new();
        for r in rows {
            set.insert(r?);
        }
        Ok(set)
    }

    /// Build a previewable MERGE plan (a diff) WITHOUT applying it.
    ///
    /// The survivor is the first id (or highest retention if unspecified). The
    /// plan is persisted to `merge_plans` with status `pending` and returned for
    /// inspection. Nothing about the nodes changes until `apply_plan`.
    #[cfg(all(feature = "embeddings", feature = "vector-search"))]
    pub fn plan_merge(
        &self,
        member_ids: &[String],
        survivor_id: Option<&str>,
        policy: crate::advanced::MergePolicy,
    ) -> Result<crate::advanced::MergePlan> {
        use crate::advanced::{
            MatchClass, PlanKind, compose_merged_content, compose_merged_tags, score_pair,
        };

        if member_ids.len() < 2 {
            return Err(StorageError::Init(
                "plan_merge needs at least 2 member ids".into(),
            ));
        }

        if member_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            != member_ids.len()
        {
            return Err(StorageError::Init("merge members must be distinct".into()));
        }
        let expected_state = self.merge_state_snapshot(member_ids)?;
        let mut nodes: Vec<KnowledgeNode> = Vec::new();
        for id in member_ids {
            let node = self
                .get_node(id)?
                .ok_or_else(|| StorageError::NotFound(id.clone()))?;
            nodes.push(node);
        }

        // Protected nodes can never be absorbed. They may only be the survivor.
        let survivor = match survivor_id {
            Some(s) => s.to_string(),
            None => {
                // highest retention
                nodes
                    .iter()
                    .max_by(|a, b| {
                        a.retention_strength
                            .partial_cmp(&b.retention_strength)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|n| n.id.clone())
                    .unwrap_or_else(|| member_ids[0].clone())
            }
        };
        // The survivor MUST be one of the members. A caller-supplied survivor_id
        // that isn't in member_ids (a typo/mixup through the plan_merge tool)
        // otherwise sails through and panics at the `.find(...).unwrap()` below,
        // taking down the request. Reject it with a clear error instead.
        if !nodes.iter().any(|n| n.id == survivor) {
            return Err(StorageError::Init(format!(
                "survivor_id {survivor} is not among the member_ids being merged"
            )));
        }

        for node in &nodes {
            if node.id != survivor && self.is_protected(&node.id)? {
                return Err(StorageError::Init(format!(
                    "Memory {} is protected and cannot be merged away. Unprotect it first or make it the survivor.",
                    node.id
                )));
            }
        }

        // Order: survivor first, then others.
        nodes.sort_by_key(|n| if n.id == survivor { 0 } else { 1 });

        let members: Vec<(String, String)> = nodes
            .iter()
            .map(|n| (n.id.clone(), n.content.clone()))
            .collect();
        let result_content = compose_merged_content(&members);
        let result_tags =
            compose_merged_tags(&nodes.iter().map(|n| n.tags.clone()).collect::<Vec<_>>());
        let result_source = nodes
            .iter()
            .find(|n| n.id == survivor)
            .and_then(|n| n.source.clone());
        let invalidated_ids: Vec<String> = nodes
            .iter()
            .filter(|n| n.id != survivor)
            .map(|n| n.id.clone())
            .collect();

        // Confidence = weakest pair survivor↔absorbed (lexical tie-breaker
        // score; nomination is exact equality in merge_candidates).
        let survivor_node = nodes.iter().find(|n| n.id == survivor).unwrap();
        let mut min_score = 1.0f32;
        let mut best_signals = score_pair(
            &survivor_node.tags,
            &survivor_node.tags,
            &survivor_node.content,
            &survivor_node.content,
        );
        for node in nodes.iter().filter(|n| n.id != survivor) {
            let sig = score_pair(
                &survivor_node.tags,
                &node.tags,
                &survivor_node.content,
                &node.content,
            );
            if sig.combined_score < min_score {
                min_score = sig.combined_score;
                best_signals = sig;
            }
        }
        let classification = policy.classify(min_score);

        let plan = crate::advanced::MergePlan {
            expected_state,
            id: uuid::Uuid::new_v4().to_string(),
            kind: PlanKind::Merge,
            survivor_id: survivor.clone(),
            member_ids: member_ids.to_vec(),
            result_content,
            result_tags,
            result_source,
            invalidated_ids,
            confidence: min_score,
            classification,
            signals: best_signals,
            explanation: format!(
                "Merge {} memories into {survivor} ({}). {} memory(ies) will be bitemporally invalidated (kept for audit, marked superseded_by={survivor}).",
                member_ids.len(),
                match classification {
                    MatchClass::Match => "strong duplicate",
                    MatchClass::Possible => "possible duplicate — review advised",
                    MatchClass::NonMatch => "weak match — review strongly advised",
                },
                member_ids.len() - 1
            ),
            reconsolidation: None,
        };

        self.persist_plan(&plan)?;
        Ok(plan)
    }

    /// Build a previewable SUPERSEDE plan: invalidate `old_id` in favour of
    /// `new_id` (bitemporal, audit-preserving) WITHOUT applying it.
    #[cfg(all(feature = "embeddings", feature = "vector-search"))]
    pub fn plan_supersede(
        &self,
        old_id: &str,
        new_id: &str,
        policy: crate::advanced::MergePolicy,
    ) -> Result<crate::advanced::MergePlan> {
        use crate::advanced::{PlanKind, score_pair};

        if old_id == new_id {
            return Err(StorageError::Init(
                "supersede members must be distinct".into(),
            ));
        }
        let expected_state =
            self.merge_state_snapshot(&[old_id.to_string(), new_id.to_string()])?;
        let old = self
            .get_node(old_id)?
            .ok_or_else(|| StorageError::NotFound(old_id.to_string()))?;
        let new = self
            .get_node(new_id)?
            .ok_or_else(|| StorageError::NotFound(new_id.to_string()))?;

        if self.is_protected(old_id)? {
            return Err(StorageError::Init(format!(
                "Memory {old_id} is protected and cannot be superseded. Unprotect it first."
            )));
        }

        let signals = score_pair(&old.tags, &new.tags, &old.content, &new.content);
        let classification = policy.classify(signals.combined_score);

        let plan = crate::advanced::MergePlan {
            expected_state,
            id: uuid::Uuid::new_v4().to_string(),
            kind: PlanKind::Supersede,
            survivor_id: new_id.to_string(),
            member_ids: vec![old_id.to_string(), new_id.to_string()],
            result_content: new.content.clone(),
            result_tags: new.tags.clone(),
            result_source: new.source.clone(),
            invalidated_ids: vec![old_id.to_string()],
            confidence: signals.combined_score,
            classification,
            signals,
            explanation: format!(
                "Supersede {old_id} with {new_id}. {old_id} is kept and remains queryable for audit, but stamped valid_until=now and superseded_by={new_id} (invalidate, don't delete)."
            ),
            reconsolidation: None,
        };

        self.persist_plan(&plan)?;
        Ok(plan)
    }

    /// Build a previewable RECONSOLIDATION plan: a conflict or supersede
    /// arrived while `target_id` was inside its labile window, so the rewrite
    /// is deferred behind an explicit verdict instead of applying immediately.
    ///
    /// - **approve** → `apply_plan` invalidates the target bitemporally in
    ///   favour of the incoming memory (snapshot-based rollback via
    ///   `merge_undo`).
    /// - **reject** → the plan is discarded; the target memory stays exactly
    ///   as its `mark_labile` snapshot captured it.
    /// - **quarantine** → the target is suppressed (top-down inhibition) and
    ///   the plan closes.
    ///
    /// Classification is always `Possible`: a conflict with a live memory is
    /// a review case by construction, never an auto-apply, regardless of
    /// match score. See [`super::reconsolidation`] for the neuroscience.
    #[cfg(all(feature = "embeddings", feature = "vector-search"))]
    pub fn plan_reconsolidation(
        &self,
        target_id: &str,
        incoming_id: &str,
        labile: &crate::advanced::reconsolidation::LabileCandidate,
        trigger: &str,
    ) -> Result<crate::advanced::MergePlan> {
        use crate::advanced::{MatchClass, PlanKind, ReconsolidationMeta, score_pair};

        if target_id == incoming_id {
            return Err(StorageError::Init(
                "reconsolidation members must be distinct".into(),
            ));
        }
        if labile.memory_id != target_id {
            return Err(StorageError::Init(
                "labile candidate does not match the reconsolidation target".into(),
            ));
        }
        if labile.window_expires_at <= Utc::now() {
            return Err(StorageError::Init(
                "labile window already expired; conflict is no longer routed through reconsolidation"
                    .into(),
            ));
        }
        let expected_state =
            self.merge_state_snapshot(&[target_id.to_string(), incoming_id.to_string()])?;
        let target = self
            .get_node(target_id)?
            .ok_or_else(|| StorageError::NotFound(target_id.to_string()))?;
        let incoming = self
            .get_node(incoming_id)?
            .ok_or_else(|| StorageError::NotFound(incoming_id.to_string()))?;

        if self.is_protected(target_id)? {
            return Err(StorageError::Init(format!(
                "Memory {target_id} is protected and cannot be superseded. Unprotect it first."
            )));
        }

        let signals = score_pair(
            &target.tags,
            &incoming.tags,
            &target.content,
            &incoming.content,
        );
        // Review-first always: `Possible` forces confirm=true in apply_plan,
        // whatever the configured match thresholds say.
        let classification = MatchClass::Possible;

        let plan = crate::advanced::MergePlan {
            expected_state,
            id: uuid::Uuid::new_v4().to_string(),
            kind: PlanKind::Reconsolidation,
            survivor_id: incoming.id.clone(),
            member_ids: vec![target_id.to_string(), incoming_id.to_string()],
            result_content: incoming.content.clone(),
            result_tags: incoming.tags.clone(),
            result_source: incoming.source.clone(),
            invalidated_ids: vec![target_id.to_string()],
            confidence: signals.combined_score,
            classification,
            signals,
            explanation: format!(
                "Reconsolidation ({trigger}): incoming memory {incoming_id} conflicts with {target_id} while its labile window is open. Approve supersedes {target_id} (kept for audit, rollback via merge_undo), reject keeps {target_id} unchanged, quarantine suppresses {target_id}. Window closes {}.",
                labile.window_expires_at.to_rfc3339()
            ),
            reconsolidation: Some(ReconsolidationMeta {
                target_memory_id: target_id.to_string(),
                snapshot: labile.snapshot.clone(),
                window_expires_at: labile.window_expires_at,
                trigger: trigger.to_string(),
            }),
        };

        self.persist_plan(&plan)?;
        Ok(plan)
    }

    /// Persist a plan row (status pending). Idempotent on plan id.
    #[cfg(all(feature = "embeddings", feature = "vector-search"))]
    pub(super) fn persist_plan(&self, plan: &crate::advanced::MergePlan) -> Result<()> {
        let writer = self
            .writer
            .lock()
            .map_err(|_| StorageError::Init("Writer lock poisoned".into()))?;
        let payload = serde_json::to_string(plan)
            .map_err(|e| StorageError::Init(format!("plan serialize failed: {e}")))?;
        let member_ids = serde_json::to_string(&plan.member_ids).unwrap_or_else(|_| "[]".into());
        writer.execute(
            "INSERT OR REPLACE INTO merge_plans
                (id, kind, status, created_at, applied_at, survivor_id, member_ids, confidence, classification, payload)
             VALUES (?1, ?2, 'pending', ?3, NULL, ?4, ?5, ?6, ?7, ?8)",
            params![
                plan.id,
                plan.kind.as_str(),
                Utc::now().to_rfc3339(),
                plan.survivor_id,
                member_ids,
                plan.confidence as f64,
                plan.classification.as_str(),
                payload,
            ],
        )?;
        Ok(())
    }

    /// Fetch a stored plan by id.
    pub fn get_plan(&self, plan_id: &str) -> Result<Option<crate::advanced::MergePlan>> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        let row: Option<(String, String)> = reader
            .query_row(
                "SELECT status, payload FROM merge_plans WHERE id = ?1",
                params![plan_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        match row {
            Some((_status, payload)) => {
                let plan: crate::advanced::MergePlan = serde_json::from_str(&payload)
                    .map_err(|e| StorageError::Init(format!("plan deserialize failed: {e}")))?;
                Ok(Some(plan))
            }
            None => Ok(None),
        }
    }

    /// Plan status string (pending | applied | cancelled | rejected |
    /// quarantined | expired), if the plan exists.
    pub fn plan_status(&self, plan_id: &str) -> Result<Option<String>> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        let status: Option<String> = reader
            .query_row(
                "SELECT status FROM merge_plans WHERE id = ?1",
                params![plan_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(status)
    }

    /// Record a non-mutating reconsolidation verdict row (reject / quarantine
    /// marker / expiry) in `merge_operations` and set the plan status. These
    /// ops carry an empty undo payload: `merge_undo` refuses anything that is
    /// not `status='applied'`, which is correct — a rejection has nothing to
    /// reverse, and suppression reversal goes through the suppression path's
    /// own 24-hour labile undo, not the merge reflog.
    fn record_reconsolidation_verdict_op(
        &self,
        tx: &rusqlite::Transaction<'_>,
        plan: &crate::advanced::MergePlan,
        status: &str,
        reason: &str,
    ) -> Result<crate::advanced::MergeOperation> {
        let op_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();
        let affected = vec![plan.survivor_id.clone()];
        tx.execute(
            "INSERT INTO merge_operations
                (id, plan_id, op_type, status, created_at, reverted_at, reverts_op_id,
                 survivor_id, affected_ids, confidence, signals, reason, undo_payload)
             VALUES (?1, ?2, 'reconsolidation', ?3, ?4, NULL, NULL, ?5, ?6, ?7, NULL, ?8, '{}')",
            params![
                op_id,
                plan.id,
                status,
                now.to_rfc3339(),
                plan.survivor_id,
                serde_json::to_string(&affected).unwrap_or_else(|_| "[]".into()),
                plan.confidence as f64,
                reason,
            ],
        )?;
        tx.execute(
            "UPDATE merge_plans SET status = ?1, applied_at = ?2 WHERE id = ?3",
            params![status, now.to_rfc3339(), plan.id],
        )?;
        Ok(crate::advanced::MergeOperation {
            id: op_id,
            plan_id: Some(plan.id.clone()),
            op_type: "reconsolidation".to_string(),
            status: status.to_string(),
            created_at: now.to_rfc3339(),
            reverted_at: None,
            reverts_op_id: None,
            survivor_id: Some(plan.survivor_id.clone()),
            affected_ids: affected,
            confidence: Some(plan.confidence),
            signals: None,
            reason: Some(reason.to_string()),
        })
    }

    /// Auto-close one expired reconsolidation plan (status `expired`) and
    /// record the close. Used both by the opportunistic sweep and by the
    /// apply-time guard, so an expired plan can never sit pending or be
    /// applied after its labile window closed — no zombie plans.
    pub fn expire_reconsolidation_plan(
        &self,
        plan: &crate::advanced::MergePlan,
    ) -> Result<crate::advanced::MergeOperation> {
        let writer = self
            .writer
            .lock()
            .map_err(|_| StorageError::Init("Writer lock poisoned".into()))?;
        let tx = Self::begin_write_transaction(&writer, "expire_reconsolidation_plan")?;
        let op = self.record_reconsolidation_verdict_op(
            &tx,
            plan,
            "expired",
            "Labile window expired without a verdict; reconsolidation plan auto-closed",
        )?;
        tx.commit()?;
        Ok(op)
    }

    /// Close every pending reconsolidation plan whose labile window has
    /// expired. Returns the closed plan ids. Called from the consolidation
    /// cycle and before listing, so verdict surfaces never offer a stale
    /// conflict.
    pub fn expire_stale_reconsolidation_plans(&self) -> Result<Vec<String>> {
        let stale = self.pending_expired_reconsolidation_plans()?;
        let mut closed = Vec::with_capacity(stale.len());
        for plan in stale {
            self.expire_reconsolidation_plan(&plan)?;
            closed.push(plan.id);
        }
        Ok(closed)
    }

    /// Load pending reconsolidation plans past their window.
    fn pending_expired_reconsolidation_plans(&self) -> Result<Vec<crate::advanced::MergePlan>> {
        let cutoff = Utc::now().to_rfc3339();
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        let mut stmt = reader.prepare(
            "SELECT payload FROM merge_plans
             WHERE kind = 'reconsolidation' AND status = 'pending' AND created_at < ?1",
        )?;
        let mut rows = stmt.query(params![cutoff])?;
        let mut stale = Vec::new();
        while let Some(row) = rows.next()? {
            let payload: String = row.get(0)?;
            if let Ok(plan) = serde_json::from_str::<crate::advanced::MergePlan>(&payload)
                && let Some(meta) = &plan.reconsolidation
                && meta.window_expires_at <= Utc::now()
            {
                stale.push(plan);
            }
        }
        Ok(stale)
    }

    /// Pending reconsolidation plans with their verdict deadline, oldest
    /// first. Runs the expiry sweep first, so a caller never sees a plan
    /// whose window already closed.
    pub fn list_reconsolidation_plans(
        &self,
        limit: usize,
    ) -> Result<Vec<(crate::advanced::MergePlan, String)>> {
        self.expire_stale_reconsolidation_plans()?;
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        let mut stmt = reader.prepare(
            "SELECT payload, status FROM merge_plans
             WHERE kind = 'reconsolidation' AND status = 'pending'
             ORDER BY created_at ASC LIMIT ?1",
        )?;
        let mut rows = stmt.query(params![limit as i64])?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let payload: String = row.get(0)?;
            let status: String = row.get(1)?;
            if let Ok(plan) = serde_json::from_str::<crate::advanced::MergePlan>(&payload) {
                out.push((plan, status));
            }
        }
        Ok(out)
    }

    /// Apply an explicit verdict to a reconsolidation plan.
    ///
    /// - `approve` → applies the plan (bitemporal invalidation of the labile
    ///   target in favour of the incoming memory); reversible through
    ///   `merge_undo`. `confirm` is implied: reaching the verdict IS the
    ///   confirmation, and the plan's `Possible` classification would
    ///   otherwise demand a redundant flag.
    /// - `reject` → the plan is discarded (`rejected`); the target memory
    ///   stays exactly as its `mark_labile` snapshot captured it.
    /// - `quarantine` → the target memory is suppressed (top-down inhibition,
    ///   `suppress_memory`) and the plan closes (`quarantined`).
    ///
    /// Expired plans are refused and auto-closed. Verdict on anything that is
    /// not a pending reconsolidation plan is an error.
    #[cfg(all(feature = "embeddings", feature = "vector-search"))]
    pub fn verdict_reconsolidation_plan(
        &self,
        plan_id: &str,
        verdict: &str,
        reason: Option<&str>,
    ) -> Result<crate::advanced::MergeOperation> {
        let plan = self
            .get_plan(plan_id)?
            .ok_or_else(|| StorageError::NotFound(format!("plan {plan_id}")))?;
        if plan.kind != crate::advanced::PlanKind::Reconsolidation {
            return Err(StorageError::Init(format!(
                "plan {plan_id} is not a reconsolidation plan"
            )));
        }
        match self.plan_status(plan_id)?.as_deref() {
            Some("pending") => {}
            other => {
                return Err(StorageError::Init(format!(
                    "plan {plan_id} is not pending (status: {})",
                    other.unwrap_or("missing")
                )));
            }
        }
        if let Some(meta) = &plan.reconsolidation
            && meta.window_expires_at <= Utc::now()
        {
            self.expire_reconsolidation_plan(&plan)?;
            return Err(StorageError::Init(
                "plan expired with its labile window and was auto-closed".to_string(),
            ));
        }
        let note = reason.unwrap_or("no reason given");
        match verdict {
            "approve" => self.apply_plan(plan_id, true),
            "reject" => {
                let writer = self
                    .writer
                    .lock()
                    .map_err(|_| StorageError::Init("Writer lock poisoned".into()))?;
                let tx = Self::begin_write_transaction(&writer, "reject_reconsolidation_plan")?;
                let op = self.record_reconsolidation_verdict_op(
                    &tx,
                    &plan,
                    "rejected",
                    &format!("Reconsolidation rejected; target memory unchanged ({note})"),
                )?;
                tx.commit()?;
                Ok(op)
            }
            "quarantine" => {
                let target = plan
                    .reconsolidation
                    .as_ref()
                    .map(|meta| meta.target_memory_id.clone())
                    .unwrap_or_else(|| plan.invalidated_ids[0].clone());
                // Top-down suppression via the existing suppress path. This
                // is a real inhibition (count, timestamp, before/after log),
                // not a delete; the MCP layer closes the in-memory labile
                // window alongside.
                self.suppress_memory(&target)?;
                let writer = self
                    .writer
                    .lock()
                    .map_err(|_| StorageError::Init("Writer lock poisoned".into()))?;
                let tx =
                    Self::begin_write_transaction(&writer, "quarantine_reconsolidation_plan")?;
                let op = self.record_reconsolidation_verdict_op(
                    &tx,
                    &plan,
                    "quarantined",
                    &format!("Reconsolidation quarantined; {target} suppressed ({note})"),
                )?;
                tx.commit()?;
                Ok(op)
            }
            other => Err(StorageError::Init(format!(
                "unknown verdict '{other}'; use approve | reject | quarantine"
            ))),
        }
    }

    /// Execute a previously-generated plan by id. Everything it does is recorded
    /// as a reversible [`MergeOperation`] in `merge_operations`. Returns the
    /// recorded operation id.
    ///
    /// - **merge**: survivor content/tags are rewritten to the merged result;
    ///   each absorbed node is bitemporally invalidated (valid_until=now,
    ///   superseded_by=survivor) and kept queryable.
    /// - **supersede**: old node is bitemporally invalidated in favour of new.
    ///
    /// `auto_apply` must be true in the policy to apply a `Match` plan without an
    /// explicit `confirm`; non-`Match` plans always require `confirm=true`.
    #[cfg(all(feature = "embeddings", feature = "vector-search"))]
    pub fn apply_plan(
        &self,
        plan_id: &str,
        confirm: bool,
    ) -> Result<crate::advanced::MergeOperation> {
        use crate::advanced::{MatchClass, PlanKind};

        let plan = self
            .get_plan(plan_id)?
            .ok_or_else(|| StorageError::NotFound(format!("plan {plan_id}")))?;

        // Reconsolidation plans are verdicts on a labile window: past the
        // window they are auto-closed (recorded as `expired`) and refused —
        // approving a stale conflict would rewrite a memory that already
        // reconsolidated.
        if plan.kind == PlanKind::Reconsolidation
            && let Some(meta) = &plan.reconsolidation
            && meta.window_expires_at <= Utc::now()
        {
            self.expire_reconsolidation_plan(&plan)?;
            return Err(StorageError::Init(format!(
                "plan {plan_id} expired with its labile window and was auto-closed"
            )));
        }

        match self.plan_status(plan_id)?.as_deref() {
            Some("applied") => {
                return Err(StorageError::Init(format!(
                    "plan {plan_id} was already applied"
                )));
            }
            Some("cancelled") => {
                return Err(StorageError::Init(format!("plan {plan_id} was cancelled")));
            }
            _ => {}
        }

        let now = Utc::now();
        let op_id = uuid::Uuid::new_v4().to_string();

        // The whole apply is ONE IMMEDIATE transaction, and the undo row is
        // written FIRST inside it.
        //
        // The old shape mutated through helpers that each committed on their
        // own, and only afterwards inserted the reflog row. Any failure between
        // the survivor rewrite and that insert left the survivor's content
        // overwritten with NO undo row at all: unrecoverable. SQLITE_BUSY was
        // the realistic trigger, because several MCP server processes share one
        // database file. The plan-status check was not atomic with the
        // mutations either, so two processes could both pass it and both apply.
        //
        // Now: the status re-check, the undo row, the plan transition, the
        // survivor rewrite and the invalidations either all commit or none do,
        // and the status re-check inside the transaction closes the race.
        //
        // The embedding is deliberately NOT regenerated in here. That is model
        // inference behind a tokio runtime, and holding the write lock across
        // it is the same defect as holding a mutex across a model load. The
        // transaction marks the survivor `has_embedding = 0` instead, and the
        // regeneration runs after COMMIT. If it fails, or the process dies
        // first, the node is already flagged and the consolidation cycle's
        // `generate_missing_embeddings` rebuilds it. Stale-but-flagged is
        // recoverable; overwritten-with-no-undo-row is not.
        let mut content_changed = false;
        {
            let writer = self
                .writer
                .lock()
                .map_err(|_| StorageError::Init("Writer lock poisoned".into()))?;
            let tx = Self::begin_write_transaction(&writer, "apply_plan")?;

            // Re-check status INSIDE the transaction. Two processes can both
            // pass the read above; only one can pass this.
            let status: Option<String> = tx
                .query_row(
                    "SELECT status FROM merge_plans WHERE id = ?1",
                    params![plan_id],
                    |row| row.get(0),
                )
                .optional()?;
            match status.as_deref() {
                Some("applied") => {
                    return Err(StorageError::Init(format!(
                        "plan {plan_id} was already applied"
                    )));
                }
                Some("cancelled") => {
                    return Err(StorageError::Init(format!("plan {plan_id} was cancelled")));
                }
                _ => {}
            }

            // Re-read policy in the same transaction as the mutation. A plan's
            // classification alone never grants unattended write permission.
            let auto_apply: Option<f64> = tx
                .query_row(
                    "SELECT value FROM fsrs_config WHERE key = 'merge_auto_apply'",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            let auto_apply = auto_apply.map(|v| v != 0.0).unwrap_or_else(|| {
                std::env::var("VESTIGE_MERGE_AUTO_APPLY")
                    .ok()
                    .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            });
            if !confirm && (plan.classification != MatchClass::Match || !auto_apply) {
                return Err(StorageError::Init(format!(
                    "plan {plan_id} requires confirm=true under the current merge policy"
                )));
            }
            // Check every affected identity at action time. A stored plan is
            // not authorization to combine independent project namespaces.
            let mut plan_scope: Option<String> = None;
            for id in std::iter::once(&plan.survivor_id)
                .chain(plan.member_ids.iter())
                .chain(plan.invalidated_ids.iter())
            {
                let scope: String = tx.query_row(
                    "SELECT COALESCE(NULLIF(trim(scope), ''), 'user') FROM knowledge_nodes WHERE id = ?1",
                    params![id], |row| row.get(0),
                )?;
                if plan_scope
                    .as_ref()
                    .is_some_and(|expected| expected != &scope)
                {
                    return Err(StorageError::Init(
                        "merge plan crosses project scopes; no changes applied".into(),
                    ));
                }
                plan_scope = Some(scope);
            }

            let actual = Self::merge_state_on(&tx, &plan.member_ids)?;
            if plan.expected_state.is_empty() || actual != plan.expected_state {
                return Err(StorageError::Init(
                    "merge plan is stale or lacks source-state fingerprints; create a new plan"
                        .into(),
                ));
            }
            for id in &plan.member_ids {
                let node = tx.query_row(
                    "SELECT * FROM knowledge_nodes WHERE id = ?1",
                    params![id],
                    Self::row_to_node,
                )?;
                if node.suppression_count > 0 || !node.is_currently_valid() {
                    return Err(StorageError::Init("merge plan contains suppressed or temporally inactive memory; no changes applied".into()));
                }
            }

            // Snapshot everything we need to undo, BEFORE mutating, and from
            // inside the same transaction so the snapshot and the mutation see
            // one consistent state.
            let mut undo = serde_json::Map::new();
            undo.insert("plan_id".into(), serde_json::json!(plan_id));
            undo.insert("kind".into(), serde_json::json!(plan.kind.as_str()));
            undo.insert("survivor_id".into(), serde_json::json!(plan.survivor_id));

            match plan.kind {
                PlanKind::Merge => {
                    let (prev_content, prev_tags_json): (String, String) = tx
                        .query_row(
                            "SELECT content, COALESCE(tags, '[]') FROM knowledge_nodes WHERE id = ?1",
                            params![plan.survivor_id],
                            |row| Ok((row.get(0)?, row.get(1)?)),
                        )
                        .optional()?
                        .ok_or_else(|| StorageError::NotFound(plan.survivor_id.clone()))?;
                    let prev_tags: Vec<String> =
                        serde_json::from_str(&prev_tags_json).unwrap_or_default();
                    undo.insert(
                        "survivor_prev_content".into(),
                        serde_json::json!(prev_content),
                    );
                    undo.insert("survivor_prev_tags".into(), serde_json::json!(prev_tags));

                    let mut absorbed = Vec::new();
                    for id in &plan.invalidated_ids {
                        let (vu, sb) = Self::read_bitemporal_in_transaction(&tx, id)?;
                        absorbed.push(serde_json::json!({
                            "id": id,
                            "prev_valid_until": vu,
                            "prev_superseded_by": sb,
                        }));
                    }
                    undo.insert("absorbed".into(), serde_json::json!(absorbed));
                    content_changed = prev_content != plan.result_content;
                }
                PlanKind::Supersede => {
                    let old_id = &plan.member_ids[0];
                    let (vu, sb) = Self::read_bitemporal_in_transaction(&tx, old_id)?;
                    undo.insert(
                        "absorbed".into(),
                        serde_json::json!([{
                            "id": old_id,
                            "prev_valid_until": vu,
                            "prev_superseded_by": sb,
                        }]),
                    );
                }
                PlanKind::Reconsolidation => {
                    // Same reversal shape as supersede: the approve verdict
                    // bitemporally invalidates the labile target; the undo
                    // payload restores its previous validity window. The
                    // mark_labile snapshot travels in the plan payload for
                    // reviewer diffing, while this payload is what makes the
                    // apply reversible.
                    let old_id = &plan.member_ids[0];
                    let (vu, sb) = Self::read_bitemporal_in_transaction(&tx, old_id)?;
                    undo.insert(
                        "absorbed".into(),
                        serde_json::json!([{
                            "id": old_id,
                            "prev_valid_until": vu,
                            "prev_superseded_by": sb,
                        }]),
                    );
                }
            }

            let affected: Vec<String> = {
                let mut v = vec![plan.survivor_id.clone()];
                v.extend(plan.invalidated_ids.clone());
                v
            };
            let signals = serde_json::to_string(&plan.signals).unwrap_or_else(|_| "{}".into());

            // The undo row goes in FIRST. Nothing below it can leave a mutation
            // without its reversal, because nothing below it commits alone.
            tx.execute(
                "INSERT INTO merge_operations
                    (id, plan_id, op_type, status, created_at, reverted_at, reverts_op_id,
                     survivor_id, affected_ids, confidence, signals, reason, undo_payload)
                 VALUES (?1, ?2, ?3, 'applied', ?4, NULL, NULL, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    op_id,
                    plan_id,
                    plan.kind.as_str(),
                    now.to_rfc3339(),
                    plan.survivor_id,
                    serde_json::to_string(&affected).unwrap_or_else(|_| "[]".into()),
                    plan.confidence as f64,
                    signals,
                    plan.explanation,
                    serde_json::Value::Object(undo.clone()).to_string(),
                ],
            )?;
            tx.execute(
                "UPDATE merge_plans SET status = 'applied', applied_at = ?1 WHERE id = ?2",
                params![now.to_rfc3339(), plan_id],
            )?;

            match plan.kind {
                PlanKind::Merge => {
                    let tags_json =
                        serde_json::to_string(&plan.result_tags).unwrap_or_else(|_| "[]".into());
                    tx.execute(
                        "UPDATE knowledge_nodes SET content = ?1, tags = ?2, updated_at = ?3
                         WHERE id = ?4",
                        params![
                            plan.result_content,
                            tags_json,
                            now.to_rfc3339(),
                            plan.survivor_id
                        ],
                    )?;
                    #[cfg(all(feature = "embeddings", feature = "vector-search"))]
                    if content_changed {
                        // Flag for rebuild before COMMIT, so a crash between
                        // here and the regeneration below is self-healing.
                        tx.execute(
                            "UPDATE knowledge_nodes SET has_embedding = 0 WHERE id = ?1",
                            params![plan.survivor_id],
                        )?;
                        tx.execute(
                            "DELETE FROM embedding_profile_vectors WHERE node_id = ?1",
                            params![plan.survivor_id],
                        )?;
                        tx.execute(
                            "DELETE FROM node_embeddings WHERE node_id = ?1",
                            params![plan.survivor_id],
                        )?;
                    }
                    for id in &plan.invalidated_ids {
                        Self::invalidate_node_in_transaction(&tx, id, &plan.survivor_id, now)?;
                    }
                }
                PlanKind::Supersede => {
                    let old_id = &plan.member_ids[0];
                    Self::invalidate_node_in_transaction(&tx, old_id, &plan.survivor_id, now)?;
                }
                PlanKind::Reconsolidation => {
                    // Approve verdict: invalidate the labile target exactly as
                    // a supersede would have — bitemporally, never deleted.
                    let old_id = &plan.member_ids[0];
                    Self::invalidate_node_in_transaction(&tx, old_id, &plan.survivor_id, now)?;
                }
            }

            let post_state = Self::merge_state_on(&tx, &plan.member_ids)?;
            undo.insert(
                "post_state".into(),
                serde_json::to_value(post_state)
                    .map_err(|error| StorageError::Init(error.to_string()))?,
            );
            tx.execute(
                "UPDATE merge_operations SET undo_payload = ?1 WHERE id = ?2",
                params![serde_json::Value::Object(undo).to_string(), op_id],
            )?;

            tx.commit()?;
        }

        // Committed. Regenerate the survivor's embedding outside the write
        // lock; `has_embedding = 0` is already persisted, so failure here is
        // recoverable by the next consolidation cycle rather than silent.
        #[cfg(all(feature = "embeddings", feature = "vector-search"))]
        if content_changed {
            if let Some(index) = self.vector_index.as_ref()
                && let Ok(mut index) = index.lock()
            {
                let _ = index.remove(&plan.survivor_id);
            }
            if self.active_embedding_runtime_ready().unwrap_or(false)
                && let Err(e) =
                    self.generate_embedding_for_node(&plan.survivor_id, &plan.result_content)
            {
                tracing::warn!(
                    survivor_id = %plan.survivor_id,
                    error = %e,
                    "apply_plan committed but could not regenerate the survivor embedding; \
                     it stays has_embedding=0 for the next consolidation sweep"
                );
            }
        }

        self.read_operation(&op_id)?
            .ok_or_else(|| StorageError::Init("operation vanished after insert".into()))
    }

    /// Reverse a prior merge/supersede operation by id (the "memory reflog").
    /// Restores survivor content/tags and clears the bitemporal invalidation on
    /// every node the operation touched, then records a compensating `undo` op.
    #[cfg(all(feature = "embeddings", feature = "vector-search"))]
    pub fn merge_undo(&self, op_id: &str) -> Result<crate::advanced::MergeOperation> {
        let op = self
            .read_operation(op_id)?
            .ok_or_else(|| StorageError::NotFound(format!("operation {op_id}")))?;
        if matches!(op.op_type.as_str(), "tag_rename" | "tag_merge") {
            return self.undo_tag_mutation(op_id);
        }
        if op.op_type == "undo" {
            return Err(StorageError::Init("cannot undo an undo operation".into()));
        }
        let now = Utc::now();
        let new_op_id = uuid::Uuid::new_v4().to_string();
        let mut regenerated = None;
        {
            let writer = self
                .writer
                .lock()
                .map_err(|_| StorageError::Init("Writer lock poisoned".into()))?;
            let tx = Self::begin_write_transaction(&writer, "merge_undo")?;
            let (status, payload): (String, String) = tx.query_row(
                "SELECT status, undo_payload FROM merge_operations WHERE id = ?1",
                params![op_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if status != "applied" {
                return Err(StorageError::Init(format!(
                    "operation {op_id} was already reverted or is not applied"
                )));
            }
            let undo: serde_json::Value = serde_json::from_str(&payload)
                .map_err(|error| StorageError::Init(format!("undo payload invalid: {error}")))?;
            let expected: std::collections::BTreeMap<String, String> = serde_json::from_value(
                undo.get("post_state").cloned().ok_or_else(|| StorageError::Init(
                    "legacy undo has no post-state fingerprints; manual recovery review required".into()))?)
                .map_err(|error| StorageError::Init(error.to_string()))?;
            if expected.is_empty() || Self::merge_state_on(&tx, &op.affected_ids)? != expected {
                return Err(StorageError::Init(
                    "undo conflicts with later memory changes; no changes applied".into(),
                ));
            }
            let survivor_id = undo
                .get("survivor_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| StorageError::Init("undo missing survivor".into()))?;
            let kind = undo.get("kind").and_then(|v| v.as_str());
            if kind == Some("merge") {
                let content = undo
                    .get("survivor_prev_content")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| StorageError::Init("undo missing survivor content".into()))?;
                let tags: Vec<String> = serde_json::from_value(
                    undo.get("survivor_prev_tags")
                        .cloned()
                        .ok_or_else(|| StorageError::Init("undo missing tags".into()))?,
                )
                .map_err(|error| StorageError::Init(error.to_string()))?;
                tx.execute(
                    "UPDATE knowledge_nodes SET content = ?1, tags = ?2,
                    updated_at = ?3, has_embedding = 0 WHERE id = ?4",
                    params![
                        content,
                        serde_json::to_string(&tags)
                            .map_err(|error| StorageError::Init(error.to_string()))?,
                        now.to_rfc3339(),
                        survivor_id
                    ],
                )?;
                tx.execute(
                    "DELETE FROM embedding_profile_vectors WHERE node_id = ?1",
                    params![survivor_id],
                )?;
                tx.execute(
                    "DELETE FROM node_embeddings WHERE node_id = ?1",
                    params![survivor_id],
                )?;
                regenerated = Some((survivor_id.to_string(), content.to_string()));
            } else if kind != Some("supersede") && kind != Some("reconsolidation") {
                return Err(StorageError::Init("unsupported undo kind".into()));
            }
            let absorbed = undo
                .get("absorbed")
                .and_then(|value| value.as_array())
                .ok_or_else(|| StorageError::Init("undo missing absorbed state".into()))?;
            for entry in absorbed {
                let id = entry
                    .get("id")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| StorageError::Init("undo missing absorbed identity".into()))?;
                let previous_until = entry
                    .get("prev_valid_until")
                    .and_then(|value| value.as_str());
                let previous_superseded = entry
                    .get("prev_superseded_by")
                    .and_then(|value| value.as_str());
                if tx.execute(
                    "UPDATE knowledge_nodes SET valid_until = ?1, superseded_by = ?2,
                    updated_at = ?3 WHERE id = ?4",
                    params![previous_until, previous_superseded, now.to_rfc3339(), id],
                )? != 1
                {
                    return Err(StorageError::NotFound(id.to_string()));
                }
            }
            tx.execute(
                "UPDATE merge_operations SET status = 'reverted', reverted_at = ?1 WHERE id = ?2",
                params![now.to_rfc3339(), op_id],
            )?;
            if let Some(plan_id) = op.plan_id.as_deref() {
                tx.execute(
                    "UPDATE merge_plans SET status = 'pending', applied_at = NULL WHERE id = ?1",
                    params![plan_id],
                )?;
            }
            tx.execute(
                "INSERT INTO merge_operations
                (id, plan_id, op_type, status, created_at, reverted_at, reverts_op_id,
                 survivor_id, affected_ids, confidence, signals, reason, undo_payload)
                 VALUES (?1, ?2, 'undo', 'applied', ?3, NULL, ?4, ?5, ?6, NULL, NULL, ?7, '{}')",
                params![
                    new_op_id,
                    op.plan_id,
                    now.to_rfc3339(),
                    op_id,
                    survivor_id,
                    serde_json::to_string(&op.affected_ids)
                        .map_err(|error| StorageError::Init(error.to_string()))?,
                    format!("Reverted {} operation {op_id}", op.op_type)
                ],
            )?;
            tx.commit()?;
        }
        // All durable state is committed before index cleanup or inference.
        if let Some((id, content)) = regenerated {
            if let Some(index) = self.vector_index.as_ref()
                && let Ok(mut index) = index.lock()
            {
                let _ = index.remove(&id);
            }
            if self.active_embedding_runtime_ready().unwrap_or(false)
                && let Err(error) = self.generate_embedding_for_node(&id, &content)
            {
                tracing::warn!(%error, "undo committed; embedding remains pending regeneration");
            }
        }
        self.read_operation(&new_op_id)?
            .ok_or_else(|| StorageError::Init("undo operation vanished after insert".into()))
    }

    /// List recent merge/supersede operations (the reflog), newest first.
    pub fn list_merge_operations(
        &self,
        limit: usize,
    ) -> Result<Vec<crate::advanced::MergeOperation>> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        let mut stmt = reader.prepare(
            "SELECT id, plan_id, op_type, status, created_at, reverted_at, reverts_op_id,
                    survivor_id, affected_ids, confidence, signals, reason
             FROM merge_operations ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], Self::row_to_operation)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Read one durable merge/tag operation from the memory reflog.
    pub fn get_merge_operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<crate::advanced::MergeOperation>> {
        self.read_operation(operation_id)
    }

    /// Read a single operation by id.
    pub(super) fn read_operation(
        &self,
        op_id: &str,
    ) -> Result<Option<crate::advanced::MergeOperation>> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        let op = reader
            .query_row(
                "SELECT id, plan_id, op_type, status, created_at, reverted_at, reverts_op_id,
                        survivor_id, affected_ids, confidence, signals, reason
                 FROM merge_operations WHERE id = ?1",
                params![op_id],
                Self::row_to_operation,
            )
            .optional()?;
        Ok(op)
    }

    pub(super) fn row_to_operation(
        row: &rusqlite::Row,
    ) -> rusqlite::Result<crate::advanced::MergeOperation> {
        let affected: String = row.get("affected_ids")?;
        let affected_ids: Vec<String> = serde_json::from_str(&affected).unwrap_or_default();
        Ok(crate::advanced::MergeOperation {
            id: row.get("id")?,
            plan_id: row.get("plan_id").ok().flatten(),
            op_type: row.get("op_type")?,
            status: row.get("status")?,
            created_at: row.get("created_at")?,
            reverted_at: row.get("reverted_at").ok().flatten(),
            reverts_op_id: row.get("reverts_op_id").ok().flatten(),
            survivor_id: row.get("survivor_id").ok().flatten(),
            affected_ids,
            confidence: row
                .get::<_, Option<f64>>("confidence")
                .ok()
                .flatten()
                .map(|v| v as f32),
            signals: row
                .get::<_, Option<String>>("signals")
                .ok()
                .flatten()
                .and_then(|value| serde_json::from_str(&value).ok()),
            reason: row.get("reason").ok().flatten(),
        })
    }

    /// Read (valid_until, superseded_by) for a node.
    /// Read a node's bitemporal columns off the reader connection. Production
    /// now snapshots inside the apply transaction instead (see
    /// [`Self::read_bitemporal_in_transaction`]); this remains as the assertion
    /// helper the merge/supersede tests read state through.
    #[cfg(all(test, feature = "embeddings", feature = "vector-search"))]
    pub(super) fn read_bitemporal(&self, id: &str) -> Result<(Option<String>, Option<String>)> {
        let reader = self
            .reader
            .lock()
            .map_err(|_| StorageError::Init("Reader lock poisoned".into()))?;
        let res = reader
            .query_row(
                "SELECT valid_until, superseded_by FROM knowledge_nodes WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()?;
        res.ok_or_else(|| StorageError::NotFound(id.to_string()))
    }

    /// `read_bitemporal` against an open transaction, so a snapshot and the
    /// mutation it protects observe the same database state.
    #[cfg(all(feature = "embeddings", feature = "vector-search"))]
    pub(super) fn read_bitemporal_in_transaction(
        tx: &rusqlite::Transaction<'_>,
        id: &str,
    ) -> Result<(Option<String>, Option<String>)> {
        let res = tx
            .query_row(
                "SELECT valid_until, superseded_by FROM knowledge_nodes WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()?;
        res.ok_or_else(|| StorageError::NotFound(id.to_string()))
    }

    /// `invalidate_node` against an open transaction. The helper that takes the
    /// writer lock itself cannot be called from inside a transaction: the lock
    /// is not reentrant, so it would deadlock.
    #[cfg(all(feature = "embeddings", feature = "vector-search"))]
    pub(super) fn invalidate_node_in_transaction(
        tx: &rusqlite::Transaction<'_>,
        id: &str,
        superseded_by: &str,
        now: DateTime<Utc>,
    ) -> Result<()> {
        tx.execute(
            "UPDATE knowledge_nodes
             SET valid_until = ?1, superseded_by = ?2, updated_at = ?1
             WHERE id = ?3",
            params![now.to_rfc3339(), superseded_by, id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod exact_nomination_tests {
    //! merge_candidates nominates by EXACT EQUALITY ONLY (owner decision
    //! 2026-09-28): identical content hash / same declared source key /
    //! exactly equal entity sets. Near-identical content must NOT nominate.

    use crate::{IngestInput, Storage};

    fn store() -> (tempfile::TempDir, Storage) {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::new(Some(dir.path().join("test.db"))).unwrap();
        (dir, storage)
    }

    fn ingest(storage: &Storage, content: &str, tags: &[&str]) -> String {
        storage
            .ingest(IngestInput {
                content: content.to_string(),
                tags: tags.iter().map(|t| t.to_string()).collect(),
                ..Default::default()
            })
            .unwrap()
            .id
    }

    fn ingest_with_envelope(
        storage: &Storage,
        content: &str,
        envelope: crate::memory::SourceEnvelope,
    ) -> String {
        storage
            .ingest(IngestInput {
                content: content.to_string(),
                source_envelope: Some(envelope),
                ..Default::default()
            })
            .unwrap()
            .id
    }

    fn envelope(
        source_system: Option<&str>,
        source_id: Option<&str>,
        content_hash: Option<&str>,
    ) -> crate::memory::SourceEnvelope {
        crate::memory::SourceEnvelope {
            source_system: source_system.map(String::from),
            source_id: source_id.map(String::from),
            content_hash: content_hash.map(String::from),
            ..Default::default()
        }
    }

    fn candidate_members(
        storage: &Storage,
    ) -> Vec<Vec<String>> {
        storage
            .merge_candidates(crate::advanced::MergePolicy::default(), 20, &[])
            .unwrap()
            .into_iter()
            .map(|c| c.member_ids)
            .collect()
    }

    #[test]
    fn identical_content_nominate() {
        let (_dir, storage) = store();
        let a = ingest(&storage, "Deploy the gateway before Friday", &[]);
        let b = ingest(&storage, "Deploy the gateway before Friday", &[]);
        let _c = ingest(&storage, "An unrelated memory about cooking", &[]);

        let clusters = candidate_members(&storage);
        assert_eq!(clusters.len(), 1, "exactly one exact-duplicate cluster");
        assert!(clusters[0].contains(&a) && clusters[0].contains(&b));
        assert!(!clusters[0].contains(&_c));
    }

    #[test]
    fn identical_content_hash_nominate_across_different_text() {
        let (_dir, storage) = store();
        // Same declared payload hash, different raw text (e.g. two renderings
        // of the same upstream record). The stored hash is the identity.
        let a = ingest_with_envelope(
            &storage,
            "issue 7: timeout on import",
            envelope(None, None, Some("sha256:abc")),
        );
        let b = ingest_with_envelope(
            &storage,
            "issue 7: timeout during import (reformatted)",
            envelope(None, None, Some("sha256:abc")),
        );

        let clusters = candidate_members(&storage);
        assert_eq!(clusters.len(), 1);
        assert!(clusters[0].contains(&a) && clusters[0].contains(&b));
    }

    #[test]
    fn same_source_key_nominate_even_with_diverged_content() {
        let (_dir, storage) = store();
        // Fresh schemas enforce a UNIQUE index on the source key, so a
        // same-key duplicate can only exist in a store written before that
        // constraint (or with it relaxed). Simulate that legacy state by
        // dropping the index for the duration of the test.
        {
            let writer = storage.writer.lock().unwrap();
            writer.execute_batch("DROP INDEX idx_nodes_source_key").unwrap();
        }
        let a = ingest_with_envelope(
            &storage,
            "Redmine 42: original description",
            envelope(Some("redmine"), Some("42"), None),
        );
        let b = ingest_with_envelope(
            &storage,
            "Redmine 42: edited description after upstream change",
            envelope(Some("redmine"), Some("42"), None),
        );
        // A different source key must not join.
        let c = ingest_with_envelope(
            &storage,
            "Redmine 42: cross-posted note",
            envelope(Some("jira"), Some("42"), None),
        );

        let clusters = candidate_members(&storage);
        assert_eq!(clusters.len(), 1);
        assert!(clusters[0].contains(&a) && clusters[0].contains(&b));
        assert!(!clusters[0].contains(&c));
    }

    #[test]
    fn near_identical_content_no_longer_nominate() {
        let (_dir, storage) = store();
        // Under the old cosine scan this pair scored ~0.99 and clustered.
        // It shares no content hash, no source key, and its extracted entity
        // sets differ (services vs service), so exact-equality nomination
        // must NOT offer it.
        ingest(&storage, "Use tokio runtime for async Rust services", &[]);
        ingest(&storage, "Use the tokio runtime for async Rust service", &[]);

        let clusters = candidate_members(&storage);
        assert!(
            clusters.is_empty(),
            "near-identical content must not be nominated: {clusters:?}"
        );
    }

    #[test]
    fn exact_entity_set_nominate() {
        let (_dir, storage) = store();
        // Two notes over the exact same entity set {alpha, beta, gamma,
        // delta}: different word order and stopword filler (words shorter
        // than 4 letters are not entities).
        ingest(&storage, "alpha beta gamma delta", &[]);
        ingest(&storage, "delta or gamma and beta of alpha", &[]);
        // A third note with a different entity set stays out.
        ingest(&storage, "tokio runtime tuning for the worker pool", &[]);

        let clusters = candidate_members(&storage);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].len(), 2);
    }

    #[test]
    fn tag_filter_restricts_nomination() {
        let (_dir, storage) = store();
        ingest(&storage, "Duplicated release note text", &["rust"]);
        ingest(&storage, "Duplicated release note text", &["python"]);

        let clusters = candidate_members(&storage);
        assert_eq!(clusters.len(), 1, "no tag filter: the pair is nominated");

        let filtered = storage
            .merge_candidates(
                crate::advanced::MergePolicy::default(),
                20,
                &["rust".to_string()],
            )
            .unwrap();
        assert!(
            filtered.is_empty(),
            "with one member filtered out, the cluster dissolves"
        );
    }
}
