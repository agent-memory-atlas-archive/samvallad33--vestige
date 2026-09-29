//! The admission rule — the core invariant. A write with no approving
//! verdict before it in the log is REJECTED at admission.

use crate::inputs::compute_inputs;
use crate::log::{EventLog, SeqAck, hash32};
use crate::policy::{Policy, Veto, evaluate_detailed, first_matching_rule, policy_hash};
use crate::record::{EffectRecord, GateEvent, GateRecord, Propose, RecordKind, Verdict};

/// Why an effect was rejected at admission. Codes are stable and stored
/// inside `GapDetail::OrphanEffect.reason` by [`crate::sweep`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejected {
    /// No PROPOSE at `effect.propose_seq` (or its action_hash does not cover
    /// this effect).
    NoProposal,
    /// No GATE for the proposal in the window `(propose_seq, effect_seq)`,
    /// or the effect does not cite the latest such gate.
    NoGate,
    /// The latest gate's verdict is not Allow (Deny or Hold).
    GateDenied,
    /// The gate was decided under a different policy than the pinned one.
    /// Carries the stale gate's policy_hash.
    GateStale([u8; 32]),
    /// The gate's recorded inputs differ from recomputation over the prefix.
    InputsDrift,
    /// A LESSON_ALARM below the forget floor vetoed a rule that forbids
    /// forgotten lessons.
    ForbiddenByAlarm,
}

impl Rejected {
    pub const fn code(&self) -> u8 {
        match self {
            Rejected::NoProposal => 1,
            Rejected::NoGate => 2,
            Rejected::GateDenied => 3,
            Rejected::GateStale(_) => 4,
            Rejected::InputsDrift => 5,
            Rejected::ForbiddenByAlarm => 6,
        }
    }
}

impl core::fmt::Display for Rejected {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Rejected::NoProposal => write!(f, "no covering proposal"),
            Rejected::NoGate => write!(f, "no admitting gate before effect"),
            Rejected::GateDenied => write!(f, "gate verdict is not Allow"),
            Rejected::GateStale(h) => write!(f, "gate policy {h:02x?} is stale"),
            Rejected::InputsDrift => write!(f, "gate inputs drifted from recomputation"),
            Rejected::ForbiddenByAlarm => write!(f, "forbidden by lesson alarm"),
        }
    }
}

impl std::error::Error for Rejected {}

/// Core admission check, shared by live admission ([`admit`]) and the GAP
/// sweep. `effect_seq` bounds the gate window: only GATE records with
/// `propose_seq < gate_seq < effect_seq` count. `pinned == None` is sweep
/// mode: policy staleness and re-evaluation are skipped (they are
/// admission-time facts, not replay-time ones).
pub(crate) fn admission_check(
    log: &dyn EventLog,
    effect: &EffectRecord,
    effect_seq: u64,
    pinned: Option<&Policy>,
) -> Result<(), Rejected> {
    let all = log.events_before(u64::MAX);

    // 1. Covering proposal.
    let propose = covering_proposal(&all, effect)?;

    // 2. Latest gate for this proposal inside the window.
    let mut candidate: Option<(u64, GateRecord)> = None;
    for ev in all.iter().filter(|e| e.kind == RecordKind::Gate) {
        let Some(gate) = ev.gate() else { continue };
        if gate.propose_seq == effect.propose_seq
            && ev.seq > effect.propose_seq
            && ev.seq < effect_seq
        {
            candidate = Some((ev.seq, gate));
        }
    }
    let (gate_seq, gate) = candidate.ok_or(Rejected::NoGate)?;
    // The effect must cite the operative (latest in-window) gate.
    if gate_seq != effect.gate_seq {
        return Err(Rejected::NoGate);
    }

    // 3. Approving verdict.
    if gate.verdict != Verdict::Allow {
        return Err(Rejected::GateDenied);
    }

    // 4. Policy pin (live admission only).
    if let Some(p) = pinned {
        let pinned_hash = policy_hash(p);
        if gate.policy_hash != pinned_hash {
            return Err(Rejected::GateStale(gate.policy_hash));
        }
    }

    // 5. Inputs drift: recompute over the prefix and require equality.
    let recomputed = compute_inputs(log, effect.propose_seq).map_err(|_| Rejected::InputsDrift)?;
    if recomputed != gate.inputs {
        return Err(Rejected::InputsDrift);
    }

    // 6. Re-evaluate under the pinned policy (live admission only). A
    // forgotten-lesson veto is reported as ForbiddenByAlarm; any other
    // non-Allow re-derivation is GateDenied.
    if let Some(p) = pinned {
        let (v, veto) = evaluate_detailed(p, &propose, &recomputed);
        let v = if v == Verdict::Allow && recomputed.canary_hits > 0 {
            Verdict::Hold
        } else {
            v
        };
        if v != Verdict::Allow {
            return Err(if veto == Some(Veto::ForbiddenLessons) {
                Rejected::ForbiddenByAlarm
            } else {
                Rejected::GateDenied
            });
        }
        // A LESSON_ALARM matching this proposal raised inside the window
        // (gate -> effect) vetoes the write when the winning rule forbids
        // forgotten lessons. Inputs are frozen over the prefix before the
        // PROPOSE, so a later alarm is only visible here.
        let matching_alarm = all.iter().any(|e| {
            e.kind == RecordKind::LessonAlarm
                && e.seq > effect.propose_seq
                && e.seq < effect_seq
                && e.lesson_alarm().is_some_and(|a| {
                    a.propose_seq == effect.propose_seq
                        && a.retention_milli < crate::inputs::FORGET_FLOOR_MILLI
                })
        });
        if matching_alarm
            && first_matching_rule(p, &propose, &recomputed)
                .is_some_and(|r| r.forbid_forgotten_lessons)
        {
            return Err(Rejected::ForbiddenByAlarm);
        }
    }

    Ok(())
}

fn covering_proposal(all: &[GateEvent], effect: &EffectRecord) -> Result<Propose, Rejected> {
    let ev = all
        .iter()
        .find(|e| e.seq == effect.propose_seq && e.kind == RecordKind::Propose)
        .ok_or(Rejected::NoProposal)?;
    let propose = ev.propose().ok_or(Rejected::NoProposal)?;
    if propose.action_hash != effect.action_hash {
        return Err(Rejected::NoProposal);
    }
    Ok(propose)
}

/// The admission rule. Pure check — it never appends. On success returns the
/// projected admission ticket: `seq` is the next free seq (the slot the
/// effect will occupy) and `frame_hash` is blake3(borsh(EffectRecord)).
/// [`crate::GateRuntime::commit_effect`] calls this, then appends and
/// returns the log's real `SeqAck`.
pub fn admit(log: &dyn EventLog, effect: &EffectRecord, pinned: &Policy) -> Result<SeqAck, Rejected> {
    admission_check(log, effect, u64::MAX, Some(pinned))?;
    let bytes = borsh::to_vec(effect).expect("borsh EffectRecord is infallible");
    Ok(SeqAck { seq: log.tip(), frame_hash: hash32(&bytes) })
}
