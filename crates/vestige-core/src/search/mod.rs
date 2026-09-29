//! Search Module
//!
//! Provides search capabilities:
//! - Keyword search using BM25/FTS5 (sanitization in `crate::fts`)
//! - Hybrid result fusion with RRF
//! - Temporal-aware search

mod hybrid;
mod temporal;

// Re-exported so `vestige_core::search::sanitize_fts5_query` keeps resolving
// (downstream code uses this path); the implementation lives in `crate::fts`,
// quarantined behind `legacy-sqlite` (build/t5-legacy-isolation).
#[cfg(feature = "legacy-sqlite")]
pub use crate::fts::sanitize_fts5_query;

pub use hybrid::{HybridSearchConfig, HybridSearcher, linear_combination, reciprocal_rank_fusion};

pub use temporal::TemporalSearcher;
