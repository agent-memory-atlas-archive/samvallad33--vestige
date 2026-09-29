//! Type stand-ins for a `vestige-core` build without `legacy-sqlite`.
//!
//! The store trait is always compiled and still names the SQLite surface.
//! The real definitions stay behind the feature. Method bodies on the trait
//! already return [`StorageError::Init`] for an unimplemented backend, so
//! these types only have to be nameable.

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StorageError {
    #[error("Initialization error: {0}")]
    Init(String),
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// Borrowed write descriptor. The SQLite type carries the receipt bytes;
/// this build only keeps the lifetime the trait signature names.
pub struct SignedReceiptWrite<'a> {
    _borrow: std::marker::PhantomData<&'a ()>,
}

macro_rules! absent_types {
    ($($name:ident),* $(,)?) => {
        $(
            #[derive(Debug, Clone)]
            pub struct $name;
        )*
    };
}

absent_types! {
    ActorMutationOutcome,
    AgentRunSummary,
    BlastReport,
    ClosedIssueNode,
    CompositionEventRecord,
    CompositionMemberRecord,
    CompositionNeighborRecord,
    CompositionOutcomeRecord,
    ConnectionRecord,
    ConnectorCursor,
    ConsolidationHistoryRecord,
    CoverageSnapshot,
    DreamHistoryRecord,
    DurableCounterfactualReplay,
    DurableRetrievalReplayCapsule,
    DurableSignedRetrievalReceipt,
    DurableSynapticCapture,
    EndorsementEventRecord,
    FailedToolCall,
    FailureFeedbackReport,
    GitCommitNode,
    HandleResolution,
    HygieneSnapshot,
    InsightRecord,
    IntentionRecord,
    NeverComposedCandidate,
    OpenFailureTouching,
    PendingMemoryMutationDecision,
    PortableArchive,
    PortableImportMode,
    PortableImportReport,
    PortableSyncReport,
    PurgeReport,
    ReceiptAttestationStatus,
    ReconcileReport,
    RetireOutcome,
    RetrievalReplayCapsuleDraft,
    RetrievalReplayCapsuleSummary,
    SmartIngestResult,
    SourceUpsertResult,
    StateTransitionRecord,
    StoredCounterfactualReplay,
    StoredReceiptAttestationVerification,
    StoredWalkReceipt,
    SynapticCaptureRequest,
    SynapticIngestOutcome,
    SynapticIngestRequest,
    TagVocabulary,
    WalkReceiptHandle,
    WalCheckpointMode,
    WalCheckpointStatus,
}
