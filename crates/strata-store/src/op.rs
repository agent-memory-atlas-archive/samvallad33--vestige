//! The store's data-frame payload: one borsh enum covering every mutation the
//! store can land. Each admitted write appends exactly one `STORE_WRITE`
//! frame whose payload is `borsh(StoreOp)` and whose blake3 digest equals the
//! admitting `EFFECT.payload_digest`.

use crate::types::{ConnectionRecord, NodeRecord};
use borsh::io::{Error, ErrorKind, Read};
use borsh::{BorshDeserialize, BorshSerialize};

/// Frame kind: a store data frame (payload = `borsh(StoreOp)`).
///
/// Kinds 1..=7 belong to strata-gate records; store frames start at 32 to
/// leave room for further gate duties.
pub const KIND_STORE_WRITE: u8 = 32;

/// Frame kind: a sealed FSRS checkpoint (payload =
/// `borsh(strata_kernel::Checkpoint)`).
pub const KIND_STORE_CHECKPOINT: u8 = 33;

/// One admitted mutation. The FSRS review event for an ingest is folded
/// deterministically from the frame seq (see `store::StrataStore`), so it
/// carries no explicit event here; explicit reviews use [`StoreOp::ReviewNode`].
///
/// Borsh is manual: [`StoreOp::ReviewNode`] may end with an optional `i64`.
/// Derived borsh would put an `Option` tag in every new frame and reject
/// frames written before the field existed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreOp {
    /// Create or rewrite a node record (ingest and `set_created_at` path).
    /// A brand-new id folds one ingest `ReviewEvent` into the kernel state.
    UpsertNode {
        /// The full record; `id` is the registry key.
        record: NodeRecord,
    },
    /// Append one typed edge and update the forward/reverse indexes.
    SaveEdge {
        /// The edge; `link_type` was vocabulary-validated at write time.
        edge: ConnectionRecord,
    },
    /// Mark `id` as superseded by `superseded_by` (gate action RETIRE).
    SupersedeNode {
        /// The node being retired.
        id: String,
        /// The superseding node.
        superseded_by: String,
    },
    /// Fold an explicit FSRS review for a card.
    ///
    /// Wire, after the `u8` discriminant `3`: `card_id: u64`, `rating: u8`,
    /// then `reviewed_at_ms: i64` **only when set**. No tag byte. A payload
    /// that ends after `rating` is an older frame (or a source with no
    /// review time): `reviewed_at_ms` is `None`. Those bytes are not
    /// rewritten. The `i64` sits in the `STORE_WRITE` payload, so the
    /// existing `payload_blake3` and segment signature already cover it.
    ReviewNode {
        /// Card id (the blake3-derived u64 handle of the node id).
        card_id: u64,
        /// Rating 1..=4 (clamped by the kernel fold if outside).
        rating: u8,
        /// Unix epoch milliseconds of the review. `None` when the frame
        /// predates this field or the source had no last-review time.
        reviewed_at_ms: Option<i64>,
    },
}

impl BorshSerialize for StoreOp {
    fn serialize<W: borsh::io::Write>(&self, writer: &mut W) -> borsh::io::Result<()> {
        match self {
            StoreOp::UpsertNode { record } => {
                0u8.serialize(writer)?;
                record.serialize(writer)
            }
            StoreOp::SaveEdge { edge } => {
                1u8.serialize(writer)?;
                edge.serialize(writer)
            }
            StoreOp::SupersedeNode { id, superseded_by } => {
                2u8.serialize(writer)?;
                id.serialize(writer)?;
                superseded_by.serialize(writer)
            }
            StoreOp::ReviewNode {
                card_id,
                rating,
                reviewed_at_ms,
            } => {
                3u8.serialize(writer)?;
                card_id.serialize(writer)?;
                rating.serialize(writer)?;
                if let Some(ms) = reviewed_at_ms {
                    ms.serialize(writer)?;
                }
                Ok(())
            }
        }
    }
}

impl BorshDeserialize for StoreOp {
    fn deserialize_reader<R: Read>(reader: &mut R) -> borsh::io::Result<Self> {
        match u8::deserialize_reader(reader)? {
            0 => Ok(StoreOp::UpsertNode {
                record: NodeRecord::deserialize_reader(reader)?,
            }),
            1 => Ok(StoreOp::SaveEdge {
                edge: ConnectionRecord::deserialize_reader(reader)?,
            }),
            2 => Ok(StoreOp::SupersedeNode {
                id: String::deserialize_reader(reader)?,
                superseded_by: String::deserialize_reader(reader)?,
            }),
            3 => Ok(StoreOp::ReviewNode {
                card_id: u64::deserialize_reader(reader)?,
                rating: u8::deserialize_reader(reader)?,
                reviewed_at_ms: read_optional_i64(reader)?,
            }),
            other => Err(Error::new(
                ErrorKind::InvalidData,
                format!("unknown StoreOp discriminant {other}"),
            )),
        }
    }
}

/// Missing tail = unset. Eight bytes = one signed millisecond timestamp.
/// Any other remainder is a torn payload, not an old frame.
fn read_optional_i64<R: Read>(reader: &mut R) -> borsh::io::Result<Option<i64>> {
    let mut buf = [0u8; 8];
    let mut filled = 0;
    while filled < 8 {
        match reader.read(&mut buf[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "reviewed_at_ms truncated",
                ));
            }
            Ok(n) => filled += n,
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
    Ok(Some(i64::from_le_bytes(buf)))
}
