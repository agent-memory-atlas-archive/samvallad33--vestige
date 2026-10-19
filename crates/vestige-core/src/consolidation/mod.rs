//! Memory Consolidation Module
//!
//! Data types for Strata review-gated dream compile reports and configuration.

pub mod dream_compile;

pub use dream_compile::{
    DreamCompileConfig, DreamCompilePhase, DreamCompilePr, DreamCompileReport,
    REPLAY_STRENGTHEN_BOOST, WEAK_EDGE_FLOOR,
};
