//! CognitiveEngine — Stateful neuroscience modules that persist across tool calls.

use std::sync::Arc;
use vestige_core::neuroscience::prospective_memory::{IntentionParser, ProspectiveMemory};
use vestige_core::{
    AccessibilityCalculator, ActivationNetwork, ActivityTracker, ArousalSignal, AttentionSignal,
    CompetitionManager, ConsolidationScheduler, CrossProjectLearner, ImportanceSignals,
    ImportanceTracker, IntentDetector, LinkType, MemoryDreamer, NoveltySignal,
    ReconsolidationManager, RewardSignal, StateUpdateService, Storage, SynapticTaggingSystem,
};

pub const COGNITIVE_MODULE_COUNT: usize = 16 + 10;

/// Stateful cognitive engine holding neuroscience and lifecycle modules.
pub struct CognitiveEngine {
    // -- Neuroscience --
    pub activation_network: ActivationNetwork,
    pub synaptic_tagging: SynapticTaggingSystem,
    pub accessibility_calc: AccessibilityCalculator,
    pub competition_mgr: CompetitionManager,
    pub state_service: StateUpdateService,
    pub importance_signals: ImportanceSignals,
    pub novelty_signal: NoveltySignal,
    pub arousal_signal: ArousalSignal,
    pub reward_signal: RewardSignal,
    pub attention_signal: AttentionSignal,
    pub prospective_memory: ProspectiveMemory,
    pub intention_parser: IntentionParser,

    // -- Advanced --
    pub importance_tracker: ImportanceTracker,
    pub reconsolidation: ReconsolidationManager,
    pub intent_detector: IntentDetector,
    pub activity_tracker: ActivityTracker,
    pub dreamer: MemoryDreamer,
    pub cross_project: CrossProjectLearner,
    pub consolidation_scheduler: ConsolidationScheduler,
}

impl Default for CognitiveEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CognitiveEngine {
    /// Load persisted source-of-truth state into in-memory cognitive modules.
    pub fn hydrate(&mut self, storage: &Arc<Storage>) {
        match storage.get_all_connections() {
            Ok(connections) => {
                for conn in &connections {
                    let link_type = match conn.link_type.as_str() {
                        "semantic" => LinkType::Semantic,
                        "temporal" => LinkType::Temporal,
                        "causal" => LinkType::Causal,
                        "spatial" => LinkType::Spatial,
                        "shared_concepts" | "complementary" => LinkType::Semantic,
                        _ => LinkType::Semantic,
                    };
                    self.activation_network.add_edge(
                        conn.source_id.clone(),
                        conn.target_id.clone(),
                        link_type,
                        conn.strength,
                    );
                }
                tracing::info!(
                    count = connections.len(),
                    "Hydrated cognitive modules from persisted connections"
                );
            }
            Err(e) => {
                tracing::warn!("Failed to hydrate cognitive modules: {}", e);
            }
        }

        match storage.load_active_synaptic_tags() {
            Ok(tags) => {
                let count = tags.len();
                for tag in tags {
                    self.synaptic_tagging.restore_tag(tag);
                }
                tracing::info!(count, "Hydrated active synaptic tags from durable storage");
            }
            Err(e) => {
                tracing::warn!("Failed to hydrate synaptic tags: {}", e);
            }
        }
    }

    /// Initialize all cognitive modules with default configurations.
    pub fn new() -> Self {
        Self {
            activation_network: ActivationNetwork::new(),
            synaptic_tagging: SynapticTaggingSystem::new(),
            accessibility_calc: AccessibilityCalculator::default(),
            competition_mgr: CompetitionManager::new(),
            state_service: StateUpdateService::new(),
            importance_signals: ImportanceSignals::new(),
            novelty_signal: NoveltySignal::new(),
            arousal_signal: ArousalSignal::new(),
            reward_signal: RewardSignal::new(),
            attention_signal: AttentionSignal::new(),
            prospective_memory: ProspectiveMemory::new(),
            intention_parser: IntentionParser::new(),
            importance_tracker: ImportanceTracker::new(),
            reconsolidation: ReconsolidationManager::new(),
            intent_detector: IntentDetector::new(),
            activity_tracker: ActivityTracker::new(),
            dreamer: MemoryDreamer::new(),
            cross_project: CrossProjectLearner::new(),
            consolidation_scheduler: ConsolidationScheduler::new(),
        }
    }
}
