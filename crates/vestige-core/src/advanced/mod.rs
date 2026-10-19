//! # Advanced Memory Features
//!
//! Cognitive memory capabilities for the Strata Causal Proof Engine:
//! - **Causal Walk**: Backward/forward graph traversal over recorded edges
//! - **Retroactive Backfill**: Failure-to-precursor causal link discovery
//! - **Reconsolidation**: Labile state and controlled memory updates on retrieval
//! - **Merge / Supersede**: Deterministic merge plans with provenance preservation
//! - **Contradiction Detection**: Structural polarity and numeric conflict checks
//! - **Importance & Intent**: Multi-factor usage tracking and intent classification
//! - **Cross-Project Learning**: Pattern transfer across project scopes
//! - **Memory Dreams**: Consolidation scheduling and activity tracking

pub mod causal_walk;
pub mod contradiction;
pub mod cross_project;
pub mod dreams;
pub mod git_records;
pub mod importance;
pub mod intent;
pub mod merge_supersede;
pub mod reconsolidation;
pub mod retroactive_backfill;

pub use causal_walk::{
    CausalWalkRequest, CausalWalkResult, EVIDENCE_LINK_TYPE, NeedsReport, PathHop, StartPoint,
    persist_evidence_edges, walk_storage,
};
pub use contradiction::{SubjectIdentity, appears_contradictory};
pub use cross_project::{
    ApplicableKnowledge, CrossProjectLearner, ProjectContext, UniversalPattern,
};
pub use dreams::{
    ActivityStats, ActivityTracker, ConnectionGraph, ConnectionReason, ConnectionStats,
    ConsolidationReport, ConsolidationScheduler, DiscoveredConnection, DiscoveredConnectionType,
    DreamConfig, DreamMemory, DreamResult, MemoryConnection, MemoryDreamer, MemoryReplay, Pattern,
    PatternType, SynthesizedInsight,
};
pub use importance::{ImportanceDecayConfig, ImportanceScore, ImportanceTracker, UsageEvent};
pub use intent::{ActionType, DetectedIntent, IntentDetector, MaintenanceType, UserAction};
pub use merge_supersede::{
    DEFAULT_MATCH_THRESHOLD, DEFAULT_POSSIBLE_THRESHOLD, MatchClass, MatchSignals, MergeCandidate,
    MergeOperation, MergePlan, MergePolicy, PlanKind, ReconsolidationMeta, compose_merged_content,
    compose_merged_tags, score_pair,
};
pub use reconsolidation::{
    AccessContext, AccessTrigger, AppliedModification, ChangeSummary, LabileCandidate, LabileState,
    MemorySnapshot, Modification, ReconsolidatedMemory, ReconsolidationManager,
    ReconsolidationStats, RelationshipType, RetrievalRecord,
};
pub use retroactive_backfill::{
    BackfillCandidate, BackfillResult, BackfilledCause, FailureEvent, RetroactiveBackfill,
};
