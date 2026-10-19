//! # Neuroscience-Inspired Memory Mechanisms
//!
//! Biologically grounded mechanisms powering Vestige's memory lifecycle:
//! - **Active Forgetting**: Top-down inhibitory control and Rac1 cascade decay
//! - **Synaptic Tagging and Capture (STC)**: Retroactive importance consolidation (Frey & Morris, 1997)
//! - **Memory States**: Continuum of accessibility (Active, Dormant, Silent, Unavailable)
//! - **Multi-Channel Importance Signals**: Dopamine/norepinephrine/acetylcholine-inspired scoring
//! - **Prospective Memory**: Trigger-based future intentions (Einstein & McDaniel, 1990)
//! - **Spreading Activation**: Associative network activation (Collins & Loftus, 1975)

pub mod active_forgetting;
pub mod importance_signals;
pub mod memory_states;
pub mod prospective_memory;
pub mod spreading_activation;
pub mod synaptic_tagging;

// Active forgetting — top-down inhibitory control (Anderson 2025 + Davis Rac1)
pub use active_forgetting::{
    ActiveForgettingSystem, DEFAULT_CASCADE_DECAY, DEFAULT_LABILE_HOURS, DEFAULT_MAX_PENALTY,
    DEFAULT_SIF_K, SuppressionStats,
};

// Synaptic Tagging and Capture (STC)
pub use synaptic_tagging::{
    CaptureResult, CaptureWindow, CapturedMemory, DecayFunction, ImportanceCluster,
    ImportanceEvent, ImportanceEventType, SynapticTag, SynapticTaggingConfig,
    SynapticTaggingSystem, TaggingStats,
};

// Memory states (accessibility continuum)
pub use memory_states::{
    ACCESSIBILITY_ACTIVE, ACCESSIBILITY_DORMANT, ACCESSIBILITY_SILENT, ACCESSIBILITY_UNAVAILABLE,
    AccessibilityCalculator, BatchUpdateResult, COMPETITION_SIMILARITY_THRESHOLD,
    CompetitionCandidate, CompetitionConfig, CompetitionEvent, CompetitionManager,
    CompetitionResult, DEFAULT_ACTIVE_DECAY_HOURS, DEFAULT_DORMANT_DECAY_DAYS, LifecycleSummary,
    MemoryLifecycle, MemoryState, MemoryStateInfo, StateDecayConfig, StatePercentages,
    StateTimeAccumulator, StateTransition, StateTransitionReason, StateUpdateService,
};

// Multi-channel importance signaling (Neuromodulator-inspired)
pub use importance_signals::{
    AccessPattern, ArousalExplanation, ArousalSignal, AttentionExplanation, AttentionSignal,
    CompositeWeights, ConsolidationPriority, Context, EmotionalMarker,
    ImportanceConsolidationConfig, ImportanceEncodingConfig, ImportanceRetrievalConfig,
    ImportanceScore, ImportanceSignals, MarkerType, NoveltyExplanation, NoveltySignal, Outcome,
    OutcomeType, RewardExplanation, RewardSignal, SentimentAnalyzer, SentimentResult, Session,
};

// Prospective memory (Einstein & McDaniel, 1990)
pub use prospective_memory::{
    Context as ProspectiveContext, ContextMonitor, ContextPattern, Intention, IntentionParser,
    IntentionSource, IntentionStats, IntentionStatus, IntentionTrigger, Priority,
    ProspectiveMemory, ProspectiveMemoryConfig, ProspectiveMemoryError, RecurrencePattern,
    TriggerPattern,
};

// Spreading activation (Associative Memory Network - Collins & Loftus, 1975)
pub use spreading_activation::{
    ActivatedMemory, ActivationConfig, ActivationNetwork, ActivationNode, AssociatedMemory,
    AssociationEdge, LinkType,
};
