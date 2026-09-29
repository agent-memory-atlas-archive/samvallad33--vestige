//! Memory Consolidation Module
//!
//! Implements sleep-inspired memory consolidation:
//! - Decay weak memories
//! - Promote emotional/important memories
//! - Generate embeddings
//! - Prune very weak memories (optional)
//! - 4-Phase biologically-accurate dream cycle (v2.0)
//! - Dream compile: the 4-phase engine wired to durable state, review-gated

pub mod dream_compile;
pub mod phases;
mod sleep;

pub use dream_compile::{
    DreamCompileConfig, DreamCompilePhase, DreamCompilePr, DreamCompileReport,
    REPLAY_STRENGTHEN_BOOST, WEAK_EDGE_FLOOR, run_dream_compile,
};
pub use phases::{
    CreativeConnection, CreativeConnectionType, DreamEngine, DreamInsight, DreamPhase,
    FourPhaseDreamResult, PhaseResult, TriageCategory, TriagedMemory,
};
pub use sleep::SleepConsolidation;
