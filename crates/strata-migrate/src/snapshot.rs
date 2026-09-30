//! Reading a migrated STRATA log back into typed records.
//!
//! This is the log-level reopen path: consumers (and the migration tests)
//! assert against the log itself via `strata::StrataLog::read_frames`,
//! decoded here into the migration record types.

use strata::StrataLog;
use strata_kernel::checkpoint::Checkpoint;
use strata_kernel::event::ReviewEvent;

use crate::records::*;
use crate::MigrationError;

/// Everything a migration wrote, decoded from log frames.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub genesis: Option<GenesisRecord>,
    pub params: Option<ParamsRecord>,
    pub nodes: Vec<NodeRecord>,
    pub edges: Vec<EdgeRecord>,
    pub reviews: Vec<ReviewEvent>,
    /// Parallel to `reviews`: `reviewed_at_ms` from the required option suffix.
    pub review_times: Vec<Option<i64>>,
    /// Imported FSRS card states (`v4-migrate/2` logs), in log order.
    pub fsrs_states: Vec<FsrsStateRecord>,
    pub tombstones: Vec<TombstoneRecord>,
    pub supersessions: Vec<SupersessionRecord>,
    pub checkpoints: Vec<Checkpoint>,
    /// The final signed MIGRATION_RECEIPT, when the log carries one.
    pub receipt: Option<MigrationReceipt>,
    /// Count of frames whose kind is not part of the migration family
    /// (e.g. gate records sharing the log), plus every frame after the
    /// last receipt.
    pub other_frames: usize,
}

/// Read and decode a full migration snapshot from `log` (from seq 1).
///
/// Frames after the last MIGRATION_RECEIPT are store frames (carried
/// intentions, later writes). Kinds `0x20` and `0x21` there are
/// `STORE_WRITE` and `STORE_CHECKPOINT`, not nodes and edges, so they are
/// counted in `other_frames` and never decoded as migration records.
pub fn read_snapshot(log: &StrataLog) -> Result<Snapshot, MigrationError> {
    let frames = log
        .read_frames(1)
        .map_err(|e| MigrationError::Strata(e.to_string()))?;
    let migration_end = frames
        .iter()
        .rposition(|frame| frame.kind == KIND_MIGRATION_RECEIPT)
        .map_or(frames.len(), |index| index + 1);

    let mut snapshot = Snapshot {
        other_frames: frames.len() - migration_end,
        ..Snapshot::default()
    };
    for frame in frames.into_iter().take(migration_end) {
        let decode = |e: borsh::io::Error| {
            MigrationError::Corrupt(format!(
                "frame seq {} kind {:#04x}: {e}",
                frame.seq, frame.kind
            ))
        };
        match frame.kind {
            KIND_GENESIS => {
                snapshot.genesis = Some(decode_genesis(&frame.payload).map_err(decode)?)
            }
            KIND_PARAMS => snapshot.params = Some(decode_params(&frame.payload).map_err(decode)?),
            KIND_NODE => snapshot
                .nodes
                .push(decode_node(&frame.payload).map_err(decode)?),
            KIND_EDGE => snapshot
                .edges
                .push(decode_edge(&frame.payload).map_err(decode)?),
            KIND_FSRS_REVIEW => {
                snapshot
                    .reviews
                    .push(decode_review(&frame.payload).map_err(decode)?);
                snapshot
                    .review_times
                    .push(decode_reviewed_at_ms(&frame.payload).map_err(decode)?);
            }
            KIND_FSRS_STATE => snapshot
                .fsrs_states
                .push(decode_fsrs_state(&frame.payload).map_err(decode)?),
            KIND_TOMBSTONE => snapshot
                .tombstones
                .push(decode_tombstone(&frame.payload).map_err(decode)?),
            KIND_SUPERSESSION => snapshot
                .supersessions
                .push(decode_supersession(&frame.payload).map_err(decode)?),
            KIND_CHECKPOINT => snapshot
                .checkpoints
                .push(decode_checkpoint(&frame.payload).map_err(decode)?),
            KIND_MIGRATION_RECEIPT => {
                snapshot.receipt = Some(decode_receipt(&frame.payload).map_err(decode)?)
            }
            _ => snapshot.other_frames += 1,
        }
    }
    Ok(snapshot)
}
