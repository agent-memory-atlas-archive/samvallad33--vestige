//! Persisted, exact salience adjustments. Planning stays above the store.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_kernel::canonical::to_q32_32;
use strata_kernel::fsrs::{S_MAX, S_MIN};

/// A requested salience adjustment, justified by one exact shared entity.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct BackfillAdjustment {
    /// Earlier memory whose stability changes.
    pub memory_id: String,
    /// Later memory that makes the earlier memory more or less relevant.
    pub trigger_id: String,
    /// Byte-exact identity recorded by both memories.
    pub shared_entity: String,
    /// Plain-language explanation; never a proof of causality.
    pub reason: String,
    /// Requested new stability in Q32.32 days; the store bounds it again.
    pub new_stability_q: i64,
    /// Caller-supplied change time in Unix milliseconds.
    pub timestamp_ms: i64,
}

/// Evidence for one admitted adjustment or its compensating undo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillReceipt {
    /// The admitting effect's `eff-` identifier.
    pub receipt_id: String,
    /// Memory whose stability changed.
    pub memory_id: String,
    /// Memory that triggered the original adjustment.
    pub trigger_id: String,
    /// Exact identity that justified the original join.
    pub shared_entity: String,
    /// Plain-language reason carried in the admitted payload.
    pub reason: String,
    /// Exact stability before this change, in Q32.32 days.
    pub old_stability_q: i64,
    /// Exact stability after this change, in Q32.32 days.
    pub new_stability_q: i64,
    /// Caller-supplied change time in Unix milliseconds.
    pub timestamp_ms: i64,
    /// Original receipt being undone, or `None` for an adjustment.
    pub undo_of: Option<String>,
}

/// Bounds both directions using integer arithmetic. Round lower bounds up
/// so quantization never lets a decrease cross the reciprocal 1.5 floor.
pub(crate) fn bounded_stability(old: i64, requested: i64) -> i64 {
    let year = to_q32_32(365.0);
    let upper = old
        .saturating_add(old / 2)
        .min(old.saturating_add(year))
        .min(to_q32_32(S_MAX));
    let reciprocal_floor = ((i128::from(old) * 2 + 2) / 3) as i64;
    let lower = reciprocal_floor
        .max(old.saturating_sub(year))
        .max(to_q32_32(S_MIN));
    requested.clamp(lower, upper)
}

/// Identity evidence is explicit tags or a complete structural token, never
/// a shared prose word. Entity tags may contain spaces (a place name).
pub(crate) fn records_entity(record: &crate::NodeRecord, entity: &str) -> bool {
    let typed = [
        "person:", "place:", "food:", "file:", "service:", "symbol:", "path:", "commit:", "issue:",
        "url:",
    ]
    .iter()
    .any(|prefix| {
        entity
            .strip_prefix(prefix)
            .is_some_and(|value| !value.trim().is_empty())
    });
    let structural = !entity.contains(char::is_whitespace)
        && (entity.contains('/')
            || entity.contains('\\')
            || entity.contains('.')
            || entity.contains('_')
            || entity.contains("::")
            || (entity.len() >= 7 && entity.bytes().all(|byte| byte.is_ascii_hexdigit())));
    if !typed && !structural {
        return false;
    }
    if record.tags.iter().any(|tag| tag == entity) {
        return true;
    }
    structural
        && record.content.split_whitespace().any(|token| {
            token.trim_matches(|c: char| {
                matches!(
                    c,
                    '`' | '"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            }) == entity
        })
}
