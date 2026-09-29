//! Tamper tests: every field of `VerifyReport` is recomputed, so flipping
//! one sealed byte or forging an ungated EFFECT must fail verification with
//! a failure that NAMES the offending seq.

mod common;

use common::{build_store, gate_log_bytes, kernel_log_bytes, materialize};
use strata_gate::record::{EffectRecord, GateRecord, RecordKind, Verdict};
use strata_verify::layout::{GateFrame, StoreFiles, gate_frame_hash, store_paths, write_store};
use strata_verify::{GapKind, VerifyFailure, verify_store};

fn sample_store() -> common::BuiltStore {
    // 3 gated writes + 1 retire (gate log), 7 reviews over cards 1..=3
    // (kernel log), sealed at the tail.
    build_store(3, &[(1, 3), (2, 1), (1, 4), (3, 2), (2, 3), (1, 1), (3, 3)])
}

#[test]
fn clean_store_verifies_green() {
    let built = sample_store();
    // The runtime's discipline: every propose has exactly one gate and one
    // effect (3 writes + 1 retire = 4 of each).
    assert_eq!(built.propose_seqs.len(), 4);
    assert_eq!(built.gate_seqs.len(), built.propose_seqs.len());
    assert_eq!(built.effect_seqs.len(), built.propose_seqs.len());
    let dir = materialize(&built);
    let report = verify_store(&dir);
    assert!(report.ok(), "clean store must verify: {report:?}");
    assert!(report.log_tail_ok);
    assert!(report.checkpoint_chain_ok);
    assert!(report.state_root_matches);
    assert!(report.gate_verdicts_rederived);
    assert!(report.gaps.is_empty());
}

#[test]
fn flipping_one_byte_in_a_sealed_kernel_segment_fails_naming_the_seq() {
    let built = sample_store();
    let dir = materialize(&built);

    // Flip ONE payload byte of the sealed record at seq 4 (0-based index 3)
    // while leaving its stored event_hash untouched: the recomputed hash no
    // longer matches, and the fold past that point diverges.
    let victim_seq = 4u64;
    let mut files = built.files.clone();
    let victim = files
        .kernel_records
        .iter_mut()
        .find(|r| r.seq == victim_seq)
        .expect("victim record exists");
    let last = victim.payload.last_mut().expect("payload non-empty");
    *last ^= 0x01;
    std::fs::write(store_paths(&dir).kernel_log, kernel_log_bytes(&files))
        .expect("rewrite kernel.log");

    let report = verify_store(&dir);
    assert!(!report.ok());
    assert!(
        !report.log_tail_ok,
        "tampered segment must fail the tail check"
    );
    assert!(
        !report.state_root_matches,
        "tampered segment must break the replayed state root"
    );
    let named = report
        .failures
        .iter()
        .any(|f| f.to_string().contains(&format!("seq {victim_seq}")));
    assert!(
        named,
        "a failure must name the tampered seq {victim_seq}: {:?}",
        report.failures
    );
}

#[test]
fn flipping_one_byte_in_a_stored_checkpoint_breaks_the_chain() {
    let built = sample_store();
    let dir = materialize(&built);

    // Flip one byte inside the borsh checkpoint stream (not the anchors): the
    // chain/head recomputation or the root comparison must fail naming the
    // checkpoint position.
    let mut bytes = std::fs::read(store_paths(&dir).kernel_checkpoints).unwrap();
    let last = bytes.last_mut().expect("checkpoint bytes non-empty");
    *last ^= 0x01;
    std::fs::write(store_paths(&dir).kernel_checkpoints, bytes).unwrap();

    let report = verify_store(&dir);
    assert!(!report.ok());
    assert!(
        !report.checkpoint_chain_ok || !report.state_root_matches,
        "checkpoint tamper must fail a recomputed field: {report:?}"
    );
    let names_position = report
        .failures
        .iter()
        .any(|f| f.to_string().contains("log_seq"));
    assert!(
        names_position,
        "a failure must name the checkpoint log_seq: {:?}",
        report.failures
    );
}

#[test]
fn forged_effect_without_gate_is_flagged_by_rederive_and_sweep() {
    let built = sample_store();
    let dir = materialize(&built);

    // Forge a well-formed EFFECT citing a real PROPOSE but a gate that never
    // admitted it, append it to the raw log, and RE-ANCHOR gate.head like a
    // real forger would. Only recomputation (sweep admission check) can catch
    // it: framing, seq density, and the anchor are all internally consistent.
    let mut files: StoreFiles = built.files.clone();
    let forged = EffectRecord {
        propose_seq: built.propose_seqs[0],
        gate_seq: built.gate_seqs[0] + 100, // no GATE at this seq
        action_hash: [0; 32],
        payload_digest: [0xab; 32],
    };
    let forged_seq = files.gate_frames.last().unwrap().seq + 1;
    files.gate_frames.push(GateFrame {
        seq: forged_seq,
        kind: RecordKind::Effect,
        payload: borsh::to_vec(&forged).expect("borsh EffectRecord is infallible"),
    });
    files.gate_head = {
        let last = files.gate_frames.last().unwrap();
        gate_frame_hash(last.seq, last.kind, &last.payload)
    };
    std::fs::write(store_paths(&dir).gate_log, gate_log_bytes(&files)).unwrap();
    std::fs::write(store_paths(&dir).gate_head, files.gate_head).unwrap();

    let report = verify_store(&dir);
    assert!(
        !report.ok(),
        "forged effect must fail verification: {report:?}"
    );
    assert!(
        report.gaps.contains(&GapKind::OrphanEffect),
        "sweep must flag the orphan effect: {:?}",
        report.gaps
    );
    // The forger did not touch any GATE, so rederivation still passes; the
    // catch is the structural sweep naming the forged effect's seq.
    assert!(report.gate_verdicts_rederived);
    assert!(
        report
            .failures
            .iter()
            .any(|f| matches!(f, VerifyFailure::StructuralGap {
                kind: GapKind::OrphanEffect,
                seq,
            } if *seq == forged_seq)),
        "a failure must name the forged effect seq {forged_seq}: {:?}",
        report.failures
    );
}

#[test]
fn tampered_gate_verdict_is_caught_by_rederivation() {
    let built = sample_store();
    let dir = materialize(&built);

    // Flip one verdict inside a stored GATE record (Allow -> Deny) and
    // re-anchor the head: rederivation under the stored policy disagrees
    // with the stored verdict and names the gate's seq.
    let mut files = built.files.clone();
    let gate_seq = built.gate_seqs[0];
    let frame = files
        .gate_frames
        .iter_mut()
        .find(|f| f.seq == gate_seq)
        .expect("gate frame exists");
    let mut record: GateRecord = borsh::from_slice(&frame.payload).expect("gate payload decodes");
    record.verdict = if record.verdict == Verdict::Allow {
        Verdict::Deny
    } else {
        Verdict::Allow
    };
    frame.payload = borsh::to_vec(&record).expect("re-borsh is infallible");
    files.gate_head = {
        let last = files.gate_frames.last().unwrap();
        gate_frame_hash(last.seq, last.kind, &last.payload)
    };
    std::fs::write(store_paths(&dir).gate_log, gate_log_bytes(&files)).unwrap();
    std::fs::write(store_paths(&dir).gate_head, files.gate_head)
        .unwrap_or_else(|e| panic!("re-anchor failed: {e}"));

    let report = verify_store(&dir);
    assert!(!report.ok(), "verdict tamper must fail: {report:?}");
    assert!(
        !report.gate_verdicts_rederived,
        "rederivation must disagree with the tampered verdict"
    );
    assert!(
        report
            .failures
            .iter()
            .any(|f| matches!(f, VerifyFailure::VerdictMismatch { gate_seq: s } if *s == gate_seq)),
        "a failure must name the tampered gate seq {gate_seq}: {:?}",
        report.failures
    );
}

#[test]
fn missing_artifact_fails_with_io_error() {
    let built = sample_store();
    let dir = materialize(&built);
    std::fs::remove_file(store_paths(&dir).kernel_head).unwrap();
    let report = verify_store(&dir);
    assert!(!report.ok());
    assert!(matches!(
        report.failures.first(),
        Some(VerifyFailure::Io {
            file: "kernel.head",
            ..
        })
    ));
}

#[test]
fn trailing_kernel_event_past_head_checkpoint_fails_naming_seq() {
    let built = sample_store();
    let dir = materialize(&built);

    // Append one extra sealed event PAST the head checkpoint (log tail no
    // longer covered by any checkpoint) without updating anchors.
    let mut files = built.files.clone();
    let extra_seq = files.kernel_records.last().unwrap().seq + 1;
    let event = strata_kernel::event::ReviewEvent {
        card_id: 9,
        rating: 4,
        event_seq: extra_seq,
    };
    let payload = borsh::to_vec(&event).expect("borsh infallible");
    files.kernel_records.push(strata_verify::KernelRecord {
        seq: extra_seq,
        event_hash: *blake3::hash(&payload).as_bytes(),
        payload,
    });
    std::fs::write(store_paths(&dir).kernel_log, kernel_log_bytes(&files)).unwrap();

    let report = verify_store(&dir);
    assert!(!report.ok());
    assert!(
        !report.log_tail_ok,
        "trailing event must fail the tail check"
    );
    assert!(
        report
            .failures
            .iter()
            .any(|f| f.to_string().contains(&format!("seq {extra_seq}"))),
        "a failure must name the trailing seq {extra_seq}: {:?}",
        report.failures
    );
    // Sanity for the writer round-trip used above.
    let _ = write_store; // writer is exercised by materialize()
}
