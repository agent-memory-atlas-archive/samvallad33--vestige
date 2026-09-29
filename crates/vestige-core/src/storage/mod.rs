//! Storage Module
//!
//! Backend-agnostic memory store abstraction plus SQLite reference impl.

mod attestation_store;
#[cfg(feature = "cloud-sync")]
mod cloud_crypto;
#[cfg(feature = "cloud-sync")]
mod cloud_sync;
mod edges;
mod blast;
mod intention_claim;
mod intention_graph_store;
mod memory_store;
mod maintenance_batches;
mod migrations;
mod portable;
pub mod receipt_attestation;
mod replay_store;
mod resolver;
mod session_queries;
mod sqlite;
mod synaptic_store;
mod trace_store;
mod walk_receipts;
pub mod unlearning;
mod unlearning_store;

#[cfg(feature = "cloud-sync")]
pub use cloud_sync::HttpPortableSyncBackend;

pub use attestation_store::{
    DurableSignedReceipt, DurableSignedRetrievalReceipt, ProvisionedReceiptSigningKey,
    ReceiptAttestationStatus, ReceiptSigningKeyTransition, SignedReceiptWrite,
    StoredReceiptAttestationVerification, load_receipt_signing_seed,
    provision_receipt_signing_key_sidecar,
};
pub use blast::{
    BLAST_LINK_TYPES, BLAST_MAX_DEPTH, BLAST_SCAN_NODE_CAP, BlastAffected, BlastReport,
    RetireOutcome, commit_sha_of,
};
pub use memory_store::{
    ClassificationResult, Domain, HealthStatus, LocalMemoryStore, MemoryEdge, MemoryRecord,
    MemoryStore, MemoryStoreError, MemoryStoreResult, MemoryStoreSend, ModelSignature,
    SchedulingState, SearchQuery, SearchResult, StoreStats,
};
pub use edges::{
    EdgeDirection, EdgeKind, EdgeMeta, PurgeTombstone, TYPED_EDGE_VOCABULARY, TypedEdge,
};
pub use migrations::MIGRATIONS;
pub use portable::{
    PORTABLE_ARCHIVE_FORMAT, PortableArchive, PortableImportMode, PortableImportReport,
    PortableTable, PortableValue,
};
pub use resolver::{HANDLE_REQUIRED_DETAIL, HandleKind, HandleResolution, MAX_CANDIDATES};
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
pub use sqlite::{WalCheckpointMode, WalCheckpointStatus};
pub use sqlite::{
    ACCESS_LOG_RETENTION_DAYS, ActorMutationOutcome, CompositionEventRecord,
    CompositionMemberRecord, CompositionNeighborRecord, CompositionOutcomeRecord,
    ConnectionRecord, ConnectorCursor, ConsolidationHistoryRecord, DEFAULT_MEMORY_SCOPE,
    DreamHistoryRecord, EmbeddingProfileIntegrityManifest,
    EmbeddingProfileMigrationNodeCheckpoint, EmbeddingProfileMigrationRecord,
    EmbeddingProfileVector, EndorsementEventRecord, FilePortableSyncBackend, HygieneNodeSummary,
    HygieneSnapshot, InsightRecord, IntentionRecord, NeverComposedCandidate,
    PortableSyncBackend, PortableSyncReport, PurgeReport, ReconcileReport, Result,
    SmartIngestResult,
    SourceUpsertOutcome, SourceUpsertResult, SqliteMemoryStore, StateTransitionRecord,
    StorageError, TagVocabulary,
};
pub use synaptic_store::{
    DurableSynapticCapture, DurableSynapticPairReceipt, SYNAPTIC_CAPTURE_ALGORITHM_V1,
    SYNAPTIC_CAPTURE_ALGORITHM_V2, SYNAPTIC_CAPTURE_CLAIM_BOUNDARY, SYNAPTIC_CAPTURE_SCHEMA_V1,
    SYNAPTIC_CAPTURE_SCHEMA_V2, SYNAPTIC_CONTEXT_ALGORITHM_V1, SYNAPTIC_CONTEXT_THRESHOLD_V1,
    SynapticCapturePolicy, SynapticCaptureRequest, SynapticImportanceEvent, SynapticIngestOutcome,
    SynapticIngestRequest, SynapticSignalSnapshot,
};
pub use session_queries::{
    ClosedIssueNode, FailedToolCall, GitCommitNode, OpenFailureTouching, FAILED_CALLS_MAX,
};
pub use trace_store::{
    AgentRunSummary, PendingMemoryMutationDecision, PendingMemoryMutationEffect,
};
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
pub use unlearning_store::{
    AntiResurrectionGateStatus, CanaryScanResult, EligibleAuditRecord, ErasureFailureCode,
    ErasureJobStart, ErasureJobStatus, ExactCanary, LocalCanaryTable, StoredErasureJob,
    TombstoneWriteOutcome, UnlearningStore, UnlearningStoreError, UnlearningStoreResult,
    V25_REQUIRED_LOCAL_CANARY_TABLES, V25_UNLEARNING_STORAGE_SCHEMA_EXPECTATION,
    V25_UNLEARNING_STORAGE_SCHEMA_VERSION,
};

/// Phase 4 storage wall: the product-wide seam.
///
/// Every tool, server, CLI entry point, and cognitive module holds the store
/// through this trait object (`Arc<Storage>`, `&Storage`). `SqliteMemoryStore`
/// is one implementation of it, constructed only via [`open_storage`] (and
/// direct backend construction inside `vestige-core`'s own tests). A second
/// engine implements `MemoryStoreSend` and drops in behind the same alias.
pub type Storage = dyn MemoryStore;

/// Default database artifact path for the SQLite backend.
pub fn default_db_path() -> Result<std::path::PathBuf> {
    SqliteMemoryStore::default_db_path()
}

/// Database artifact path for a given data directory (SQLite backend).
pub fn db_path_for_data_dir(data_dir: std::path::PathBuf) -> Result<std::path::PathBuf> {
    SqliteMemoryStore::db_path_for_data_dir(data_dir)
}

/// Construct the default local backend (SQLite reference implementation)
/// behind the Phase 4 storage trait. This is the only constructor the MCP
/// layer is allowed to call.
pub fn open_storage(path: Option<std::path::PathBuf>) -> Result<std::sync::Arc<dyn MemoryStore>> {
    Ok(std::sync::Arc::new(SqliteMemoryStore::new(path)?))
}
