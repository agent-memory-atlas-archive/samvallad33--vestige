//! # strata-migrate — one-shot Vestige SQLite → STRATA migration
//!
//! `vestige migrate-to-strata <src> <dst>` empties a Vestige SQLite store
//! into an append-only STRATA log. It is a MIGRATION, not a sync: run it
//! once, keep the SQLite file as the pre-migration backup, and write new
//! memories to STRATA.
//!
//! ## What lands in the log
//!
//! | SQLite source                    | STRATA record                                  |
//! |----------------------------------|------------------------------------------------|
//! | `knowledge_nodes` rows           | `NODE` frames (kernel_id v1 + legacy UUID)      |
//! | `knowledge_nodes.superseded_by`  | `SUPERSESSION` frames                           |
//! | `memory_connections` rows        | `EDGE` frames (link_type VERBATIM, no re-check) |
//! | `fsrs_cards` rows                | `FSRS_REVIEW` frames (kernel `ReviewEvent`)     |
//! | `sync_tombstones` / `deletion_tombstones` | `TOMBSTONE` frames                     |
//! | everything else with rows        | counted in `MigrationReport::skipped_tables`    |
//!
//! The run finishes with a sealed segment (`StrataLog::seal`) and a kernel
//! `Checkpoint` appended as the last frame; `strata_kernel::verify_with_head`
//! plus `StrataLog::verify_tail` must both pass before `verify_passed` is
//! reported true.
//!
//! ## FSRS fold semantics (read before relying on it)
//!
//! SQLite stores only FINAL FSRS state (`reps`, `lapses`, floats) — the
//! review history that produced it is gone. Migration therefore synthesizes
//! a deterministic event series per card: `reps - lapses` rating-3 (good)
//! events followed by `lapses` rating-1 (again) events. The kernel fold
//! reproduces `review_count == reps` and `lapse_count == lapses` EXACTLY;
//! stability/difficulty are recomputed by the deterministic fold and become
//! the new truth (the legacy floats were not reproducible from any log).
//!
//! ## Event seqs
//!
//! `ReviewEvent::event_seq` is the frame seq the event lands at (the kernel
//! requires `event.seq() == record seq`), predicted from the log head before
//! the batch append and asserted against the returned acks afterward.

pub mod records;
pub mod snapshot;
pub mod source;

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use strata::StrataLog;
use strata_kernel::checkpoint::{checkpoint_hash, Checkpoint};
use strata_kernel::event::ReviewEvent;
use strata_kernel::fsrs::ALGO_V1;
use strata_kernel::kernel::Kernel;
use strata_kernel::verify::verify_with_head;
use vestige_core::PortableArchive;

pub use records::{
    EdgeRecord, MigrationMeta, NodeRecord, SupersessionRecord, TombstoneRecord, RECORD_VERSION,
};
pub use snapshot::{read_snapshot, Snapshot};

/// Frames per `append_batch` call: bounds peak memory on huge stores while
/// staying far above the log's own 64-frame group-commit cap.
const BATCH_FRAMES: usize = 1024;

/// Tables this migration maps into STRATA records. Every other source table
/// that contains rows is reported in `skipped_tables`.
const MAPPED_TABLES: &[&str] = &[
    "knowledge_nodes",
    "memory_connections",
    "fsrs_cards",
    "sync_tombstones",
    "deletion_tombstones",
];

/// Everything that can stop a migration. Nothing is ever half-written: the
/// strata log is append-only, so a failed run leaves already-appended frames
/// durable and simply reports the error.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// The source path does not exist.
    #[error("migration source not found: {0}")]
    SourceNotFound(String),
    /// The source exists but is neither a portable archive, a SQLite db,
    /// nor a directory containing `vestige.db`.
    #[error("unsupported migration source: {0}")]
    UnsupportedSource(String),
    /// Opening or exporting a live store failed.
    #[error("source store error: {0}")]
    Source(String),
    /// A source row could not be decoded into a record.
    #[error("corrupt source data: {0}")]
    Corrupt(String),
    /// The strata log refused or failed an operation.
    #[error("strata log error: {0}")]
    Strata(String),
    /// The determinism kernel refused an operation.
    #[error("strata kernel error: {0}")]
    Kernel(String),
    /// Filesystem error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Outcome of one migration run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MigrationReport {
    /// NODE frames appended.
    pub nodes: u64,
    /// EDGE frames appended.
    pub edges: u64,
    /// FSRS_REVIEW frames appended.
    pub fsrs_events: u64,
    /// Source tables that contained rows but have no STRATA mapping.
    pub skipped_tables: Vec<String>,
    /// Whether kernel replay verification AND log tail verification both
    /// passed over the finished log.
    pub verify_passed: bool,
    /// Wall-clock duration of the migration.
    #[serde(serialize_with = "ser_duration_secs", rename = "durationSeconds")]
    pub duration: Duration,
}

fn ser_duration_secs<S: serde::Serializer>(
    duration: &Duration,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_f64(duration.as_secs_f64())
}

/// Migrate from `<src>` (portable archive JSON, SQLite db file, or data
/// directory containing `vestige.db`) into a STRATA log at `<dst>`.
pub fn migrate(source: &Path, strata_dir: &Path) -> Result<MigrationReport, MigrationError> {
    let started = Instant::now();
    let archive = source::load_archive(source)?;
    let report = migrate_archive(&archive, strata_dir)?;
    Ok(MigrationReport {
        duration: started.elapsed(),
        ..report
    })
}

/// Migrate an already-loaded portable archive into a STRATA log at `dst`.
pub fn migrate_archive(
    archive: &PortableArchive,
    strata_dir: &Path,
) -> Result<MigrationReport, MigrationError> {
    std::fs::create_dir_all(strata_dir)?;
    let log = StrataLog::open(strata_dir).map_err(|e| MigrationError::Strata(e.to_string()))?;
    migrate_archive_into(archive, &log)
}

fn migrate_archive_into(
    archive: &PortableArchive,
    log: &StrataLog,
) -> Result<MigrationReport, MigrationError> {
    let mut nodes = 0u64;
    let mut edges = 0u64;
    let mut fsrs_events = 0u64;

    // ---- decode source rows -------------------------------------------
    let (node_records, kernel_ids, supersessions) = extract_nodes(archive)?;
    let edge_records = extract_edges(archive, &kernel_ids)?;
    let tombstones = extract_tombstones(archive)?;
    let skipped_tables = archive
        .tables
        .iter()
        .filter(|t| !t.rows.is_empty() && !MAPPED_TABLES.contains(&t.name.as_str()))
        .map(|t| t.name.clone())
        .collect::<Vec<_>>();

    // ---- append records -------------------------------------------------
    let mut writer = Writer::new(log);
    if log.head().frames_total == 0 {
        let meta = MigrationMeta {
            record_version: RECORD_VERSION,
            archive_format: archive.archive_format.clone(),
            vestige_version: archive.vestige_version.clone(),
            schema_version: archive.schema_version,
        };
        writer.push(records::KIND_MIGRATION_META, borsh::to_vec(&meta))?;
    }
    for record in &node_records {
        writer.push(records::KIND_NODE, borsh::to_vec(record))?;
        nodes += 1;
    }
    for record in &edge_records {
        writer.push(records::KIND_EDGE, borsh::to_vec(record))?;
        edges += 1;
    }
    for record in &tombstones {
        writer.push(records::KIND_TOMBSTONE, borsh::to_vec(record))?;
    }
    for record in &supersessions {
        writer.push(records::KIND_SUPERSESSION, borsh::to_vec(record))?;
    }

    // ---- fsrs_cards -> review-event folds --------------------------------
    // event_seq MUST equal the frame seq, so predict it from the writer's
    // view of the log head and assert the acks below.
    if let Some(table) = source::table(archive, "fsrs_cards") {
        for index in 0..table.rows.len() {
            let row = source::Row::new(table, index);
            let memory_id = row.text("memory_id")?.to_string();
            let kernel_id = *kernel_ids.get(&memory_id).ok_or_else(|| {
                MigrationError::Corrupt(format!(
                    "fsrs_cards row references unknown memory {memory_id}"
                ))
            })?;
            let reps = row.integer_or("reps", 0)?.clamp(0, u32::MAX as i64);
            let lapses = row.integer_or("lapses", 0)?.clamp(0, reps);
            for rating in fsrs_ratings_for(reps, lapses) {
                let event = ReviewEvent {
                    card_id: kernel_id,
                    rating,
                    event_seq: writer.next_seq(),
                };
                writer.push(records::KIND_FSRS_REVIEW, borsh::to_vec(&event))?;
                fsrs_events += 1;
            }
        }
    }
    writer.flush()?;

    // ---- fold + checkpoint ------------------------------------------------
    // Fold ALL review events in the log (an earlier run into the same
    // directory may have contributed some); the checkpoint state covers the
    // whole log, not just this run's events.
    let before_checkpoint = read_snapshot(log)?;
    let kernel = Kernel::<ReviewEvent>::for_version(ALGO_V1)
        .map_err(|e| MigrationError::Kernel(e.to_string()))?;
    let mut state = strata_kernel::state::State::default();
    kernel.apply_all(&mut state, before_checkpoint.reviews.iter());

    let anchor = if writer.appended > 0 {
        let prev = before_checkpoint
            .checkpoints
            .last()
            .map_or([0u8; 32], checkpoint_hash);
        let checkpoint = Checkpoint::seal(ALGO_V1, writer.last_frame_seq, prev, &state);
        let anchor = checkpoint_hash(&checkpoint);
        let bytes = borsh::to_vec(&checkpoint)
            .map_err(|e| MigrationError::Corrupt(format!("borsh encode checkpoint: {e}")))?;
        log.append(records::KIND_CHECKPOINT, &bytes)
            .map_err(|e| MigrationError::Strata(e.to_string()))?;
        anchor
    } else {
        // Nothing new was appended; keep the existing head checkpoint as the
        // anchor instead of duplicating it (duplicate log_seq would trip the
        // verifier's strict checkpoint ordering).
        before_checkpoint
            .checkpoints
            .last()
            .map_or([0u8; 32], checkpoint_hash)
    };

    log.seal()
        .map_err(|e| MigrationError::Strata(e.to_string()))?;

    // ---- verify -------------------------------------------------------------
    let verify_passed = match verify_migrated(log, anchor) {
        Ok(passed) => passed,
        Err(reason) => {
            eprintln!("strata-migrate: verification error: {reason}");
            false
        }
    };

    Ok(MigrationReport {
        nodes,
        edges,
        fsrs_events,
        skipped_tables,
        verify_passed,
        duration: Duration::ZERO,
    })
}

/// Batched append helper: predicts frame seqs from the log head, asserts the
/// returned acks match, and never holds more than `BATCH_FRAMES` frames.
struct Writer<'a> {
    log: &'a StrataLog,
    batch: Vec<(u8, Vec<u8>)>,
    /// Seq the next queued frame will receive.
    next: u64,
    /// Frames appended by this writer so far.
    appended: u64,
    /// Seq of the last frame this writer appended (0 if none).
    last_frame_seq: u64,
}

impl<'a> Writer<'a> {
    fn new(log: &'a StrataLog) -> Self {
        Self {
            log,
            batch: Vec::new(),
            next: log.head().next_seq,
            appended: 0,
            last_frame_seq: 0,
        }
    }

    /// Predicted seq of the next frame.
    fn next_seq(&self) -> u64 {
        self.next
    }

    fn push(
        &mut self,
        kind: u8,
        payload: Result<Vec<u8>, borsh::io::Error>,
    ) -> Result<(), MigrationError> {
        let payload = payload
            .map_err(|e| MigrationError::Corrupt(format!("borsh encode kind {kind:#04x}: {e}")))?;
        if self.batch.len() >= BATCH_FRAMES {
            self.flush()?;
        }
        self.batch.push((kind, payload));
        self.last_frame_seq = self.next;
        self.next += 1;
        self.appended += 1;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), MigrationError> {
        if self.batch.is_empty() {
            return Ok(());
        }
        let expected_base = self.next - self.batch.len() as u64;
        let batch = std::mem::take(&mut self.batch);
        let acks = self
            .log
            .append_batch(batch)
            .map_err(|e| MigrationError::Strata(e.to_string()))?;
        for (offset, ack) in acks.iter().enumerate() {
            if ack.seq != expected_base + offset as u64 {
                return Err(MigrationError::Strata(format!(
                    "append ack mismatch: frame {} acked at seq {}",
                    expected_base + offset as u64,
                    ack.seq
                )));
            }
        }
        Ok(())
    }
}

/// Kernel replay + log tail verification over the finished log.
fn verify_migrated(log: &StrataLog, anchor: [u8; 32]) -> Result<bool, MigrationError> {
    let tail_ok = log.verify_tail().is_ok();
    let snapshot = read_snapshot(log)?;
    let events = snapshot
        .reviews
        .iter()
        .map(|event| {
            let bytes = borsh::to_vec(event)
                .map_err(|e| MigrationError::Corrupt(format!("re-encode review event: {e}")))?;
            Ok((event.event_seq, blake3::hash(&bytes).into(), *event))
        })
        .collect::<Result<Vec<_>, MigrationError>>()?;
    let replay = verify_with_head(&snapshot.checkpoints, Some(anchor), events.into_iter());
    let replay_ok = replay.is_ok();
    if let Err(error) = replay {
        eprintln!("strata-migrate: kernel verify failed: {error}");
    }
    Ok(tail_ok && replay_ok)
}

/// Deterministic rating series reproducing an fsrs_cards row exactly:
/// `reps - lapses` good (3) reviews, then `lapses` again (1) reviews. The
/// fold yields `review_count == reps` and `lapse_count == lapses`; the final
/// phase is Relearning iff the card ever lapsed, else Review.
fn fsrs_ratings_for(reps: i64, lapses: i64) -> Vec<u8> {
    if reps <= 0 {
        return Vec::new();
    }
    let reps = reps.min(u32::MAX as i64);
    let again = lapses.clamp(0, reps) as usize;
    let good = (reps as usize) - again;
    let mut ratings = Vec::with_capacity(good + again);
    ratings.resize(good, 3);
    ratings.resize(good + again, 1);
    ratings
}

/// Decoded `knowledge_nodes`: node records, the legacy→kernel id map (dense,
/// 1-based, source row order), and supersession pointers.
type NodeSet = (
    Vec<NodeRecord>,
    HashMap<String, u64>,
    Vec<SupersessionRecord>,
);

/// Decode `knowledge_nodes` into node records, the legacy→kernel id map
/// (dense, 1-based, source row order), and supersession pointers.
fn extract_nodes(archive: &PortableArchive) -> Result<NodeSet, MigrationError> {
    let Some(table) = source::table(archive, "knowledge_nodes") else {
        return Ok((Vec::new(), HashMap::new(), Vec::new()));
    };
    let has_superseded_by = table.columns.iter().any(|c| c == "superseded_by");

    let mut records = Vec::with_capacity(table.rows.len());
    let mut kernel_ids: HashMap<String, u64> = HashMap::with_capacity(table.rows.len());
    for index in 0..table.rows.len() {
        let row = source::Row::new(table, index);
        let legacy_id = row.text("id")?.to_string();
        let kernel_id = (index as u64) + 1;
        kernel_ids.insert(legacy_id.clone(), kernel_id);
        records.push(NodeRecord {
            record_version: RECORD_VERSION,
            kernel_id,
            content: row.text("content")?.to_string(),
            node_type: row.text("node_type")?.to_string(),
            tags: source::parse_tags(row.opt_text("tags")?),
            created_ms: source::timestamp_ms(row.text("created_at")?)?,
            updated_ms: source::timestamp_ms(row.text("updated_at")?)?,
            last_accessed_ms: source::timestamp_ms(row.text("last_accessed")?)?,
            legacy_id,
        });
    }

    let mut supersessions = Vec::new();
    if has_superseded_by {
        for index in 0..table.rows.len() {
            let row = source::Row::new(table, index);
            if let Some(target) = row.opt_text("superseded_by")? {
                let superseded_legacy_id = row.text("id")?.to_string();
                let superseded_by_legacy_id = target.to_string();
                supersessions.push(SupersessionRecord {
                    record_version: RECORD_VERSION,
                    superseded_kernel_id: kernel_ids
                        .get(&superseded_legacy_id)
                        .copied()
                        .unwrap_or(0),
                    superseded_by_kernel_id: kernel_ids
                        .get(&superseded_by_legacy_id)
                        .copied()
                        .unwrap_or(0),
                    superseded_legacy_id,
                    superseded_by_legacy_id,
                });
            }
        }
    }
    Ok((records, kernel_ids, supersessions))
}

/// Decode `memory_connections` into edge records. FK cascades make dangling
/// edges impossible in a consistent store; a dangling edge in an archive is
/// corruption and stops the migration (fail-stop, never silently dropped).
fn extract_edges(
    archive: &PortableArchive,
    kernel_ids: &HashMap<String, u64>,
) -> Result<Vec<EdgeRecord>, MigrationError> {
    let Some(table) = source::table(archive, "memory_connections") else {
        return Ok(Vec::new());
    };
    let mut records = Vec::with_capacity(table.rows.len());
    for index in 0..table.rows.len() {
        let row = source::Row::new(table, index);
        let source_legacy_id = row.text("source_id")?.to_string();
        let target_legacy_id = row.text("target_id")?.to_string();
        let source_kernel_id = *kernel_ids.get(&source_legacy_id).ok_or_else(|| {
            MigrationError::Corrupt(format!(
                "memory_connections row references unknown source {source_legacy_id}"
            ))
        })?;
        let target_kernel_id = *kernel_ids.get(&target_legacy_id).ok_or_else(|| {
            MigrationError::Corrupt(format!(
                "memory_connections row references unknown target {target_legacy_id}"
            ))
        })?;
        records.push(EdgeRecord {
            record_version: RECORD_VERSION,
            source_kernel_id,
            target_kernel_id,
            strength_q32: strata_kernel::canonical::to_q32_32(row.real("strength")?),
            link_type: row.text("link_type")?.to_string(),
            created_ms: source::timestamp_ms(row.text("created_at")?)?,
            last_activated_ms: source::timestamp_ms(row.text("last_activated")?)?,
            activation_count: row.integer_or("activation_count", 0)? as i32,
            source_legacy_id,
            target_legacy_id,
        });
    }
    Ok(records)
}

/// Decode `sync_tombstones` and `deletion_tombstones`.
fn extract_tombstones(archive: &PortableArchive) -> Result<Vec<TombstoneRecord>, MigrationError> {
    let mut records = Vec::new();
    if let Some(table) = source::table(archive, "sync_tombstones") {
        for index in 0..table.rows.len() {
            let row = source::Row::new(table, index);
            records.push(TombstoneRecord {
                record_version: RECORD_VERSION,
                origin_table: "sync_tombstones".to_string(),
                row_id: row.text("row_id")?.to_string(),
                deleted_ms: source::timestamp_ms(row.text("deleted_at")?)?,
                reason: row.opt_text("reason")?.map(str::to_string),
                node_type: None,
                tags: Vec::new(),
            });
        }
    }
    if let Some(table) = source::table(archive, "deletion_tombstones") {
        for index in 0..table.rows.len() {
            let row = source::Row::new(table, index);
            records.push(TombstoneRecord {
                record_version: RECORD_VERSION,
                origin_table: "deletion_tombstones".to_string(),
                row_id: row.text("memory_id")?.to_string(),
                deleted_ms: source::timestamp_ms(row.text("deleted_at")?)?,
                reason: row.opt_text("reason")?.map(str::to_string),
                node_type: row.opt_text("node_type")?.map(str::to_string),
                tags: source::parse_tags(row.opt_text("tags")?),
            });
        }
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rating_series_reproduces_counters() {
        assert!(fsrs_ratings_for(0, 0).is_empty());
        assert_eq!(fsrs_ratings_for(3, 0), vec![3, 3, 3]);
        assert_eq!(fsrs_ratings_for(5, 2), vec![3, 3, 3, 1, 1]);
        assert_eq!(fsrs_ratings_for(2, 2), vec![1, 1]);
        // lapses clamped to reps
        assert_eq!(fsrs_ratings_for(2, 9), vec![1, 1]);
        // negative reps -> nothing
        assert!(fsrs_ratings_for(-3, 1).is_empty());
    }

    #[test]
    fn rating_series_folds_to_exact_counters() {
        let kernel = Kernel::<ReviewEvent>::for_version(ALGO_V1).unwrap();
        let mut state = strata_kernel::state::State::default();
        let events: Vec<ReviewEvent> = fsrs_ratings_for(6, 2)
            .into_iter()
            .enumerate()
            .map(|(i, rating)| ReviewEvent {
                card_id: 7,
                rating,
                event_seq: i as u64 + 1,
            })
            .collect();
        kernel.apply_all(&mut state, events.iter());
        let card = state.cards.get(&7).unwrap();
        assert_eq!(card.review_count, 6);
        assert_eq!(card.lapse_count, 2);
        assert_eq!(card.phase, strata_kernel::fsrs::CardPhase::Relearning);
    }

    #[test]
    fn rating_series_without_lapses_ends_in_review_phase() {
        let kernel = Kernel::<ReviewEvent>::for_version(ALGO_V1).unwrap();
        let mut state = strata_kernel::state::State::default();
        let events: Vec<ReviewEvent> = fsrs_ratings_for(2, 0)
            .into_iter()
            .enumerate()
            .map(|(i, rating)| ReviewEvent {
                card_id: 1,
                rating,
                event_seq: i as u64 + 1,
            })
            .collect();
        kernel.apply_all(&mut state, events.iter());
        let card = state.cards.get(&1).unwrap();
        assert_eq!(card.review_count, 2);
        assert_eq!(card.lapse_count, 0);
        assert_eq!(card.phase, strata_kernel::fsrs::CardPhase::Review);
    }

    #[test]
    fn timestamps_parse_from_rfc3339_variants() {
        assert_eq!(
            source::timestamp_ms("2026-09-28T12:00:00+00:00").unwrap(),
            1_790_596_800_000
        );
        assert!(source::timestamp_ms("2026-09-28T12:00:00Z").is_ok());
        assert!(source::timestamp_ms("not a date").is_err());
    }

    #[test]
    fn tags_parse_tolerantly() {
        assert_eq!(source::parse_tags(None), Vec::<String>::new());
        assert_eq!(source::parse_tags(Some("[]")), Vec::<String>::new());
        assert_eq!(
            source::parse_tags(Some(r#"["rust","memory"]"#)),
            vec!["rust".to_string(), "memory".to_string()]
        );
        assert_eq!(source::parse_tags(Some("null")), Vec::<String>::new());
        assert_eq!(source::parse_tags(Some("{broken")), Vec::<String>::new());
    }
}
