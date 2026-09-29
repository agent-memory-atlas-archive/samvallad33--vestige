//! Contract tests for the strata determinism kernel.
//!
//! Covers the fixed contract end to end: fold determinism over a 10^4-event
//! stream (two runs, byte-identical state and roots; identical after
//! serialize/deserialize round-trip), cross-thread smoke, quantization edge
//! cases (unit tests in `canonical`), checkpoint tamper detection naming
//! the offending seq, algorithm version dispatch (v1 vs v2 differ, both
//! replayable, mixed-version upgrade path), and retrievability
//! monotonicity between reviews.

use strata_kernel::canonical::{lint_roundtrip, to_q32_32};
use strata_kernel::checkpoint::{checkpoint_hash, state_root, Checkpoint, MAGIC};
use strata_kernel::event::{ReviewEvent, StrataEvent};
use strata_kernel::fsrs::{CardPhase, FsrsFold, ALGO_V1, ALGO_V2};
use strata_kernel::kernel::{kernel_for, Kernel};
use strata_kernel::state::State;
use strata_kernel::verify::{verify, verify_with_head, VerifyError};

/// Deterministic xorshift64* PRNG — no external rand dependency, no
/// platform variance.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// A fixed 10^4-event stream over 64 cards: seqs 1..=n strictly increasing.
fn synth_events(n: u64) -> Vec<ReviewEvent> {
    let mut rng = Rng::new(0x9E37_79B9_7F4A_7C15);
    (1..=n)
        .map(|seq| {
            let card = rng.next_u64() % 64;
            let rating = 1 + (rng.next_u64() % 4) as u8;
            ReviewEvent {
                card_id: card,
                rating,
                event_seq: seq,
            }
        })
        .collect()
}

/// Fold `events` under `version`, returning the final state.
fn fold_all(version: u32, events: &[ReviewEvent]) -> State {
    let kernel = Kernel::<ReviewEvent>::for_version(version).unwrap();
    let mut state = State::default();
    kernel.apply_all(&mut state, events);
    state
}

fn record_of(ev: &ReviewEvent) -> Record {
    let bytes = borsh::to_vec(ev).unwrap();
    (StrataEvent::seq(ev), *blake3::hash(&bytes).as_bytes(), *ev)
}

/// A verifier record: `(seq, blake3(borsh(event)), event)`.
type Record = (u64, [u8; 32], ReviewEvent);

/// Build a full log: checkpoints every `every` seqs (plus genesis), and the
/// records the verifier consumes.
fn build_log(version: u32, events: &[ReviewEvent], every: u64) -> (Vec<Checkpoint>, Vec<Record>) {
    let kernel = Kernel::<ReviewEvent>::for_version(version).unwrap();
    let mut state = State::default();
    let mut checkpoints = vec![Checkpoint::genesis(version)];
    let mut records = Vec::with_capacity(events.len());
    for ev in events {
        kernel.apply(&mut state, ev);
        records.push(record_of(ev));
        let seq = StrataEvent::seq(ev);
        if seq % every == 0 {
            let prev = checkpoints.last().unwrap().hash();
            checkpoints.push(Checkpoint::seal(version, seq, prev, &state));
        }
    }
    (checkpoints, records)
}

// ---------------------------------------------------------------- determinism

#[test]
fn fold_determinism_10k_two_runs_identical() {
    let events = synth_events(10_000);

    let run1 = fold_all(ALGO_V1, &events);
    let run2 = fold_all(ALGO_V1, &events);

    let bytes1 = borsh::to_vec(&run1).unwrap();
    let bytes2 = borsh::to_vec(&run2).unwrap();
    assert_eq!(bytes1, bytes2, "state bytes must be identical across runs");
    assert_eq!(state_root(&run1), state_root(&run2));
    assert!(run1.cards.len() > 32, "stream should exercise most cards");
    assert_eq!(run1.applied_seq, 10_000);
}

#[test]
fn state_survives_serialize_deserialize_bit_for_bit() {
    let events = synth_events(10_000);
    let state = fold_all(ALGO_V1, &events);

    let bytes = borsh::to_vec(&state).unwrap();
    let back: State = borsh::from_slice(&bytes).unwrap();
    let bytes2 = borsh::to_vec(&back).unwrap();

    assert_eq!(
        bytes, bytes2,
        "round-trip must re-serialize to identical bytes"
    );
    assert_eq!(state_root(&back), state_root(&state));
    assert_eq!(back, state);

    // State-type canonicity backstop (debug_assert inside).
    lint_roundtrip(&state);
}

#[test]
fn cross_thread_smoke_identical_root() {
    let events = synth_events(10_000);

    let main_root = state_root(&fold_all(ALGO_V1, &events));

    let handle = std::thread::spawn(move || state_root(&fold_all(ALGO_V1, &events)));
    let thread_root = handle.join().expect("worker thread must not panic");

    assert_eq!(main_root, thread_root);
}

// ------------------------------------------------------------------- dispatch

#[test]
fn algo_versions_dispatch_v1_v2_differ_but_both_replay() {
    let events = synth_events(500);

    let root_v1 = state_root(&fold_all(ALGO_V1, &events));
    let root_v2 = state_root(&fold_all(ALGO_V2, &events));
    assert_ne!(
        root_v1, root_v2,
        "different constants must give different roots"
    );

    let (cps1, recs1) = build_log(ALGO_V1, &events, 100);
    let (cps2, recs2) = build_log(ALGO_V2, &events, 100);
    assert!(verify(&cps1, recs1.into_iter()).is_ok(), "v1 log replays");
    assert!(verify(&cps2, recs2.into_iter()).is_ok(), "v2 log replays");

    // Distinct static kernels, and unknown versions are refused.
    assert!(kernel_for::<ReviewEvent>(ALGO_V1).is_ok());
    assert!(kernel_for::<ReviewEvent>(ALGO_V2).is_ok());
    assert!(kernel_for::<ReviewEvent>(999).is_err());
}

#[test]
fn mixed_version_log_upgrades_at_checkpoint_boundary() {
    // v1 for seqs 1..=250, v2 from 251..=500: old constants keep replaying.
    let events = synth_events(500);
    let (early, late) = events.split_at(250);

    let k1 = Kernel::<ReviewEvent>::for_version(ALGO_V1).unwrap();
    let k2 = Kernel::<ReviewEvent>::for_version(ALGO_V2).unwrap();
    let mut state = State::default();
    let mut checkpoints = vec![Checkpoint::genesis(ALGO_V1)];
    let mut records = Vec::new();

    for ev in early {
        k1.apply(&mut state, ev);
        records.push(record_of(ev));
    }
    let prev = checkpoints.last().unwrap().hash();
    let boundary = StrataEvent::seq(early.last().unwrap());
    checkpoints.push(Checkpoint::seal(ALGO_V1, boundary, prev, &state));

    for ev in late {
        k2.apply(&mut state, ev);
        records.push(record_of(ev));
    }
    let prev = checkpoints.last().unwrap().hash();
    let end = StrataEvent::seq(late.last().unwrap());
    checkpoints.push(Checkpoint::seal(ALGO_V2, end, prev, &state));

    assert!(verify(&checkpoints, records.into_iter()).is_ok());
}

// -------------------------------------------------------------------- tamper

#[test]
fn verify_accepts_well_formed_log() {
    let events = synth_events(300);
    let (cps, recs) = build_log(ALGO_V1, &events, 100);
    assert_eq!(cps.len(), 4); // genesis + 3 seals
    assert!(verify(&cps, recs.clone().into_iter()).is_ok());
    // ...including with a correctly anchored head.
    let anchor = cps.last().unwrap().hash();
    assert!(verify_with_head(&cps, Some(anchor), recs.into_iter()).is_ok());
}

#[test]
fn empty_log_verifies() {
    let empty: std::iter::Empty<Record> = std::iter::empty();
    assert!(verify::<ReviewEvent>(&[], empty).is_ok());
    let genesis = [Checkpoint::genesis(ALGO_V1)];
    let empty: std::iter::Empty<Record> = std::iter::empty();
    assert!(verify(&genesis, empty).is_ok());
    // But an event with only a genesis checkpoint is trailing.
    let ev = ReviewEvent {
        card_id: 1,
        rating: 3,
        event_seq: 1,
    };
    let rec = record_of(&ev);
    assert_eq!(
        verify(&genesis, std::iter::once(rec)).unwrap_err(),
        VerifyError::TrailingEvents { first_extra_seq: 1 }
    );
}

#[test]
fn tampered_event_bytes_fail_naming_seq() {
    let events = synth_events(300);
    let (cps, mut recs) = build_log(ALGO_V1, &events, 100);
    recs[41].1[0] ^= 0x80; // corrupt stored hash of the 42nd record

    let err = verify(&cps, recs.into_iter()).unwrap_err();
    assert_eq!(err, VerifyError::EventHashMismatch { seq: 42 });
}

#[test]
fn tampered_event_payload_fails_naming_seq() {
    let events = synth_events(300);
    let (cps, mut recs) = build_log(ALGO_V1, &events, 100);
    recs[7].2.rating = if recs[7].2.rating == 4 { 1 } else { 4 }; // payload drift

    let err = verify(&cps, recs.into_iter()).unwrap_err();
    assert_eq!(err, VerifyError::EventHashMismatch { seq: 8 });
}

#[test]
fn tampered_checkpoint_root_fails_naming_its_own_seq() {
    let events = synth_events(300);
    let (mut cps, recs) = build_log(ALGO_V1, &events, 100);
    cps[1].state_root[0] ^= 0x01; // tamper the first sealed checkpoint's root

    // Root tamper is detected AT the checkpoint (bit-exact compare), naming
    // its own log_seq — before the successor's chain check would fire.
    let err = verify(&cps, recs.into_iter()).unwrap_err();
    assert_eq!(
        err,
        VerifyError::StateRootMismatch {
            index: 1,
            log_seq: 100,
            expected: cps[1].state_root,
            actual: match err {
                VerifyError::StateRootMismatch { actual, .. } => actual,
                _ => unreachable!(),
            },
        }
    );
}

#[test]
fn tampered_prev_link_fails_as_chain_break_naming_seq() {
    let events = synth_events(300);
    let (mut cps, recs) = build_log(ALGO_V1, &events, 100);
    cps[2].prev_checkpoint[0] ^= 0x01; // corrupt the chain link itself

    let err = verify(&cps, recs.into_iter()).unwrap_err();
    assert_eq!(
        err,
        VerifyError::ChainBreak {
            index: 2,
            log_seq: 200
        }
    );
}

#[test]
fn tampered_head_state_root_fails_on_root() {
    let events = synth_events(300);
    let (mut cps, recs) = build_log(ALGO_V1, &events, 100);
    let last = cps.len() - 1;
    cps[last].state_root[31] ^= 0xFF;

    let err = verify(&cps, recs.into_iter()).unwrap_err();
    match err {
        VerifyError::StateRootMismatch {
            index,
            log_seq,
            expected,
            actual,
        } => {
            assert_eq!(index, last);
            assert_eq!(log_seq, 300);
            assert_ne!(expected, actual);
        }
        other => panic!("expected StateRootMismatch, got {other:?}"),
    }
}

#[test]
fn tampered_head_log_seq_caught_by_anchored_hash() {
    let events = synth_events(300);
    let (mut cps, recs) = build_log(ALGO_V1, &events, 100);
    let anchor = cps.last().unwrap().hash();
    let last = cps.len() - 1;
    cps[last].log_seq += 50; // raising head log_seq is only detectable via anchor

    let err = verify_with_head(&cps, Some(anchor), recs.into_iter()).unwrap_err();
    assert_eq!(
        err,
        VerifyError::CheckpointHashMismatch {
            index: last,
            log_seq: 350
        }
    );
}

#[test]
fn trailing_events_rejected_naming_first_extra_seq() {
    let events = synth_events(300);
    let (cps, mut recs) = build_log(ALGO_V1, &events, 100);
    let extra = ReviewEvent {
        card_id: 1,
        rating: 3,
        event_seq: 301,
    };
    recs.push(record_of(&extra));

    let err = verify(&cps, recs.into_iter()).unwrap_err();
    assert_eq!(
        err,
        VerifyError::TrailingEvents {
            first_extra_seq: 301
        }
    );
}

#[test]
fn bad_magic_rejected_naming_seq() {
    let events = synth_events(50);
    let (mut cps, recs) = build_log(ALGO_V1, &events, 25);
    cps[1].magic = *b"STRATAv0";
    assert_eq!(
        verify(&cps, recs.into_iter()).unwrap_err(),
        VerifyError::BadMagic {
            index: 1,
            log_seq: 25
        }
    );
}

#[test]
fn unknown_algo_version_rejected_naming_seq() {
    // A head checkpoint whose version has no pinned kernel must be refused
    // before any folding (using the head avoids a chain-break firing first).
    let events = synth_events(50);
    let (_cps, recs) = build_log(ALGO_V1, &events, 25);
    let mut state = State::default();
    Kernel::<ReviewEvent>::for_version(ALGO_V1)
        .unwrap()
        .apply_all(&mut state, &events);
    let head = Checkpoint {
        magic: MAGIC,
        algo_version: 77,
        log_seq: 50,
        prev_checkpoint: [0; 32],
        state_root: state_root(&state),
    };
    assert_eq!(
        verify(&[head], recs.into_iter()).unwrap_err(),
        VerifyError::UnknownAlgoVersion {
            version: 77,
            index: 0,
            log_seq: 50
        }
    );
}

#[test]
fn out_of_order_events_rejected() {
    let events = synth_events(30);
    let (cps, mut recs) = build_log(ALGO_V1, &events, 30);
    // Swap two records' payloads and repair their hashes: only the seq
    // ordering is now wrong.
    recs.swap(3, 4);
    for r in recs.iter_mut().take(5).skip(3) {
        let bytes = borsh::to_vec(&r.2).unwrap();
        r.1 = *blake3::hash(&bytes).as_bytes();
    }
    match verify(&cps, recs.into_iter()).unwrap_err() {
        VerifyError::OutOfOrderEvent { seq, applied_seq } => {
            assert_eq!(seq, 4);
            assert_eq!(applied_seq, 5);
        }
        other => panic!("expected OutOfOrderEvent, got {other:?}"),
    }
}

// ------------------------------------------------------ retrievability + misc

#[test]
fn retrievability_decreases_between_reviews() {
    let mut state = State::default();
    let kernel = Kernel::<ReviewEvent>::for_version(ALGO_V1).unwrap();
    kernel.apply(
        &mut state,
        &ReviewEvent {
            card_id: 1,
            rating: 3,
            event_seq: 10,
        },
    );

    // Decay toward the second review (card still anchored at seq 10).
    let card_after_first = state.cards[&1].clone();
    let r_early = FsrsFold::retrievability(&card_after_first, 11, ALGO_V1).unwrap();
    let r_late = FsrsFold::retrievability(&card_after_first, 79, ALGO_V1).unwrap();
    assert_eq!(
        FsrsFold::retrievability(&card_after_first, 10, ALGO_V1).unwrap(),
        1.0
    );
    assert!(r_early < 1.0);
    assert!(
        r_late < r_early,
        "R must decay as seq advances: {r_late} !< {r_early}"
    );

    // A review resets R upward and future seqs decay again.
    kernel.apply(
        &mut state,
        &ReviewEvent {
            card_id: 1,
            rating: 4,
            event_seq: 80,
        },
    );
    let card = &state.cards[&1];
    let r_reset = FsrsFold::retrievability(card, 81, ALGO_V1).unwrap();
    let r_far_future = FsrsFold::retrievability(card, 5000, ALGO_V1).unwrap();
    assert!(
        r_reset > r_late,
        "review must reset retrievability: {r_reset} !> {r_late}"
    );
    assert!(r_far_future < r_reset, "monotone decay after the review");
    assert!(r_far_future > 0.0);
}

#[test]
fn checkpoint_borsh_layout_is_fixed() {
    let g = Checkpoint::genesis(ALGO_V1);
    let bytes = borsh::to_vec(&g).unwrap();
    // magic(8) + algo_version(4) + log_seq(8) + prev(32) + root(32) = 84.
    assert_eq!(bytes.len(), 84);
    assert_eq!(&bytes[0..8], &MAGIC);
    assert_eq!(
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        ALGO_V1
    );
    assert_eq!(u64::from_le_bytes(bytes[12..20].try_into().unwrap()), 0);
    assert_eq!(
        checkpoint_hash(&g),
        *blake3::hash(&borsh::to_vec(&g).unwrap()).as_bytes()
    );
}

#[test]
fn card_state_is_quantized_and_bounded() {
    let events = synth_events(2000);
    let state = fold_all(ALGO_V1, &events);
    for card in state.cards.values() {
        assert!(card.stability_q <= to_q32_32(36500.0));
        assert!(card.stability_q >= to_q32_32(0.01));
        assert!(card.difficulty_q >= to_q32_32(1.0));
        assert!(card.difficulty_q <= to_q32_32(10.0));
    }
}

#[test]
fn short_term_zero_elapsed_fold_is_deterministic() {
    // Two events on the same card at the same seq: the second takes the
    // short-term path. Repeated runs must agree bit for bit.
    let evs = vec![
        ReviewEvent {
            card_id: 5,
            rating: 3,
            event_seq: 1,
        },
        ReviewEvent {
            card_id: 5,
            rating: 4,
            event_seq: 1,
        },
    ];
    let s1 = fold_all(ALGO_V1, &evs);
    let s2 = fold_all(ALGO_V1, &evs);
    assert_eq!(borsh::to_vec(&s1).unwrap(), borsh::to_vec(&s2).unwrap());
    assert_eq!(s1.cards[&5].review_count, 2);
    assert_eq!(s1.cards[&5].phase, CardPhase::Review);
}

#[test]
fn state_bytes_are_btree_ordered() {
    let mut state = State::default();
    let kernel = Kernel::<ReviewEvent>::for_version(ALGO_V1).unwrap();
    // Insert cards in descending id order.
    for (card, seq) in [(9u64, 1u64), (4, 2), (2, 3)] {
        kernel.apply(
            &mut state,
            &ReviewEvent {
                card_id: card,
                rating: 3,
                event_seq: seq,
            },
        );
    }
    let bytes = borsh::to_vec(&state).unwrap();
    let back: State = borsh::from_slice(&bytes).unwrap();
    let ids: Vec<u64> = back.cards.keys().copied().collect();
    assert_eq!(
        ids,
        vec![2, 4, 9],
        "BTreeMap must iterate ascending in borsh bytes"
    );
    assert_eq!(state_root(&back), state_root(&state));
}
