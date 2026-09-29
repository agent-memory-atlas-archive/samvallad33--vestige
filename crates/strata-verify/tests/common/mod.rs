//! Shared builder for tamper tests: a real gate log (via `GateRuntime` over
//! `MemLog`) and a real kernel log (via the version-pinned fold), sealed
//! into the on-disk layout `strata_verify::layout::write_store` emits.

use strata_gate::policy::{ANY_KIND, Rule, WILDCARD_PREFIX};
use strata_gate::record::{EffectRecord, Propose, Verdict};
use strata_gate::{GateRuntime, Policy};
use strata_kernel::checkpoint::{Checkpoint, checkpoint_hash};
use strata_kernel::event::ReviewEvent;
use strata_kernel::fsrs::ALGO_V1;
use strata_kernel::kernel::Kernel;
use strata_kernel::state::State;
use strata_verify::layout::{
    GateFrame, KernelRecord, MAGIC_BYTES, StoreFiles, encode_gate_frame, encode_kernel_frame,
    gate_frame_hash, write_store,
};

/// The permissive pinned policy used by the harness stores: one rule,
/// any subject, unbounded blast radius, Allow.
pub fn permissive_policy() -> Policy {
    Policy {
        rules: vec![Rule {
            match_kind: ANY_KIND,
            match_params_hash_prefix: WILDCARD_PREFIX,
            max_blast_radius: u32::MAX,
            forbid_forgotten_lessons: false,
            require_human: false,
            verdict: Verdict::Allow,
        }],
    }
}

/// A built store in memory, before it hits disk.
pub struct BuiltStore {
    /// Files ready for `write_store`.
    pub files: StoreFiles,
    /// Seqs of the PROPOSE records (for forging cites).
    pub propose_seqs: Vec<u64>,
    /// Seqs of the EFFECT records.
    pub effect_seqs: Vec<u64>,
    /// Seqs of the GATE records.
    pub gate_seqs: Vec<u64>,
}

/// Drive `writes` gated WRITE proposals (propose -> gate -> effect) plus one
/// RETIRE, and fold `reviews` ReviewEvents through the v1 kernel, sealing a
/// genesis checkpoint and one checkpoint at the log tail.
pub fn build_store(writes: usize, reviews: &[(u64, u8)]) -> BuiltStore {
    let policy = permissive_policy();
    let mut runtime = GateRuntime::new(strata_gate::MemLog::new(), policy.clone());

    let mut propose_seqs = Vec::new();
    let mut gate_seqs = Vec::new();
    let mut effect_seqs = Vec::new();

    for i in 0..writes as u64 {
        let propose = Propose {
            action_hash: [i as u8; 32],
            action_kind: strata_gate::ActionKindCode::WRITE,
            params_hash: [0xff; 32],
            context: vec![],
        };
        let ack = runtime.commit_propose(propose);
        propose_seqs.push(ack.seq);
        let gate = runtime.commit_gate(ack.seq).expect("gate commits");
        gate_seqs.push(gate.seq);
        let effect = runtime
            .commit_effect(EffectRecord {
                propose_seq: ack.seq,
                gate_seq: gate.seq,
                action_hash: [i as u8; 32],
                payload_digest: [i as u8; 32],
            })
            .expect("admission must allow under the permissive policy");
        effect_seqs.push(effect.seq);
    }

    // A RETIRE of fact 0 so the log exercises more than one action kind.
    let retire = Propose {
        action_hash: [0xee; 32],
        action_kind: strata_gate::ActionKindCode::RETIRE,
        params_hash: [0xee; 32],
        context: vec![effect_seqs[0]],
    };
    let ack = runtime.commit_propose(retire);
    propose_seqs.push(ack.seq);
    let gate = runtime.commit_gate(ack.seq).expect("gate commits");
    gate_seqs.push(gate.seq);
    let effect = runtime
        .commit_effect(EffectRecord {
            propose_seq: ack.seq,
            gate_seq: gate.seq,
            action_hash: [0xee; 32],
            payload_digest: [0xee; 32],
        })
        .expect("retire admits");
    effect_seqs.push(effect.seq);

    let log = runtime.into_inner();

    // Kernel side: reviews folded under v1, checkpoints at genesis + tail.
    let kernel = Kernel::<ReviewEvent>::for_version(ALGO_V1).unwrap();
    let mut state = State::default();
    let mut kernel_records = Vec::new();
    for (idx, (card_id, rating)) in reviews.iter().enumerate() {
        let event = ReviewEvent {
            card_id: *card_id,
            rating: *rating,
            event_seq: (idx + 1) as u64,
        };
        kernel.apply(&mut state, &event);
        let payload = borsh::to_vec(&event).expect("borsh ReviewEvent is infallible");
        kernel_records.push(KernelRecord {
            seq: event.event_seq,
            event_hash: *blake3::hash(&payload).as_bytes(),
            payload,
        });
    }
    let tail_seq = kernel_records.last().map(|r| r.seq).unwrap_or(0);
    let genesis = Checkpoint::genesis(ALGO_V1);
    let head = Checkpoint::seal(ALGO_V1, tail_seq, checkpoint_hash(&genesis), &state);
    let checkpoints = if tail_seq > 0 {
        vec![genesis, head]
    } else {
        vec![genesis]
    };

    let gate_frames: Vec<GateFrame> = log
        .all()
        .iter()
        .map(|ev| GateFrame {
            seq: ev.seq,
            kind: ev.kind,
            payload: ev.payload.clone(),
        })
        .collect();
    let last = gate_frames.last().expect("gate log is non-empty");
    let gate_head = gate_frame_hash(last.seq, last.kind, &last.payload);

    let kernel_head = checkpoint_hash(checkpoints.last().expect("non-empty"));
    BuiltStore {
        files: StoreFiles {
            kernel_records,
            checkpoints,
            kernel_head,
            gate_frames,
            policy,
            gate_head,
        },
        propose_seqs,
        effect_seqs,
        gate_seqs,
    }
}

/// Materialize a built store into a fresh tempdir and return the path (the
/// tempdir lives for the process; tests are short-lived).
pub fn materialize(built: &BuiltStore) -> std::path::PathBuf {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("store");
    write_store(&path, &built.files).expect("write_store");
    std::mem::forget(dir);
    path
}

/// Re-encode `files` into `kernel.log` bytes (helper for byte surgery).
pub fn kernel_log_bytes(files: &StoreFiles) -> Vec<u8> {
    let mut out = MAGIC_BYTES.to_vec();
    for r in &files.kernel_records {
        out.extend_from_slice(&encode_kernel_frame(r.seq, &r.event_hash, &r.payload));
    }
    out
}

/// Re-encode `files` into `gate.log` bytes.
pub fn gate_log_bytes(files: &StoreFiles) -> Vec<u8> {
    let mut out = MAGIC_BYTES.to_vec();
    for f in &files.gate_frames {
        out.extend_from_slice(&encode_gate_frame(f.seq, f.kind, &f.payload));
    }
    out
}
