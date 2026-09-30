//! Integration shims for the two externally-owned shapes this crate references
//! by name: `SeqAck` and the generic `EventLog` trait.
//!
//! These local trait shims mirror the integration types field-for-field. When
//! the strata kernel lands, wire its log in as another `EventLog` impl (or
//! re-export its types here); nothing else in this crate changes.

use crate::record::{GateEvent, RecordKind};

/// Append acknowledgement: the sequence number of the appended frame plus the
/// hash of the serialized frame (kind byte || seq LE || payload).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeqAck {
    pub seq: u64,
    pub frame_hash: [u8; 32],
}

/// Minimal append-only log view the gate runtime needs.
///
/// Contract: seqs are allocated densely in append order starting at 0;
/// `events_before(bound)` returns every event with `seq < bound` in seq order.
pub trait EventLog {
    /// All events with `seq < bound`, in seq order.
    fn events_before(&self, bound: u64) -> Vec<GateEvent>;

    /// Append a frame; returns its ack.
    ///
    /// Invariant enforced by [`crate::GateRuntime`]: this is never called with
    /// an `EFFECT` frame except through [`crate::GateRuntime::commit_effect`],
    /// which runs [`crate::admit`] first.
    fn append(&mut self, kind: RecordKind, payload: Vec<u8>) -> SeqAck;

    /// First unallocated seq (provided method; O(n) default, integrations may
    /// override with an O(1) tip counter).
    fn tip(&self) -> u64 {
        u64::try_from(self.events_before(u64::MAX).len()).unwrap_or(u64::MAX)
    }
}

/// In-memory `EventLog` used by tests and as a wiring example.
///
/// Frame hash recipe (byte-exact): `blake3(kind_u8 || seq_le_8 || payload)`.
#[derive(Debug, Default, Clone)]
pub struct MemLog {
    events: Vec<GateEvent>,
}

impl MemLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// All events, in seq order.
    pub fn all(&self) -> &[GateEvent] {
        &self.events
    }
}

impl EventLog for MemLog {
    fn events_before(&self, bound: u64) -> Vec<GateEvent> {
        self.events
            .iter()
            .take_while(|e| e.seq < bound)
            .cloned()
            .collect()
    }

    fn append(&mut self, kind: RecordKind, payload: Vec<u8>) -> SeqAck {
        let seq = u64::try_from(self.events.len()).expect("seq space exhausted");
        let mut hasher = blake3::Hasher::new();
        hasher.update(&[kind.to_u8()]);
        hasher.update(&seq.to_le_bytes());
        hasher.update(&payload);
        let frame_hash: [u8; 32] = *hasher.finalize().as_bytes();
        self.events.push(GateEvent { seq, kind, payload });
        SeqAck { seq, frame_hash }
    }
}

/// blake3 helper shared across the crate.
pub(crate) fn hash32(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}
