//! # strata-migrate — one-shot Vestige SQLite → STRATA migration
//!
//! `vestige migrate-to-strata --from <src> [--to <dst>]` reads a Vestige
//! SQLite store STRICTLY READ-ONLY and replays it into an append-only
//! STRATA log through a signed MIGRATION_RECEIPT. It is a MIGRATION, not a
//! sync: run it once, keep the SQLite file (byte-identical, never opened
//! read-write) as the pre-migration record, and write new memories to
//! STRATA.
//!
//! ## What lands in the log
//!
//! | SQLite source                    | STRATA record                                  |
//! |----------------------------------|------------------------------------------------|
//! | `GENESIS` / `PARAMS v4-migrate/1`| provenance + parameter frames on a fresh log   |
//! | `knowledge_nodes` rows           | `NODE` frames (kernel_id v1 + legacy UUID)      |
//! | V40 `walk_receipts` rows         | reference `NODE` frames tagged migrated_from_v4 |
//! | `knowledge_nodes.superseded_by`  | `SUPERSESSION` frames                           |
//! | `memory_connections` rows        | `EDGE` frames (8-type vocabulary; legacy types  |
//! |                                  | become `derived_from{legacy_inferred=1}`)       |
//! | `fsrs_cards` rows                | `FSRS_REVIEW` frames (kernel `ReviewEvent`)     |
//! | `sync_tombstones` / `deletion_tombstones` | `TOMBSTONE` frames                     |
//! | everything else with rows        | counted in `MigrationReport::skipped_tables`    |
//! | final frame                      | signed `MIGRATION_RECEIPT` (kind 46)            |
//!
//! The run re-hashes the source after the replay and refuses to seal if a
//! single byte changed (the reader is read-only at the SQLite VFS level,
//! so this is belt-and-suspenders). `strata_kernel::verify_with_head` plus
//! `StrataLog::verify_tail` must both pass before `verify_passed` is
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
//!
//! ## Determinism
//!
//! Every timestamp in the log comes from the source rows (a replay clock:
//! the migration never reads the wall clock into hashed state), the log is
//! opened seeded from the source BLAKE3 unless a seed is pinned, and all
//! hashed collections are ordered Vecs. Two runs over one source produce
//! byte-identical segments.

pub mod records;
pub mod snapshot;
#[cfg(feature = "sqlite-source")]
pub mod source;

use std::path::Path;
use std::time::Duration;

#[cfg(feature = "sqlite-source")]
use std::collections::HashMap;
#[cfg(feature = "sqlite-source")]
use std::time::Instant;
#[cfg(feature = "sqlite-source")]
use strata::StrataLog;
#[cfg(feature = "sqlite-source")]
use strata_kernel::checkpoint::{checkpoint_hash, Checkpoint};
#[cfg(feature = "sqlite-source")]
use strata_kernel::event::ReviewEvent;
#[cfg(feature = "sqlite-source")]
use strata_kernel::fsrs::ALGO_V1;
#[cfg(feature = "sqlite-source")]
use strata_kernel::kernel::Kernel;
#[cfg(feature = "sqlite-source")]
use strata_kernel::verify::verify_with_head;
#[cfg(feature = "sqlite-source")]
use vestige_core::storage::PortableArchive;
#[cfg(feature = "sqlite-source")]
use vestige_core::storage::PortableValue;

pub use records::{
    EdgeRecord, GenesisRecord, MigrationReceipt, NodeRecord, ParamsRecord, ReceiptBody,
    SupersessionRecord, TombstoneRecord, KIND_MIGRATION_RECEIPT, RECEIPT_SIGNING_KEY_ID,
    RECORD_VERSION,
};
pub use snapshot::{read_snapshot, Snapshot};

/// Parameter set implemented by this migrator. Written as the `PARAMS`
/// frame on a fresh log.
pub const PARAMS_ID: &str = "v4-migrate/1";

/// Frames per `append_batch` call: bounds peak memory on huge stores while
/// staying far above the log's own 64-frame group-commit cap.
#[cfg(feature = "sqlite-source")]
const BATCH_FRAMES: usize = 1024;

/// Tables this migration maps into STRATA records. Every other source table
/// that contains rows is reported in `skipped_tables`.
#[cfg(feature = "sqlite-source")]
const MAPPED_TABLES: &[&str] = &[
    "knowledge_nodes",
    "memory_connections",
    "fsrs_cards",
    "sync_tombstones",
    "deletion_tombstones",
];

/// The only edge vocabulary STRATA carries (H4). Any legacy `link_type`
/// outside this set migrates as `derived_from` with `legacy_inferred = 1`.
pub const STRATA_EDGE_VOCABULARY: [&str; 8] = [
    "touched",
    "anchored_to",
    "derived_from",
    "supersedes",
    "corrects",
    "closed_by",
    "projected_to",
    "evidence_of",
];

/// Options for one migration run.
#[derive(Debug, Clone, Default)]
pub struct MigrateOptions {
    /// Read and verify the source, report counts, write nothing.
    pub dry_run: bool,
    /// Allow migrating a source with a non-empty `-wal` by snapshot-copying
    /// db + sidecars to a scratch directory first. The original is still
    /// never modified.
    pub accept_wal_snapshot: bool,
    /// Pin the strata log seed (signing key + segment ids derive from it).
    /// `None` derives the seed from the source BLAKE3, which already makes
    /// two runs over one source byte-identical.
    pub seed: Option<[u8; 32]>,
}

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
    /// Opening or reading the source failed.
    #[error("source store error: {0}")]
    Source(String),
    /// The source has a non-empty `-wal`; rerun with `--accept-wal-snapshot`
    /// to migrate from a consistent snapshot copy.
    #[error("refusing non-empty WAL {path}; rerun with --accept-wal-snapshot")]
    WalPresent {
        /// Path of the offending `-wal` file.
        path: String,
    },
    /// The `receipt_envelopes` hash chain is broken. The message names the
    /// first break; nothing is written.
    #[error("broken receipt_envelopes hash chain: {0}")]
    BrokenEnvelopeChain(String),
    /// The destination directory is not empty and holds no receipt for this
    /// source. A killed run must be cleared by the operator, never extended.
    #[error(
        "destination {path} is not empty; a killed run must be removed by hand, never extended"
    )]
    DestinationNotEmpty {
        /// The refusing destination directory.
        path: String,
    },
    /// The source's BLAKE3 changed while frames were being appended. The
    /// log is incomplete; the destination is poisoned (further runs refuse).
    #[error("source changed during migration (before {before}, after {after}); the destination log is incomplete and must not be trusted")]
    SourceTampered {
        /// BLAKE3 taken before the first read.
        before: String,
        /// BLAKE3 taken before the seal.
        after: String,
    },
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
    /// NODE frames appended (knowledge_nodes + walk_receipt references).
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
    /// `node_embeddings` rows whose vector values were never read.
    pub dropped_vectors: u64,
    /// Last verified `receipt_envelopes` entry digest (empty = none).
    pub envelope_head: String,
    /// BLAKE3 hex of the source files (identical before and after; the run
    /// aborts otherwise). Empty for portable-archive sources.
    pub source_blake3: String,
    /// The sealed receipt digest, hex (`None` under `--dry-run`).
    pub receipt_digest: Option<String>,
    /// Whether the sealed receipt verifies (checksum + ed25519 signature).
    pub receipt_verified: bool,
    /// True when the destination already carried a receipt for this source:
    /// nothing was written; the existing receipt is echoed.
    pub idempotent_reuse: bool,
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

/// Migrate from `<src>` into a STRATA log at `<dst>`.
///
/// The SQLite reader is compiled only with the `sqlite-source` feature.
/// The default 4.0 binaries do not enable it and do not link rusqlite.
#[cfg(not(feature = "sqlite-source"))]
pub fn migrate(source: &Path, strata_dir: &Path) -> Result<MigrationReport, MigrationError> {
    let _ = (source, strata_dir);
    Err(MigrationError::Source(
        "built without the SQLite source reader; this binary does not link rusqlite".into(),
    ))
}

/// Migrate with explicit options. See [`MigrateOptions`].
#[cfg(not(feature = "sqlite-source"))]
pub fn migrate_with_options(
    source: &Path,
    strata_dir: &Path,
    options: MigrateOptions,
) -> Result<MigrationReport, MigrationError> {
    let _ = options;
    migrate(source, strata_dir)
}

#[cfg(feature = "sqlite-source")]
include!("migrate_impl.rs");
