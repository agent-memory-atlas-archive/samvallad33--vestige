//! Storage Module
//!
//! Backend-agnostic memory store abstraction plus SQLite reference impl.
//!
//! The SQLite reference implementation (and every module that talks
//! rusqlite) is quarantined behind the `legacy-sqlite` feature
//! (build/t5-legacy-isolation). Default ON; the flip to default-off lands
//! with the STRATA backend. Builders must open stores through
//! [`open_storage`] so a `legacy-sqlite`-free build fails with the clear
//! [`LegacySqliteDisabled`] error instead of a compile error.

#[cfg(feature = "legacy-sqlite")]
mod attestation_store;
#[cfg(feature = "cloud-sync")]
mod cloud_crypto;
#[cfg(all(feature = "cloud-sync", feature = "legacy-sqlite"))]
mod cloud_sync;
#[cfg(feature = "legacy-sqlite")]
mod edges;
#[cfg(feature = "legacy-sqlite")]
mod blast;
#[cfg(feature = "legacy-sqlite")]
mod intention_claim;
#[cfg(feature = "legacy-sqlite")]
mod intention_graph_store;
mod memory_store;
#[cfg(feature = "legacy-sqlite")]
mod maintenance_batches;
#[cfg(feature = "legacy-sqlite")]
mod migrations;
#[cfg(feature = "legacy-sqlite")]
mod portable;
pub mod receipt_attestation;
#[cfg(feature = "legacy-sqlite")]
mod replay_store;
#[cfg(feature = "legacy-sqlite")]
mod resolver;
#[cfg(feature = "legacy-sqlite")]
mod session_queries;
#[cfg(feature = "legacy-sqlite")]
mod sqlite;
#[cfg(feature = "legacy-sqlite")]
mod synaptic_store;
#[cfg(feature = "legacy-sqlite")]
mod trace_store;
#[cfg(feature = "legacy-sqlite")]
mod walk_receipts;
pub mod unlearning;
#[cfg(feature = "legacy-sqlite")]
mod unlearning_store;

#[cfg(all(feature = "cloud-sync", feature = "legacy-sqlite"))]
pub use cloud_sync::HttpPortableSyncBackend;

#[cfg(feature = "legacy-sqlite")]
pub use attestation_store::{
    DurableSignedReceipt, DurableSignedRetrievalReceipt, ProvisionedReceiptSigningKey,
    ReceiptAttestationStatus, ReceiptSigningKeyTransition, SignedReceiptWrite,
    StoredReceiptAttestationVerification, load_receipt_signing_seed,
    provision_receipt_signing_key_sidecar,
};
#[cfg(feature = "legacy-sqlite")]
pub use blast::{
    BLAST_LINK_TYPES, BLAST_MAX_DEPTH, BLAST_SCAN_NODE_CAP, BlastAffected, BlastReport,
    RetireOutcome, commit_sha_of,
};
pub use memory_store::{
    ClassificationResult, Domain, HealthStatus, LocalMemoryStore, MemoryEdge, MemoryRecord,
    MemoryStore, MemoryStoreError, MemoryStoreResult, MemoryStoreSend, ModelSignature,
    SchedulingState, SearchQuery, SearchResult, StoreStats,
};
#[cfg(feature = "legacy-sqlite")]
pub use edges::{
    EdgeDirection, EdgeKind, EdgeMeta, PurgeTombstone, TYPED_EDGE_VOCABULARY, TypedEdge,
};
#[cfg(feature = "legacy-sqlite")]
pub use migrations::MIGRATIONS;
#[cfg(feature = "legacy-sqlite")]
pub use portable::{
    PORTABLE_ARCHIVE_FORMAT, PortableArchive, PortableImportMode, PortableImportReport,
    PortableTable, PortableValue,
};
#[cfg(feature = "legacy-sqlite")]
pub use resolver::{HANDLE_REQUIRED_DETAIL, HandleKind, HandleResolution, MAX_CANDIDATES};
#[cfg(feature = "legacy-sqlite")]
pub use replay_store::{
    CounterfactualReplayResult, DurableCounterfactualReplay, DurableRetrievalReplayCapsule,
    FrozenReplayItem, REPLAY_ALGORITHM_VERSION, REPLAY_CLAIM_BOUNDARY, REPLAY_SCHEMA_VERSION,
    REPLAY_SELECTION_BOUNDARY, ReplayBuildError, ReplayDecayRisk, ReplayEvidenceItemSummary,
    ReplayEvidenceSetSummary, ReplayInfluence, ReplayInvalidationReason,
    ReplayMaterializationCheck, ReplayPrivacyInvalidation, ReplayPrivacyState,
    RetrievalReplayCapsuleDraft, RetrievalReplayCapsuleSummary, RetrievalReplayItemDraft,
    StoredCounterfactualReplay, ablate_frozen_context, private_evidence_digest,
    replay_evidence_slot, replay_idempotency_key, replay_policy_digest,
};
#[cfg(feature = "legacy-sqlite")]
pub use sqlite::{WalCheckpointMode, WalCheckpointStatus};
#[cfg(feature = "legacy-sqlite")]
pub use sqlite::{
    ACCESS_LOG_RETENTION_DAYS, ActorMutationOutcome, CompositionEventRecord,
    CompositionMemberRecord, CompositionNeighborRecord, CompositionOutcomeRecord,
    ConnectionRecord, ConnectorCursor, ConsolidationHistoryRecord, DEFAULT_MEMORY_SCOPE,
    DreamHistoryRecord, EmbeddingProfileIntegrityManifest,
    EmbeddingProfileMigrationNodeCheckpoint, EmbeddingProfileMigrationRecord,
    EmbeddingProfileVector, EndorsementEventRecord, FilePortableSyncBackend, HygieneNodeSummary,
    HygieneSnapshot, InsightRecord, IntentionRecord, NeverComposedCandidate,
    PortableSyncBackend, PortableSyncReport, ReconcileReport, Result, SmartIngestResult,
    SourceUpsertOutcome, SourceUpsertResult, SqliteMemoryStore, StateTransitionRecord,
    StorageError, TagVocabulary,
};
#[cfg(feature = "legacy-sqlite")]
pub use synaptic_store::{
    DurableSynapticCapture, DurableSynapticPairReceipt, SYNAPTIC_CAPTURE_ALGORITHM_V1,
    SYNAPTIC_CAPTURE_ALGORITHM_V2, SYNAPTIC_CAPTURE_CLAIM_BOUNDARY, SYNAPTIC_CAPTURE_SCHEMA_V1,
    SYNAPTIC_CAPTURE_SCHEMA_V2, SYNAPTIC_CONTEXT_ALGORITHM_V1, SYNAPTIC_CONTEXT_THRESHOLD_V1,
    SynapticCapturePolicy, SynapticCaptureRequest, SynapticImportanceEvent, SynapticIngestOutcome,
    SynapticIngestRequest, SynapticSignalSnapshot,
};
#[cfg(feature = "legacy-sqlite")]
pub use session_queries::{
    ClosedIssueNode, FailedToolCall, GitCommitNode, OpenFailureTouching, FAILED_CALLS_MAX,
};
#[cfg(feature = "legacy-sqlite")]
pub use trace_store::{
    AgentRunSummary, PendingMemoryMutationDecision, PendingMemoryMutationEffect,
};
#[cfg(feature = "legacy-sqlite")]
pub use walk_receipts::{
    CoverageSnapshot, StoredWalkReceipt, WalkReceiptHandle, WALK_RECEIPT_SCHEMA_V1,
    canonical_walk_json,
};
pub use unlearning::{
    AntiResurrectionCommitments, ArtifactKind, ArtifactRef, CheckStatus, Commitment, CommitmentKey,
    CommitmentKind, ErasureLedgerRecord, GuaranteeExclusion, LineageClosure, LineageEdge,
    LineageRelation, PostconditionCheck, PostconditionKind, PostconditionReport, SurfaceAction,
    SurfaceDetailCode, SurfaceResult, UnlearningScope, UnlearningVerdict,
    VERIFIED_LOCAL_UNLEARNING_SCHEMA_V1, VerificationFence, anti_resurrection_commitments,
    commit_lineage_closure, compute_lineage_closure, evaluate_unlearning_verdict,
};
#[cfg(feature = "legacy-sqlite")]
pub use unlearning_store::{
    AntiResurrectionGateStatus, CanaryScanResult, EligibleAuditRecord, ErasureFailureCode,
    ErasureJobStart, ErasureJobStatus, ExactCanary, LocalCanaryTable, StoredErasureJob,
    TombstoneWriteOutcome, UnlearningStore, UnlearningStoreError, UnlearningStoreResult,
    V25_REQUIRED_LOCAL_CANARY_TABLES, V25_UNLEARNING_STORAGE_SCHEMA_EXPECTATION,
    V25_UNLEARNING_STORAGE_SCHEMA_VERSION,
};

/// Backwards-compatibility alias. Retained until Phase 4 completes so every
/// existing `Arc<Storage>` call site keeps compiling. Scheduled for removal
/// once no downstream source file references it.
#[cfg(feature = "legacy-sqlite")]
pub type Storage = SqliteMemoryStore;

/// Error returned by [`open_storage`] when the binary was built without the
/// `legacy-sqlite` feature. Exists in every build so callers can name it
/// (and print it) regardless of feature state.
#[derive(Debug, thiserror::Error)]
#[error("built without legacy-sqlite; STRATA default lands in the next merge")]
pub struct LegacySqliteDisabled;

/// Open the process-local store.
///
/// The single sanctioned constructor path (build/t5-legacy-isolation).
/// With `legacy-sqlite` on (current default) this is
/// `SqliteMemoryStore::new`. Without it, the call fails with
/// [`LegacySqliteDisabled`] ("built without legacy-sqlite; STRATA default
/// lands in the next merge") — the STRATA backend, not SQLite, becomes the
/// default in the next merge.
#[cfg(feature = "legacy-sqlite")]
pub fn open_storage(path: Option<std::path::PathBuf>) -> sqlite::Result<SqliteMemoryStore> {
    SqliteMemoryStore::new(path)
}

/// Feature-off twin of [`open_storage`]: always fails with
/// [`LegacySqliteDisabled`]. The `Ok` side is `Infallible` because no store
/// type exists in this build.
#[cfg(not(feature = "legacy-sqlite"))]
pub fn open_storage(
    _path: Option<std::path::PathBuf>,
) -> std::result::Result<std::convert::Infallible, LegacySqliteDisabled> {
    Err(LegacySqliteDisabled)
}
