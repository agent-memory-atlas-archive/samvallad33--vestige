//! Re-derivation: replay the log, re-evaluate every GATE with its pinned
//! policy over the prefix, return the verdicts. The sibling kernel's
//! `verify()` compares these bit-for-bit with the stored verdicts.

use crate::inputs::compute_inputs;
use crate::log::EventLog;
use crate::policy::{Policy, gate_verdict};
use crate::record::{GateError, RecordKind, Verdict};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RederiveError {
    /// A GATE cites a proposal seq that holds no PROPOSE.
    UnknownProposal { gate_seq: u64, propose_seq: u64 },
    /// A payload failed to decode.
    Malformed(GateError),
}

impl core::fmt::Display for RederiveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RederiveError::UnknownProposal { gate_seq, propose_seq } => {
                write!(f, "GATE at seq {gate_seq} cites missing proposal {propose_seq}")
            }
            RederiveError::Malformed(e) => write!(f, "malformed record: {e}"),
        }
    }
}

impl std::error::Error for RederiveError {}

/// Re-derive every GATE's verdict under `pinned`, in log order. Uses exactly
/// the same pure path that produced the stored verdicts
/// ([`crate::policy::gate_verdict`] over [`crate::inputs::compute_inputs`]),
/// so for any gate decided under `pinned` the re-derived verdict is
/// bit-for-bit equal to the stored one. Gates recorded under an older policy
/// (their `policy_hash` differs from `pinned`'s) are still re-evaluated under
/// `pinned`; the verdict (and hash) mismatch against the stored record is
/// precisely what `verify()` reports.
///
/// The runtime-bound form is [`crate::GateRuntime::rederive_verdicts`], which
/// supplies the pinned policy the runtime holds; policy content is not
/// embedded in the log (GATE records carry `policy_hash` only), so the
/// standalone function takes the pinned policy explicitly.
pub fn rederive_verdicts(
    log: &dyn EventLog,
    pinned: &Policy,
) -> Result<Vec<(u64, Verdict)>, RederiveError> {
    let all = log.events_before(u64::MAX);
    let mut out = Vec::new();
    for ev in all.iter().filter(|e| e.kind == RecordKind::Gate) {
        let gate = ev.gate().ok_or(RederiveError::Malformed(GateError::MalformedRecord {
            seq: ev.seq,
        }))?;
        let propose_ev = all
            .iter()
            .find(|e| e.seq == gate.propose_seq && e.kind == RecordKind::Propose)
            .ok_or(RederiveError::UnknownProposal {
                gate_seq: ev.seq,
                propose_seq: gate.propose_seq,
            })?;
        let propose = propose_ev
            .propose()
            .ok_or(RederiveError::Malformed(GateError::MalformedRecord { seq: propose_ev.seq }))?;
        let inputs = compute_inputs(log, gate.propose_seq).map_err(RederiveError::Malformed)?;
        out.push((ev.seq, gate_verdict(pinned, &propose, &inputs)));
    }
    Ok(out)
}
