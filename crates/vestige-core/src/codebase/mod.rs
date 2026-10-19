//! Codebase Anchor & Staleness Module
//!
//! Pure, deterministic file-span anchoring (`path#symbol` or `path#Lstart-Lend`)
//! and hash-based staleness verification for Strata code memories.

pub mod anchor;
pub mod staleness;

pub use anchor::{
    AnchorDraft, AnchorStatus, AnchorVerification, CodeAnchor, MAX_ANCHORED_FILE_BYTES,
    MAX_SPAN_LINES, capture_anchor, find_symbol_definition, hash_span, verify_anchor,
};
