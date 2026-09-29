//! `GateInputs` — the signed gate context, computed purely from the event-log
//! prefix by [`compute_inputs`]. Deterministic: same prefix, same inputs,
//! bit-for-bit. Digests are blake3 over borsh of the respective id sets —
//! exact equality, no similarity.

use std::collections::{BTreeMap, BTreeSet};

use borsh::{BorshDeserialize, BorshSerialize};

use crate::log::{EventLog, hash32};
use crate::record::{ActionKindCode, EffectRecord, GateError, Propose, RecordKind};

/// Lessons with retention (ms) below this floor count as forgotten and enter
/// `GateInputs.forgotten_lessons` (30 days in milliseconds).
pub const FORGET_FLOOR_MILLI: i64 = 2_592_000_000;

/// Blast radius of a proposal, derived from the prefix's context hypergraph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct BlastRadius {
    /// Size of the transitive closure over referenced ids.
    pub closure_size: u32,
    /// Fixpoint expansion rounds until the closure stopped growing.
    pub tiers: u16,
}

/// The signed gate context. Every field is a pure function of the log prefix
/// strictly before the PROPOSE.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct GateInputs {
    /// blake3(borsh(sorted Vec<u64> of live fact ids)).
    pub live_facts_digest: [u8; 32],
    /// blake3(borsh(sorted Vec<u64> of retired fact ids)).
    pub retired_facts_digest: [u8; 32],
    pub blast_radius: BlastRadius,
    /// (lesson_id, retention_milli) for lessons with retention <
    /// [`FORGET_FLOOR_MILLI`], sorted by lesson_id, latest alarm wins.
    pub forgotten_lessons: Vec<(u64, i64)>,
    /// Number of ALERT records in the prefix (canary trips).
    pub canary_hits: u32,
}

/// Compute the gate inputs for the PROPOSE at `propose_seq`, strictly over the
/// prefix `seq < propose_seq`.
///
/// Fact model: a WRITE effect creates a live fact (fact id = the effect's
/// seq); a RETIRE effect moves the ids listed in its proposal's context from
/// live to retired. Digests are over the sorted id sets — exact, no
/// similarity.
pub fn compute_inputs(log: &dyn EventLog, propose_seq: u64) -> Result<GateInputs, GateError> {
    let around = log.events_before(propose_seq.saturating_add(1));
    let propose_ev = around
        .iter()
        .find(|e| e.seq == propose_seq && e.kind == RecordKind::Propose)
        .ok_or(GateError::UnknownProposal { seq: propose_seq })?;
    let propose: Propose = propose_ev.decode()?;

    // Single ordered pass over the strict prefix.
    let mut prefix_proposes: BTreeMap<u64, Propose> = BTreeMap::new();
    let mut live: BTreeSet<u64> = BTreeSet::new();
    let mut retired: BTreeSet<u64> = BTreeSet::new();
    let mut lessons: BTreeMap<u64, i64> = BTreeMap::new();
    let mut canary_hits: u32 = 0;

    for ev in around.iter().take_while(|e| e.seq < propose_seq) {
        match ev.kind {
            RecordKind::Propose => {
                if let Some(p) = ev.propose() {
                    prefix_proposes.insert(ev.seq, p);
                }
            }
            RecordKind::Effect => {
                if let Some(effect) = ev.effect() {
                    apply_effect(ev.seq, &effect, &prefix_proposes, &mut live, &mut retired);
                }
            }
            RecordKind::LessonAlarm => {
                if let Some(a) = ev.lesson_alarm()
                    && a.retention_milli < FORGET_FLOOR_MILLI
                {
                    lessons.insert(a.lesson_id, a.retention_milli);
                }
            }
            RecordKind::Alert => {
                canary_hits = canary_hits.saturating_add(1);
            }
            RecordKind::Gate | RecordKind::Gap | RecordKind::Canary => {}
        }
    }

    let live_ids: Vec<u64> = live.iter().copied().collect();
    let retired_ids: Vec<u64> = retired.iter().copied().collect();
    let (closure_size, tiers) = closure_over(&prefix_proposes, &propose.context);

    Ok(GateInputs {
        live_facts_digest: digest_ids(&live_ids),
        retired_facts_digest: digest_ids(&retired_ids),
        blast_radius: BlastRadius {
            closure_size,
            tiers,
        },
        forgotten_lessons: lessons.into_iter().collect(),
        canary_hits,
    })
}

fn apply_effect(
    ev_seq: u64,
    effect: &EffectRecord,
    prefix_proposes: &BTreeMap<u64, Propose>,
    live: &mut BTreeSet<u64>,
    retired: &mut BTreeSet<u64>,
) {
    let Some(propose) = prefix_proposes.get(&effect.propose_seq) else {
        return; // orphan effect; sweep reports it
    };
    match propose.action_kind {
        ActionKindCode::WRITE => {
            // The fact id is the creating effect's seq.
            live.insert(ev_seq);
        }
        ActionKindCode::RETIRE => {
            for id in &propose.context {
                if live.remove(id) {
                    retired.insert(*id);
                }
            }
        }
        _ => {}
    }
}

/// blake3 over borsh of the id set (ids must already be sorted).
fn digest_ids(ids: &[u64]) -> [u8; 32] {
    let bytes = borsh::to_vec(ids).expect("borsh Vec<u64> is infallible");
    hash32(&bytes)
}

/// Transitive closure of `seed` over the prefix proposals' context hyperedges
/// (each prior PROPOSE's context is one hyperedge: if any of its ids is in the
/// closure, all of its ids join). `tiers` counts fixpoint expansion rounds.
fn closure_over(prefix_proposes: &BTreeMap<u64, Propose>, seed: &[u64]) -> (u32, u16) {
    let mut set: BTreeSet<u64> = seed.iter().copied().collect();
    if set.is_empty() {
        return (0, 0);
    }
    let edges: Vec<&Vec<u64>> = prefix_proposes
        .values()
        .map(|p| &p.context)
        .filter(|c| !c.is_empty())
        .collect();

    let mut tiers: u16 = 0;
    loop {
        let mut grew = false;
        for edge in &edges {
            if edge.iter().any(|id| set.contains(id)) {
                for id in edge.iter() {
                    if set.insert(*id) {
                        grew = true;
                    }
                }
            }
        }
        if !grew || tiers == u16::MAX {
            break;
        }
        tiers += 1;
    }
    (u32::try_from(set.len()).unwrap_or(u32::MAX), tiers)
}
