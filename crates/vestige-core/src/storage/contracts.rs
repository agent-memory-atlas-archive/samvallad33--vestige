//! Pure storage contracts. No SQLite, no similarity.
//!
//! Constants and digests the MCP surface names in every build. The legacy
//! modules re-export these so existing paths stay stable.

use serde_json::Value;

use super::{Result, StorageError};

pub const BLAST_LINK_TYPES: [&str; 3] = ["derived_from", "backfill_candidate", "evidence_of"];
pub const BLAST_MAX_DEPTH: u32 = 5;
pub const BLAST_SCAN_NODE_CAP: usize = 20_000;

const MIN_SHA_CHARS: usize = 6;

/// Extract the sha from a record's `commit <sha> ...` line, if any.
pub fn commit_sha_of(content: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix("commit ") else {
            continue;
        };
        let Some(token) = rest.split_whitespace().next() else {
            continue;
        };
        if token.len() >= MIN_SHA_CHARS && token.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(token.to_ascii_lowercase());
        }
    }
    None
}

pub const PORTABLE_ARCHIVE_FORMAT: &str = "vestige.portable.v1";

pub const REPLAY_SELECTION_BOUNDARY: &str = "post_retrieval_context_ablation";
pub const REPLAY_ALGORITHM_VERSION: &str = "vestige.post_retrieval_context_ablation.v1";
pub const REPLAY_SCHEMA_VERSION: u32 = 1;
pub const REPLAY_CLAIM_BOUNDARY: &str = "Controlled replay shows how the recorded memory context changes when specified evidence is withheld. It does not establish that a memory caused an agent answer or any real-world outcome.";

const PRIVATE_DIGEST_DOMAIN: &[u8] = b"vestige.replay.private-item.v1";
const POLICY_DIGEST_DOMAIN: &[u8] = b"vestige.replay.policy.v1";
const IDEMPOTENCY_DOMAIN: &[u8] = b"vestige.replay.idempotency.v1";

fn put_field(hasher: &mut blake3::Hasher, value: &[u8]) {
    hasher.update(&(value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn normalize_withheld_slots(withheld_slots: &[String]) -> Vec<String> {
    use std::collections::BTreeSet;
    withheld_slots
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn private_evidence_digest(
    private_key: &[u8; 32],
    evidence_slot: &str,
    evidence_bytes: &[u8],
) -> String {
    let mut hasher = blake3::Hasher::new_keyed(private_key);
    put_field(&mut hasher, PRIVATE_DIGEST_DOMAIN);
    put_field(&mut hasher, evidence_slot.as_bytes());
    put_field(&mut hasher, evidence_bytes);
    format!("b3k:{}", hasher.finalize().to_hex())
}

pub fn replay_policy_digest(canonical_policy: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    put_field(&mut hasher, POLICY_DIGEST_DOMAIN);
    put_field(&mut hasher, canonical_policy);
    format!("b3:{}", hasher.finalize().to_hex())
}

pub fn replay_evidence_slot(rank: usize) -> String {
    format!("evidence_{rank}")
}

pub fn replay_idempotency_key(
    algorithm_version: &str,
    source_receipt_id: &str,
    redaction_generation: u64,
    withheld_slots: &[String],
) -> String {
    let normalized = normalize_withheld_slots(withheld_slots);
    let mut hasher = blake3::Hasher::new();
    put_field(&mut hasher, IDEMPOTENCY_DOMAIN);
    put_field(&mut hasher, algorithm_version.as_bytes());
    put_field(&mut hasher, source_receipt_id.as_bytes());
    put_field(&mut hasher, &redaction_generation.to_be_bytes());
    for slot in normalized {
        put_field(&mut hasher, slot.as_bytes());
    }
    format!("b3:{}", hasher.finalize().to_hex())
}

pub const SYNAPTIC_CAPTURE_ALGORITHM_V1: &str = "vestige.synaptic_capture.v1";
pub const SYNAPTIC_CAPTURE_SCHEMA_V1: &str =
    "https://vestige.dev/schemas/receipt/synaptic-capture/v1";
pub const SYNAPTIC_CAPTURE_ALGORITHM_V2: &str = "vestige.synaptic_capture.v2";
pub const SYNAPTIC_CAPTURE_SCHEMA_V2: &str =
    "https://vestige.dev/schemas/receipt/synaptic-capture/v2";
pub const SYNAPTIC_CONTEXT_ALGORITHM_V1: &str = "vestige.synaptic_context.v1";
pub const SYNAPTIC_CONTEXT_THRESHOLD_V1: f64 = 0.25;
pub const SYNAPTIC_CAPTURE_CLAIM_BOUNDARY: &str = "Evidence-backed temporal association with a measured memory-state change; not proof that the trigger caused the earlier memory or a downstream outcome.";

/// Canonicalize a walk parameter envelope (RFC 8785). Input must be an object.
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
