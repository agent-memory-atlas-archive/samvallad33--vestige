//! Storage contracts for exact salience changes and compensating undo.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use strata_kernel::canonical::to_q32_32;

use crate::{BackfillAdjustment, EffectAction, IngestInput, StoreError, StrataStore};

static NEXT_TEST: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    dir: PathBuf,
    store: StrataStore,
    memory: String,
    trigger: String,
}

fn fixture() -> Fixture {
    let dir = std::env::temp_dir().join(format!(
        "strata-backfill-{}-{}",
        std::process::id(),
        NEXT_TEST.fetch_add(1, Ordering::Relaxed)
    ));
    let mut store = StrataStore::open(&dir).unwrap();
    let memory = put(
        &mut store,
        "Maya felt off after dinner at Bangkok Garden",
        &["person:Maya", "place:Bangkok Garden"],
        10,
    );
    let trigger = put(
        &mut store,
        "Maya has a peanut allergy",
        &["person:Maya", "allergy:peanut"],
        20,
    );
    Fixture {
        dir,
        store,
        memory,
        trigger,
    }
}

fn put(store: &mut StrataStore, content: &str, tags: &[&str], at: i64) -> String {
    store
        .ingest_in_scope(
            IngestInput {
                content: content.into(),
                tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
                created_at_ms: Some(at),
                ..Default::default()
            },
            "personal",
        )
        .unwrap()
}

fn request(fixture: &Fixture, new_stability_q: i64) -> BackfillAdjustment {
    BackfillAdjustment {
        memory_id: fixture.memory.clone(), trigger_id: fixture.trigger.clone(),
        shared_entity: "person:Maya".into(),
        reason: "Raised because a new health fact about Maya makes her earlier bad reaction after Bangkok Garden relevant".into(),
        new_stability_q, timestamp_ms: 30,
    }
}

#[test]
fn backfill_receipt_card_and_checkpoint_survive_reopen() {
    let mut f = fixture();
    let before = f.store.card_state(&f.memory).unwrap();
    let receipt = f
        .store
        .apply_backfill(request(&f, i64::MAX))
        .unwrap()
        .unwrap();
    let after = f.store.card_state(&f.memory).unwrap();
    assert_eq!(
        after.stability_q,
        before.stability_q + before.stability_q / 2
    );
    assert_eq!(after.last_seq, before.last_seq, "salience is not a review");
    assert_eq!(after.review_count, before.review_count);
    assert_eq!(receipt.old_stability_q, before.stability_q);
    assert_eq!(receipt.new_stability_q, after.stability_q);
    let seq = u64::from_str_radix(receipt.receipt_id.strip_prefix("eff-").unwrap(), 16).unwrap();
    let proof = f.store.effect_by_seq(seq).unwrap().unwrap();
    assert_eq!(proof.action, EffectAction::Backfill);
    assert_eq!(proof.node_id, f.memory);
    f.store.seal_checkpoint().unwrap();
    let digest = f.store.state_digest();
    assert_eq!(digest, f.store.refold().unwrap().state_digest);
    drop(f.store);
    let reopened = StrataStore::open(&f.dir).unwrap();
    assert_eq!(reopened.state_digest(), digest);
    assert_eq!(reopened.card_state(&f.memory).unwrap(), after);
    assert_eq!(reopened.backfill_receipts(), vec![receipt]);
}

#[test]
fn backfill_pair_and_receipt_reads_never_append() {
    let mut f = fixture();
    f.store
        .apply_backfill(request(&f, i64::MAX))
        .unwrap()
        .unwrap();
    let seq = f.store.log().head().next_seq;
    for _ in 0..3 {
        assert!(f
            .store
            .apply_backfill(request(&f, i64::MAX))
            .unwrap()
            .is_none());
        assert_eq!(f.store.backfill_receipts().len(), 1);
    }
    assert_eq!(f.store.log().head().next_seq, seq);
    drop(f.store);
    f.store = StrataStore::open(&f.dir).unwrap();
    let seq = f.store.log().head().next_seq;
    assert!(f
        .store
        .apply_backfill(request(&f, i64::MAX))
        .unwrap()
        .is_none());
    assert_eq!(f.store.log().head().next_seq, seq);
}

#[test]
fn backfill_lowering_floor_and_exact_idempotent_undo() {
    let mut f = fixture();
    let before = f.store.card_state(&f.memory).unwrap().stability_q;
    let receipt = f.store.apply_backfill(request(&f, 1)).unwrap().unwrap();
    assert_eq!(
        receipt.new_stability_q,
        ((i128::from(before) * 2 + 2) / 3) as i64
    );
    let undo = f.store.undo_backfill(&receipt.receipt_id, 40).unwrap();
    assert_eq!(undo.undo_of.as_deref(), Some(receipt.receipt_id.as_str()));
    assert_eq!(undo.old_stability_q, receipt.new_stability_q);
    assert_eq!(undo.new_stability_q, before);
    assert_eq!(f.store.card_state(&f.memory).unwrap().stability_q, before);
    let seq = f.store.log().head().next_seq;
    assert_eq!(
        f.store.undo_backfill(&receipt.receipt_id, 50).unwrap(),
        undo
    );
    assert!(f
        .store
        .apply_backfill(request(&f, i64::MAX))
        .unwrap()
        .is_none());
    assert_eq!(f.store.log().head().next_seq, seq);
    f.store.seal_checkpoint().unwrap();
    drop(f.store);
    f.store = StrataStore::open(&f.dir).unwrap();
    assert_eq!(f.store.card_state(&f.memory).unwrap().stability_q, before);
    assert_eq!(
        f.store.undo_backfill(&receipt.receipt_id, 60).unwrap(),
        undo
    );
    assert!(f.store.undo_backfill(&undo.receipt_id, 60).is_err());
}

#[test]
fn backfill_undo_rejects_a_later_review_or_adjustment() {
    let mut f = fixture();
    let receipt = f
        .store
        .apply_backfill(request(&f, i64::MAX))
        .unwrap()
        .unwrap();
    f.store.review(&f.memory, 3).unwrap();
    let seq = f.store.log().head().next_seq;
    assert!(f.store.undo_backfill(&receipt.receipt_id, 40).is_err());
    assert_eq!(f.store.log().head().next_seq, seq);

    let trigger = put(
        &mut f.store,
        "Maya has updated health information",
        &["person:Maya"],
        40,
    );
    let mut second = request(&f, 1);
    second.trigger_id = trigger;
    f.store.apply_backfill(second).unwrap().unwrap();
    let seq = f.store.log().head().next_seq;
    assert!(f.store.undo_backfill(&receipt.receipt_id, 50).is_err());
    assert_eq!(f.store.log().head().next_seq, seq);
}

#[test]
fn backfill_rejects_future_different_person_and_unrecorded_entity_without_writes() {
    let mut f = fixture();
    let mut invalid = request(&f, i64::MAX);
    invalid.shared_entity = "person:Alex".into();
    let seq = f.store.log().head().next_seq;
    assert!(f.store.apply_backfill(invalid).is_err());
    assert_eq!(f.store.log().head().next_seq, seq);

    let other = put(
        &mut f.store,
        "Alex felt off at Bangkok Garden",
        &["person:Alex", "place:Bangkok Garden"],
        5,
    );
    let mut other_request = request(&f, i64::MAX);
    other_request.memory_id = other;
    let seq = f.store.log().head().next_seq;
    assert!(f.store.apply_backfill(other_request).is_err());
    assert_eq!(f.store.log().head().next_seq, seq);

    let future = put(&mut f.store, "Maya's later memory", &["person:Maya"], 50);
    let mut future_request = request(&f, i64::MAX);
    future_request.memory_id = future;
    let seq = f.store.log().head().next_seq;
    assert!(f.store.apply_backfill(future_request).is_err());
    assert_eq!(f.store.log().head().next_seq, seq);
}

#[test]
fn backfill_invalid_fields_and_noop_do_not_write() {
    let mut f = fixture();
    for (score, at, reason) in [
        (0, 30, "reason"),
        (-1, 30, "reason"),
        (1, -1, "reason"),
        (1, 30, " "),
    ] {
        let mut change = request(&f, score);
        change.timestamp_ms = at;
        change.reason = reason.into();
        let seq = f.store.log().head().next_seq;
        assert!(matches!(
            f.store.apply_backfill(change),
            Err(StoreError::InvalidInput(_))
        ));
        assert_eq!(f.store.log().head().next_seq, seq);
    }
    let score = f.store.card_state(&f.memory).unwrap().stability_q;
    let seq = f.store.log().head().next_seq;
    assert!(f
        .store
        .apply_backfill(request(&f, score))
        .unwrap()
        .is_none());
    assert!(f.store.backfill_receipts().is_empty());
    assert_eq!(f.store.log().head().next_seq, seq);
}

#[test]
fn backfill_topic_tags_and_a_shared_place_cannot_bypass_person_evidence() {
    let mut f = fixture();
    let a = put(&mut f.store, "an earlier generic topic", &["personal"], 10);
    let b = put(&mut f.store, "a newer generic topic", &["personal"], 20);
    let mut topic = request(&f, i64::MAX);
    topic.memory_id = a;
    topic.trigger_id = b;
    topic.shared_entity = "personal".into();
    let seq = f.store.log().head().next_seq;
    assert!(f.store.apply_backfill(topic).is_err());
    assert_eq!(f.store.log().head().next_seq, seq);

    let alex = put(
        &mut f.store,
        "Alex felt off at Bangkok Garden",
        &["person:Alex", "place:Bangkok Garden"],
        10,
    );
    let maya = put(
        &mut f.store,
        "Maya has a peanut allergy",
        &["person:Maya", "place:Bangkok Garden", "allergy:peanut"],
        20,
    );
    let mut place = request(&f, i64::MAX);
    place.memory_id = alex;
    place.trigger_id = maya;
    place.shared_entity = "place:Bangkok Garden".into();
    let seq = f.store.log().head().next_seq;
    let error = f.store.apply_backfill(place).unwrap_err().to_string();
    assert!(error.contains("same explicit person"), "{error}");
    assert_eq!(f.store.log().head().next_seq, seq);
}

#[test]
fn backfill_distinct_triggers_preserve_step_bounds_and_global_kernel_cap() {
    let mut f = fixture();
    for index in 0..110 {
        if index != 0 {
            f.trigger = put(
                &mut f.store,
                &format!("Maya fact {index}"),
                &["person:Maya"],
                100 + index,
            );
        }
        let before = f.store.card_state(&f.memory).unwrap().stability_q;
        let change = f.store.apply_backfill(request(&f, i64::MAX)).unwrap();
        let after = f.store.card_state(&f.memory).unwrap().stability_q;
        assert!(after <= before + before / 2);
        assert!(after <= before + to_q32_32(365.0));
        assert!(after <= to_q32_32(strata_kernel::fsrs::S_MAX));
        assert_eq!(change.is_some(), before != after);
    }
}

#[test]
fn all_personal_attributes_require_the_actual_person_join_at_the_store_boundary() {
    for attribute in [
        "health:asthma",
        "condition:asthma",
        "medication:inhaler",
        "diet:vegan",
    ] {
        let mut f = fixture();
        let alex = put(
            &mut f.store,
            "Alex dinner",
            &["person:Alex", "place:Bangkok Garden"],
            10,
        );
        let trigger = put(
            &mut f.store,
            "Maya personal update",
            &["person:Maya", "place:Bangkok Garden", attribute],
            20,
        );
        let mut change = request(&f, i64::MAX);
        change.memory_id = alex;
        change.trigger_id = trigger;
        change.shared_entity = "place:Bangkok Garden".into();
        let seq = f.store.log().head().next_seq;
        assert!(
            f.store.apply_backfill(change).is_err(),
            "{attribute} must not cross people through a place"
        );
        assert_eq!(f.store.log().head().next_seq, seq);
    }
}
