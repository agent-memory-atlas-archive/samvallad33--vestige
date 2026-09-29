//! Wire records the migration writes into the STRATA log.
//!
//! Kind-byte allocation: `strata-gate` owns codes `1..=7` and `0` is reserved
//! ("unknown") by the log layer. The migration family starts at `0x1F`
//! (`MIGRATION_META`) so it cannot collide with gate records in a shared log.
//!
//! Every payload begins with a little-endian `u16` `record_version` so a
//! future migration format can evolve without kind renegotiation. Version 1
//! is the layout documented here.
//!
//! The `FSRS_REVIEW` payload is NOT a bespoke struct: it is the kernel's own
//! `strata_kernel::event::ReviewEvent` borsh encoding, and the `CHECKPOINT`
//! payload is `strata_kernel::checkpoint::Checkpoint` verbatim. Migration
//! records reuse kernel wire types wherever one exists — one encoding per
//! concept, forever.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_kernel::checkpoint::Checkpoint;
use strata_kernel::event::ReviewEvent;

/// First frame of a fresh migration log: provenance for everything after it.
pub const KIND_MIGRATION_META: u8 = 0x1F;
/// One `knowledge_nodes` row.
pub const KIND_NODE: u8 = 0x20;
/// One `memory_connections` row (typed edge, legacy link_type verbatim).
pub const KIND_EDGE: u8 = 0x21;
/// One synthesized review event (payload = kernel `ReviewEvent`).
pub const KIND_FSRS_REVIEW: u8 = 0x22;
/// One `sync_tombstones` or `deletion_tombstones` row.
pub const KIND_TOMBSTONE: u8 = 0x23;
/// One supersession lineage pointer (`knowledge_nodes.superseded_by`).
pub const KIND_SUPERSESSION: u8 = 0x24;
/// Sealed fold checkpoint (payload = kernel `Checkpoint`).
pub const KIND_CHECKPOINT: u8 = 0x25;

/// Current wire version of every migration record below.
pub const RECORD_VERSION: u16 = 1;

/// Provenance header written as the first frame of a migration.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct MigrationMeta {
    pub record_version: u16,
    /// Source archive format identifier (expected `vestige.portable.v1`).
    pub archive_format: String,
    /// Vestige version that produced the source archive.
    pub vestige_version: String,
    /// SQLite schema version of the source database.
    pub schema_version: u32,
}

/// A migrated knowledge node. `kernel_id` is the dense 1-based STRATA
/// identity assigned in source row order; `legacy_id` keeps the original
/// UUID so nothing is lost and back-references stay possible.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct NodeRecord {
    pub record_version: u16,
    pub legacy_id: String,
    pub kernel_id: u64,
    pub content: String,
    pub node_type: String,
    pub tags: Vec<String>,
    /// Milliseconds since the Unix epoch.
    pub created_ms: i64,
    pub updated_ms: i64,
    pub last_accessed_ms: i64,
}

/// A migrated typed edge. `link_type` passes the legacy vocabulary through
/// VERBATIM: vocabulary enforcement is an admission-time concern for NEW
/// writes; migration must never rewrite or reject history.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct EdgeRecord {
    pub record_version: u16,
    pub source_kernel_id: u64,
    pub target_kernel_id: u64,
    pub source_legacy_id: String,
    pub target_legacy_id: String,
    /// Legacy link type, verbatim (semantic/temporal/…/user-defined).
    pub link_type: String,
    /// Edge strength quantized to Q32.32 (`strata_kernel::canonical`).
    pub strength_q32: i64,
    pub created_ms: i64,
    pub last_activated_ms: i64,
    pub activation_count: i32,
}

/// A migrated tombstone row (`sync_tombstones` or `deletion_tombstones`).
/// The two source tables have different column sets; fields only present in
/// `deletion_tombstones` are `None`/empty for sync tombstones.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct TombstoneRecord {
    pub record_version: u16,
    /// Source table the tombstone came from.
    pub origin_table: String,
    /// Tombstoned row id (memory id for deletion tombstones).
    pub row_id: String,
    pub deleted_ms: i64,
    pub reason: Option<String>,
    /// deletion_tombstones only.
    pub node_type: Option<String>,
    /// deletion_tombstones only.
    pub tags: Vec<String>,
}

/// A supersession lineage pointer from the bitemporal store
/// (`knowledge_nodes.superseded_by`): A was superseded by B, and both ids are
/// kept — legacy strings always, kernel ids when both endpoints were mapped
/// (0 otherwise).
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct SupersessionRecord {
    pub record_version: u16,
    pub superseded_legacy_id: String,
    pub superseded_by_legacy_id: String,
    pub superseded_kernel_id: u64,
    pub superseded_by_kernel_id: u64,
}

/// Decode a `KIND_MIGRATION_META` payload.
pub fn decode_meta(payload: &[u8]) -> Result<MigrationMeta, borsh::io::Error> {
    MigrationMeta::try_from_slice(payload)
}

/// Decode a `KIND_NODE` payload.
pub fn decode_node(payload: &[u8]) -> Result<NodeRecord, borsh::io::Error> {
    NodeRecord::try_from_slice(payload)
}

/// Decode a `KIND_EDGE` payload.
pub fn decode_edge(payload: &[u8]) -> Result<EdgeRecord, borsh::io::Error> {
    EdgeRecord::try_from_slice(payload)
}

/// Decode a `KIND_FSRS_REVIEW` payload (kernel wire type).
pub fn decode_review(payload: &[u8]) -> Result<ReviewEvent, borsh::io::Error> {
    ReviewEvent::try_from_slice(payload)
}

/// Decode a `KIND_TOMBSTONE` payload.
pub fn decode_tombstone(payload: &[u8]) -> Result<TombstoneRecord, borsh::io::Error> {
    TombstoneRecord::try_from_slice(payload)
}

/// Decode a `KIND_SUPERSESSION` payload.
pub fn decode_supersession(payload: &[u8]) -> Result<SupersessionRecord, borsh::io::Error> {
    SupersessionRecord::try_from_slice(payload)
}

/// Decode a `KIND_CHECKPOINT` payload (kernel wire type).
pub fn decode_checkpoint(payload: &[u8]) -> Result<Checkpoint, borsh::io::Error> {
    Checkpoint::try_from_slice(payload)
}
