//! Anchor types for builds without `legacy-sqlite`.
//!
//! Verification lives in the SQLite backend. These stand-ins exist so the
//! storage trait can name the same types; they do not read source trees.

/// Verdict of an anchor check. Without the SQLite backend there is nothing
/// to check, so the only variant is the honest one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorStatus {
    /// This build has no anchor store.
    Unavailable,
}

/// A code anchor. Fields live on the SQLite type; this build does not
/// persist them.
#[derive(Debug, Clone)]
pub struct CodeAnchor;
