//! The strata-gate `EventLog` adapter over a `StrataLog`.
//!
//! The gate runtime sees ONLY gate-record frames (kinds 1..=7); store data
//! frames (32/33) are invisible to it. Because store frames interleave with
//! gate frames in the log, the adapter maintains its OWN dense, 0-based seq
//! space over gate frames alone — exactly the contract `EventLog` documents
//! ("seqs are allocated densely in append order starting at 0"). All gate
//! invariants (propose < gate < effect windows, blast-radius fact ids,
//! `ReadNoReceipt` duty checks) are consistent within that space.
//!
//! The frame cache is shared across clones (single-writer v1), so a runtime
//! constructed per mutation sees every prior append without re-reading the
//! log. The cache is `Arc<Mutex>`-backed so `StrataStore` is `Send` and the
//! MCP runtime can hold it. Callers still admit one write at a time.

use std::sync::{Arc, Mutex};

use strata::StrataLog;
use strata_gate::record::{GateEvent, RecordKind};
use strata_gate::{EventLog, SeqAck};

/// `EventLog` implementation backed by a durable `StrataLog`.
#[derive(Debug, Clone)]
pub struct StrataEventLog {
    log: StrataLog,
    /// Gate frames only, in append order; index == gate seq.
    cache: Arc<Mutex<Vec<GateEvent>>>,
}

impl StrataEventLog {
    /// Build the adapter view over an opened log, caching existing gate
    /// frames once.
    pub fn new(log: StrataLog) -> Result<Self, strata::StrataError> {
        let frames = log.read_frames(1)?;
        let mut cache = frames
            .into_iter()
            .filter_map(|f| {
                let kind = RecordKind::from_u8(f.kind)?;
                // Log seqs are 1-based; gate seqs are dense from 0.
                Some(GateEvent {
                    seq: 0,
                    kind,
                    payload: f.payload,
                })
            })
            .collect::<Vec<_>>();
        // Assign the dense gate seqs after filtering (store frames must not
        // open holes in the gate seq space).
        for (idx, ev) in cache.iter_mut().enumerate() {
            ev.seq = idx as u64;
        }
        Ok(Self {
            log,
            cache: Arc::new(Mutex::new(cache)),
        })
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, Vec<GateEvent>> {
        self.cache
            .lock()
            .unwrap_or_else(|e| panic!("strata-store: gate cache poisoned: {e}"))
    }

    /// The underlying durable log (shared handle).
    pub fn log(&self) -> &StrataLog {
        &self.log
    }

    /// Number of gate frames visible to the gate runtime.
    pub fn gate_frame_count(&self) -> u64 {
        self.cache().len() as u64
    }
}

impl EventLog for StrataEventLog {
    fn events_before(&self, bound: u64) -> Vec<GateEvent> {
        self.cache()
            .iter()
            .take_while(|e| e.seq < bound)
            .cloned()
            .collect()
    }

    fn append(&mut self, kind: RecordKind, payload: Vec<u8>) -> SeqAck {
        let ack = self
            .log
            .append(kind.to_u8(), &payload)
            .unwrap_or_else(|e| panic!("strata-store: durable append failed (fail-stop): {e}"));
        let mut cache = self.cache();
        let gate_seq = cache.len() as u64;
        cache.push(GateEvent {
            seq: gate_seq,
            kind,
            payload,
        });
        SeqAck {
            seq: gate_seq,
            frame_hash: ack.frame_hash,
        }
    }

    fn tip(&self) -> u64 {
        self.cache().len() as u64
    }
}
