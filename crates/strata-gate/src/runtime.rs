//! `GateRuntime` — the writer. The ONLY append path for effects is
//! [`GateRuntime::commit_effect`], which runs [`crate::admit`] first.

use std::collections::BTreeSet;

use borsh::BorshSerialize;

use crate::admit::{Rejected, admit};
use crate::inputs::compute_inputs;
use crate::log::{EventLog, SeqAck};
use crate::policy::{Policy, gate_verdict};
use crate::record::{
    CanaryRecord, EffectRecord, GateError, GateRecord, GateEvent, LessonAlarmRecord, Propose,
    RecordKind, Verdict,
};

/// Deterministic gate runtime over an arbitrary [`EventLog`].
///
/// The runtime owns the pinned policy (`pin_policy` rotates it; a GATE's
/// recorded `policy_hash` pins the policy it was decided under, so rotating
/// the policy retro-stales earlier Allow gates — admission then rejects with
/// [`Rejected::GateStale`]).
pub struct GateRuntime<L: EventLog> {
    log: L,
    pinned: Policy,
}

fn to_vec<T: BorshSerialize>(value: &T) -> Vec<u8> {
    borsh::to_vec(value).expect("borsh serialization of gate records is infallible")
}

impl<L: EventLog> GateRuntime<L> {
    pub fn new(log: L, pinned: Policy) -> Self {
        Self { log, pinned }
    }

    /// Read-only log view.
    pub fn log(&self) -> &L {
        &self.log
    }

    /// Consume the runtime, handing the log back to the integrator.
    pub fn into_inner(self) -> L {
        self.log
    }

    /// Rotate the pinned policy. Earlier Allow gates become stale at
    /// admission (their recorded policy_hash no longer matches).
    pub fn pin_policy(&mut self, policy: Policy) {
        self.pinned = policy;
    }

    pub fn pinned(&self) -> &Policy {
        &self.pinned
    }

    /// Append a PROPOSE. If its context references any planted canary id, the
    /// runtime appends an ALERT per tripped canary (sorted by canary id)
    /// immediately after; later gates then see `canary_hits > 0` and
    /// Allow-verdicts are clamped to Hold ([`crate::policy::gate_verdict`]).
    pub fn commit_propose(&mut self, propose: Propose) -> SeqAck {
        let planted: BTreeSet<u64> = self
            .log
            .events_before(u64::MAX)
            .iter()
            .filter(|e| e.kind == RecordKind::Canary)
            .filter_map(|e| e.canary().map(|c| c.canary_id))
            .collect();
        let tripped: BTreeSet<u64> =
            propose.context.iter().copied().filter(|id| planted.contains(id)).collect();

        let ack = self.log.append(RecordKind::Propose, to_vec(&propose));
        for canary_id in tripped {
            let alert = crate::record::AlertRecord { canary_id, reader_seq: ack.seq };
            self.log.append(RecordKind::Alert, to_vec(&alert));
        }
        ack
    }

    /// Evaluate the pinned policy over the prefix before `propose_seq` and
    /// append the GATE record (change+verdict land as one signed record).
    pub fn commit_gate(&mut self, propose_seq: u64) -> Result<SeqAck, GateError> {
        let propose = load_propose(&self.log, propose_seq)?;
        let gate_inputs = compute_inputs(&self.log, propose_seq)?;
        let verdict = gate_verdict(&self.pinned, &propose, &gate_inputs);
        let record = GateRecord {
            propose_seq,
            verdict,
            policy_hash: self.pinned.policy_hash(),
            inputs: gate_inputs,
        };
        Ok(self.log.append(RecordKind::Gate, to_vec(&record)))
    }

    /// The ONLY append path for effects: [`admit`] first, append second. A
    /// write with no approving verdict before it in the log never lands.
    pub fn commit_effect(&mut self, effect: EffectRecord) -> Result<SeqAck, Rejected> {
        admit(&self.log, &effect, &self.pinned)?;
        Ok(self.log.append(RecordKind::Effect, to_vec(&effect)))
    }

    /// Pure admission check (no append).
    pub fn admit(&self, effect: &EffectRecord) -> Result<SeqAck, Rejected> {
        admit(&self.log, effect, &self.pinned)
    }

    pub fn commit_lesson_alarm(&mut self, alarm: LessonAlarmRecord) -> SeqAck {
        self.log.append(RecordKind::LessonAlarm, to_vec(&alarm))
    }

    /// Plant a canary. Any later record referencing `canary_id` (via a
    /// PROPOSE context) trips an ALERT and auto-holds descendants.
    pub fn commit_canary(&mut self, canary_id: u64) -> SeqAck {
        let record = CanaryRecord { canary_id };
        self.log.append(RecordKind::Canary, to_vec(&record))
    }

    /// Append a GAP record (sweep results land in the log via this path).
    pub fn commit_gap(&mut self, gap: crate::record::GapRecord) -> SeqAck {
        self.log.append(RecordKind::Gap, to_vec(&gap))
    }

    /// Convenience: the latest recorded gate verdict for a proposal, if any.
    pub fn latest_gate(&self, propose_seq: u64) -> Option<(u64, Verdict)> {
        self.log
            .events_before(u64::MAX)
            .iter()
            .rev()
            .find_map(|e| e.gate().filter(|g| g.propose_seq == propose_seq).map(|g| (e.seq, g.verdict)))
    }

    /// Re-derive every GATE's verdict under the pinned policy (see
    /// [`crate::rederive_verdicts`]).
    pub fn rederive_verdicts(&self) -> Result<Vec<(u64, Verdict)>, crate::rederive::RederiveError> {
        crate::rederive::rederive_verdicts(&self.log, &self.pinned)
    }

    /// Structural sweep (see [`crate::sweep`]).
    pub fn sweep(&self) -> Vec<crate::record::GapRecord> {
        crate::sweep::sweep(&self.log)
    }
}

fn load_propose(log: &dyn EventLog, propose_seq: u64) -> Result<Propose, GateError> {
    let around: Vec<GateEvent> = log.events_before(propose_seq.saturating_add(1));
    let ev = around
        .iter()
        .find(|e| e.seq == propose_seq && e.kind == RecordKind::Propose)
        .ok_or(GateError::UnknownProposal { seq: propose_seq })?;
    ev.decode()
}
