//! Storage Module — backend-agnostic memory store contracts and types.

mod contracts;
mod memory_store;
pub mod receipt_attestation;
mod signing_seed;
pub mod types;
pub mod v3_guard;

pub use contracts::{
    BLAST_LINK_TYPES, BLAST_MAX_DEPTH, BLAST_SCAN_NODE_CAP, PORTABLE_ARCHIVE_FORMAT,
    REPLAY_ALGORITHM_VERSION, REPLAY_CLAIM_BOUNDARY, REPLAY_SCHEMA_VERSION,
    REPLAY_SELECTION_BOUNDARY, SYNAPTIC_CAPTURE_ALGORITHM_V1, SYNAPTIC_CAPTURE_ALGORITHM_V2,
    SYNAPTIC_CAPTURE_CLAIM_BOUNDARY, SYNAPTIC_CAPTURE_SCHEMA_V1, SYNAPTIC_CAPTURE_SCHEMA_V2,
    SYNAPTIC_CONTEXT_ALGORITHM_V1, SYNAPTIC_CONTEXT_THRESHOLD_V1, canonical_walk_json,
    commit_sha_of, private_evidence_digest, replay_evidence_slot, replay_idempotency_key,
    replay_policy_digest,
};
pub use signing_seed::load_receipt_signing_seed;
pub use types::*;

pub use memory_store::{
    ClassificationResult, Domain, HealthStatus, LocalMemoryStore, MemoryEdge, MemoryRecord,
    MemoryStore, MemoryStoreError, MemoryStoreResult, MemoryStoreSend, ModelSignature,
    SchedulingState, SearchQuery, SearchResult, StoreStats,
};

#[derive(Debug, thiserror::Error)]
#[error("built without legacy-sqlite; STRATA default lands in the next merge")]
pub struct LegacySqliteDisabled;

pub type Storage = dyn MemoryStore;

pub const DEFAULT_MEMORY_SCOPE: &str = "user";
pub const ACCESS_LOG_RETENTION_DAYS: i64 = 90;

pub fn default_db_path() -> Result<std::path::PathBuf> {
    Err(StorageError::Init(
        "SQLite store creation is disabled: SQLite is a read-only migration source in 4.0. hint: the Strata backend owns new stores (run: vestige migrate-to-strata --from <path> for existing v3 stores)".into(),
    ))
}

pub fn db_path_for_data_dir(_data_dir: std::path::PathBuf) -> Result<std::path::PathBuf> {
    Err(StorageError::Init(
        "SQLite store creation is disabled: SQLite is a read-only migration source in 4.0. hint: the Strata backend owns new stores (run: vestige migrate-to-strata --from <path> for existing v3 stores)".into(),
    ))
}

type OpenStorageHook =
    fn(
        Option<std::path::PathBuf>,
    ) -> std::result::Result<std::sync::Arc<dyn MemoryStore>, LegacySqliteDisabled>;

static OPEN_STORAGE_HOOK: std::sync::OnceLock<OpenStorageHook> = std::sync::OnceLock::new();

#[doc(hidden)]
pub fn install_open_storage_hook(hook: OpenStorageHook) {
    let _ = OPEN_STORAGE_HOOK.set(hook);
}

pub fn open_storage(
    path: Option<std::path::PathBuf>,
) -> std::result::Result<std::sync::Arc<dyn MemoryStore>, LegacySqliteDisabled> {
    if let Some(hook) = OPEN_STORAGE_HOOK.get() {
        return hook(path);
    }
    Err(LegacySqliteDisabled)
}
