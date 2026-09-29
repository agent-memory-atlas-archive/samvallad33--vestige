//! GAP sweep — structural violations over the log. Returns the GAP records;
//! the integrator (or [`crate::GateRuntime::commit_gap`]) lands them.

use std::collections::{BTreeMap, BTreeSet};

use crate::admit::admission_check;
use crate::log::EventLog;
use crate::record::{
    ActionKindCode, DutyKind, GapDetail, GapRecord, Propose, RecordKind,
};

/// Sweep the log for structural violations:
///
/// - **OrphanEffect** — an EFFECT whose admission check over the window
///   before it fails (no covering proposal, no admitting gate, non-Allow
///   latest gate, or inputs drift). Policy staleness is deliberately NOT
///   re-checked here: a policy rotation retro-stales gates at admission
///   time, and re-flagging historically admitted effects would be
///   anachronistic.
/// - **ReadNoReceipt** — a PROPOSE context id referencing a fact with no
///   producing WRITE effect earlier in the log (fact ids are WRITE-effect
///   seqs).
/// - **DutySeqGap** — a hole in the duty sequence counters. Source 0 is the
///   single append path; the expected counter is dense 0,1,2,...
pub fn sweep(log: &dyn EventLog) -> Vec<GapRecord> {
    let all = log.events_before(u64::MAX);
    let mut gaps = Vec::new();

    // Missing duty sequence numbers (per-source u64 counters; source 0).
    let mut expected: u64 = 0;
    for ev in &all {
        if ev.seq != expected {
            gaps.push(GapRecord {
                duty: DutyKind::DutySeqGap,
                detail: GapDetail::DutySeqGap { source: 0, expected, found: ev.seq },
            });
        }
        expected = ev.seq.saturating_add(1);
    }

    // Reads without receipts + orphan effects, one ordered pass.
    let mut created: BTreeSet<u64> = BTreeSet::new();
    let mut proposes: BTreeMap<u64, Propose> = BTreeMap::new();
    for ev in &all {
        match ev.kind {
            RecordKind::Propose => {
                if let Some(p) = ev.propose() {
                    for id in &p.context {
                        if !created.contains(id) {
                            gaps.push(GapRecord {
                                duty: DutyKind::ReadNoReceipt,
                                detail: GapDetail::ReadNoReceipt {
                                    reader_seq: ev.seq,
                                    dangling_id: *id,
                                },
                            });
                        }
                    }
                    proposes.insert(ev.seq, p);
                }
            }
            RecordKind::Effect => {
                if let Some(effect) = ev.effect() {
                    if let Err(reason) = admission_check(log, &effect, ev.seq, None) {
                        gaps.push(GapRecord {
                            duty: DutyKind::OrphanEffect,
                            detail: GapDetail::OrphanEffect {
                                effect_seq: ev.seq,
                                propose_seq: effect.propose_seq,
                                reason: reason.code(),
                            },
                        });
                    }
                    if proposes
                        .get(&effect.propose_seq)
                        .is_some_and(|p| p.action_kind == ActionKindCode::WRITE)
                    {
                        created.insert(ev.seq);
                    }
                }
            }
            _ => {}
        }
    }

    gaps
}
