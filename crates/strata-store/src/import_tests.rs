//! Imported v3 FSRS state on the store's card fold: `FSRS_STATE` cards,
//! `FSRS_REVIEW` series keyed by importer kernel id, restart determinism,
//! and native reviews continuing from the imported card.

use std::path::{Path, PathBuf};

use strata_kernel::canonical::to_q32_32;
use strata_kernel::event::ReviewEvent;
use strata_kernel::fsrs::{CardPhase, CardState, ALGO_V2, D_MIN, S_MAX};
use strata_kernel::kernel::Kernel;
use strata_kernel::state::State;
use strata_migrate::records::{KIND_FSRS_REVIEW, KIND_FSRS_STATE};
use strata_migrate::{FsrsStateRecord, NodeRecord, RECORD_VERSION};

use crate::card::CardEvent;
use crate::op::KIND_STORE_WRITE;
use crate::store::handle_of;
use crate::StrataStore;

const DAY_MS: i64 = 86_400_000;
/// The source clock planted rows are dated against (2026-09-29T05:52:21Z).
const SOURCE_CLOCK_MS: i64 = 1_790_661_141_211;
/// The personalized decay on the real store this regression came from.
const W20: f64 = 0.080_337_243_220_494_4;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "strata-store-import-test-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// Append one frame, returning its seq.
fn append(dir: &Path, kind: u8, payload: &[u8]) -> u64 {
    let log = strata::StrataLog::open(dir.join("log")).expect("log");
    log.append(kind, payload).expect("append").seq
}

fn node(id: &str, kernel_id: u64) -> Vec<u8> {
    borsh::to_vec(&NodeRecord {
        record_version: RECORD_VERSION,
        legacy_id: id.to_string(),
        kernel_id,
        content: format!("imported {id}"),
        node_type: "fact".to_string(),
        tags: Vec::new(),
        created_ms: SOURCE_CLOCK_MS - 30 * DAY_MS,
        updated_ms: SOURCE_CLOCK_MS - 30 * DAY_MS,
        last_accessed_ms: SOURCE_CLOCK_MS - 10 * DAY_MS,
        legacy: Vec::new(),
        source: None,
        source_updated_at_ms: None,
    })
    .expect("encode node")
}

/// Importer `FSRS_REVIEW` payload: `ReviewEvent` + `borsh(Option<i64>)`.
fn review(kernel_id: u64, rating: u8, event_seq: u64, reviewed_at_ms: Option<i64>) -> Vec<u8> {
    let mut bytes = borsh::to_vec(&ReviewEvent {
        card_id: kernel_id,
        rating,
        event_seq,
    })
    .expect("encode review");
    bytes.extend(borsh::to_vec(&reviewed_at_ms).expect("encode clock"));
    bytes
}

fn state(id: &str, kernel_id: u64, stability: f64, difficulty: f64) -> FsrsStateRecord {
    FsrsStateRecord {
        record_version: RECORD_VERSION,
        kernel_id,
        legacy_id: id.to_string(),
        algo_version: ALGO_V2,
        stability_q: to_q32_32(stability),
        difficulty_q: to_q32_32(difficulty),
        review_count: 4,
        lapse_count: 1,
        phase: CardPhase::Review,
        reviewed_at_ms: SOURCE_CLOCK_MS - 10 * DAY_MS,
        v3_retrievability_q: to_q32_32(0.8),
        v3_decay_q: to_q32_32(W20),
        fitted_at_ms: SOURCE_CLOCK_MS,
    }
}

fn next_seq(dir: &Path) -> u64 {
    strata::StrataLog::open(dir.join("log"))
        .expect("log")
        .head()
        .next_seq
}

/// An `FSRS_STATE` frame becomes the node's card, keyed by its handle, with
/// `last_seq` at the frame and the review clock at v3's `last_accessed`.
#[test]
fn imported_state_becomes_the_card_and_review_clock() {
    let dir = temp_dir("state-card");
    let id = "aaaaaaaa-0000-4000-8000-000000000001";
    append(&dir, KIND_STORE_WRITE, &node(id, 1));
    let record = state(id, 1, 6.3, 2.118);
    let seq = append(&dir, KIND_FSRS_STATE, &borsh::to_vec(&record).unwrap());

    let store = StrataStore::open(&dir).expect("replay");
    let card = store.card_state(id).expect("imported card");
    assert_eq!(
        card,
        CardState {
            stability_q: record.stability_q,
            difficulty_q: record.difficulty_q,
            last_seq: seq,
            review_count: 4,
            lapse_count: 1,
            phase: CardPhase::Review,
        }
    );
    assert_eq!(store.reviewed_at_ms(id), Some(record.reviewed_at_ms));
    // Review age counts from last_accessed, not from the import's seq.
    let ten_days = strata_kernel::fsrs::FsrsFold::retrievability_at_review(
        &card,
        Some(record.reviewed_at_ms),
        SOURCE_CLOCK_MS,
        seq,
        ALGO_V2,
    )
    .unwrap();
    assert_eq!(
        store.retrievability_at(id, SOURCE_CLOCK_MS).unwrap(),
        Some(ten_days)
    );
    assert!(ten_days < 0.9, "ten days on a 6.3-day card: {ten_days}");
    assert!(matches!(
        store.card_events().as_slice(),
        [CardEvent::Import(card)] if card.card_id == handle_of(id) && card.event_seq == seq
    ));
    std::fs::remove_dir_all(&dir).ok();
}

/// `FSRS_REVIEW` frames name the importer's kernel id. The store folds them
/// onto `handle_of(v3 id)`, never onto the raw kernel id, and ignores a
/// review that names no imported node.
#[test]
fn imported_reviews_map_kernel_id_to_the_node_handle() {
    let dir = temp_dir("review-map");
    let a = "aaaaaaaa-0000-4000-8000-00000000000a";
    let b = "bbbbbbbb-0000-4000-8000-00000000000b";
    append(&dir, KIND_STORE_WRITE, &node(a, 1));
    append(&dir, KIND_STORE_WRITE, &node(b, 2));
    let clock = SOURCE_CLOCK_MS - 3 * DAY_MS;
    let mut seqs = Vec::new();
    for (rating, reviewed_at) in [(3u8, None), (3, None), (1, Some(clock))] {
        let seq = next_seq(&dir);
        seqs.push(append(
            &dir,
            KIND_FSRS_REVIEW,
            &review(2, rating, seq, reviewed_at),
        ));
    }
    let unknown = next_seq(&dir);
    append(&dir, KIND_FSRS_REVIEW, &review(99, 3, unknown, None));

    let store = StrataStore::open(&dir).expect("replay");
    assert!(
        store.card_state(a).is_none(),
        "kernel id 1 was never reviewed"
    );
    let card = store.card_state(b).expect("b's card");
    assert_eq!((card.review_count, card.lapse_count), (3, 1));
    assert_eq!(card.phase, CardPhase::Relearning);
    assert_eq!(card.last_seq, seqs[2]);
    assert_eq!(store.reviewed_at_ms(b), Some(clock));
    let events = store.card_events();
    assert_eq!(events.len(), 3);
    assert!(events
        .iter()
        .all(|event| matches!(event, CardEvent::Review(r) if r.card_id == handle_of(b))));

    let kernel = Kernel::<ReviewEvent>::for_version(ALGO_V2).unwrap();
    let mut fold = State::default();
    for (seq, rating) in seqs.iter().zip([3u8, 3, 1]) {
        kernel.apply(
            &mut fold,
            &ReviewEvent {
                card_id: handle_of(b),
                rating,
                event_seq: *seq,
            },
        );
    }
    assert_eq!(fold.cards[&handle_of(b)], card);
    std::fs::remove_dir_all(&dir).ok();
}

/// A state never rewinds a card that already exists, and a state for an
/// unknown node or another wire version is ignored. Out-of-range values
/// are clamped into the kernel's domain.
#[test]
fn imported_state_is_first_word_only_and_clamped() {
    let dir = temp_dir("state-guards");
    let reviewed = "cccccccc-0000-4000-8000-00000000000c";
    let wild = "dddddddd-0000-4000-8000-00000000000d";
    append(&dir, KIND_STORE_WRITE, &node(reviewed, 1));
    append(&dir, KIND_STORE_WRITE, &node(wild, 2));
    let seq = next_seq(&dir);
    append(&dir, KIND_FSRS_REVIEW, &review(1, 3, seq, None));
    append(
        &dir,
        KIND_FSRS_STATE,
        &borsh::to_vec(&state(reviewed, 1, 900.0, 9.0)).unwrap(),
    );
    append(
        &dir,
        KIND_FSRS_STATE,
        &borsh::to_vec(&state("eeeeeeee-0000-4000-8000-00000000000e", 3, 5.0, 5.0)).unwrap(),
    );
    let mut future = state(wild, 2, 5.0, 5.0);
    future.record_version = RECORD_VERSION + 1;
    append(&dir, KIND_FSRS_STATE, &borsh::to_vec(&future).unwrap());
    let mut extreme = state(wild, 2, 1.0e9, 0.0);
    extreme.lapse_count = 99;
    let extreme_seq = append(&dir, KIND_FSRS_STATE, &borsh::to_vec(&extreme).unwrap());

    let store = StrataStore::open(&dir).expect("replay");
    let card = store.card_state(reviewed).expect("reviewed card");
    assert_eq!(
        card.review_count, 1,
        "the state did not overwrite the review"
    );
    assert!(store.reviewed_at_ms(reviewed).is_none());
    assert!(store
        .card_state("eeeeeeee-0000-4000-8000-00000000000e")
        .is_none());
    let clamped = store.card_state(wild).expect("wild card");
    assert_eq!(
        clamped.last_seq, extreme_seq,
        "the future version was skipped"
    );
    assert_eq!(clamped.stability_q, to_q32_32(S_MAX));
    assert_eq!(clamped.difficulty_q, to_q32_32(D_MIN));
    assert_eq!(clamped.lapse_count, clamped.review_count);
    std::fs::remove_dir_all(&dir).ok();
}

/// One planted v3 row: (id, stability, difficulty, reps, lapses, state,
/// sentiment magnitude, last access age in hours before the source clock).
type Row = (&'static str, f64, f64, i64, i64, &'static str, f64, i64);

const ROWS: [Row; 3] = [
    (
        "11111111-aaaa-4aaa-8aaa-111111111111",
        2.3065,
        2.118,
        0,
        0,
        "new",
        0.0,
        16 * 24 + 7,
    ),
    (
        "22222222-aaaa-4aaa-8aaa-222222222222",
        6.98,
        2.118,
        0,
        0,
        "new",
        0.8,
        41 * 24,
    ),
    (
        "33333333-aaaa-4aaa-8aaa-333333333333",
        1.4,
        7.4,
        3,
        1,
        "review",
        0.0,
        2,
    ),
];

fn rfc3339(ms: i64) -> String {
    // UTC, millisecond precision, the form v3 wrote (`+00:00`).
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant), valid for the proleptic Gregorian calendar.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}+00:00",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn text(value: &str) -> String {
    format!(r#"{{"type":"text","value":"{value}"}}"#)
}

fn real(value: f64) -> String {
    format!(r#"{{"type":"real","value":{value:?}}}"#)
}

fn integer(value: i64) -> String {
    format!(r#"{{"type":"integer","value":{value}}}"#)
}

/// A v3 portable archive with scheduling on `knowledge_nodes`, a
/// personalized `w20`, and no `fsrs_cards` table: the real-store shape.
fn knowledge_nodes_only_archive(path: &Path) {
    let created = rfc3339(SOURCE_CLOCK_MS - 60 * DAY_MS);
    let rows: Vec<String> = ROWS
        .iter()
        .enumerate()
        .map(
            |(index, (id, s, d, reps, lapses, phase, sentiment, age_h))| {
                let last = rfc3339(SOURCE_CLOCK_MS - age_h * 3_600_000);
                // The first row's update is the source's latest timestamp.
                let updated = if index == 0 {
                    rfc3339(SOURCE_CLOCK_MS)
                } else {
                    created.clone()
                };
                format!(
                    "[{}]",
                    [
                        text(id),
                        text(&format!("memory {index}")),
                        text("fact"),
                        text(&created),
                        text(&updated),
                        text(&last),
                        real(*s),
                        real(*d),
                        integer(*reps),
                        integer(*lapses),
                        text(phase),
                        real(*sentiment),
                        text("[]"),
                    ]
                    .join(",")
                )
            },
        )
        .collect();
    let json = format!(
        r#"{{"archiveFormat":"vestige.portable.v1","vestigeVersion":"3.1.1","schemaVersion":40,
"exportedAt":"2026-09-29T06:00:00Z","mode":"exact","tables":[
{{"name":"knowledge_nodes","columns":["id","content","node_type","created_at","updated_at",
"last_accessed","stability","difficulty","reps","lapses","learning_state","sentiment_magnitude",
"tags"],"rows":[{}]}},
{{"name":"fsrs_config","columns":["key","value","updated_at"],"rows":[[{},{},{}]]}}]}}"#,
        rows.join(","),
        text("w20"),
        real(W20),
        text(&created),
    );
    std::fs::write(path, json).expect("write archive");
}

/// v3's `apply_decay` retrievability for a planted row at `at_ms`.
fn v3_retrievability(row: &Row, at_ms: i64) -> f64 {
    let (_, s, _, _, _, _, sentiment, age_h) = *row;
    let last = SOURCE_CLOCK_MS - age_h * 3_600_000;
    let days = ((at_ms - last) / 1000) as f64 / 86_400.0;
    let factor = 0.9f64.powf(-1.0 / W20) - 1.0;
    (1.0 + factor * days / (s * (1.0 + sentiment * 0.5))).powf(-W20)
}

fn import_real(tag: &str) -> PathBuf {
    let dir = temp_dir(tag);
    let archive = dir.join("v3.json");
    knowledge_nodes_only_archive(&archive);
    let report = strata_migrate::migrate(&archive, &dir.join("log")).expect("migrate");
    assert!(report.verify_passed, "{report:?}");
    assert_eq!((report.fsrs_events, report.fsrs_states), (0, 3));
    dir
}

/// The real importer over a knowledge_nodes-only source: every memory has a
/// card, its retrievability equals v3's at the fit clock, and the fold
/// replays identically across restarts, checkpoints, and a refold.
#[test]
fn knowledge_nodes_only_import_replays_into_cards() {
    let dir = import_real("kn-only");
    let store = StrataStore::open(&dir).expect("open upgraded store");
    for row in &ROWS {
        let (id, _, d, reps, lapses, _, _, age_h) = *row;
        let card = store.card_state(id).expect("every memory has a card");
        assert_eq!(card.review_count as i64, reps);
        assert_eq!(card.lapse_count as i64, lapses);
        assert_eq!(card.difficulty_q, to_q32_32(d));
        let last = SOURCE_CLOCK_MS - age_h * 3_600_000;
        assert_eq!(store.reviewed_at_ms(id), Some(last));
        let fitted_at = SOURCE_CLOCK_MS.max(last + DAY_MS);
        let strata = store.retrievability_at(id, fitted_at).unwrap().unwrap();
        let v3 = v3_retrievability(row, fitted_at);
        assert!((strata - v3).abs() < 1e-6, "{id}: {strata} vs v3 {v3}");
    }

    let digest = store.state_digest();
    let events = store.review_event_count();
    assert_eq!(events, 3);
    drop(store);
    let mut store = StrataStore::open(&dir).expect("reopen");
    assert_eq!(
        store.state_digest(),
        digest,
        "restart replays the same fold"
    );
    assert_eq!(store.refold().expect("refold").state_digest, digest);

    let sealed = store.seal_checkpoint().expect("seal over imported cards");
    assert_eq!(sealed.algo_version, ALGO_V2);
    let digest = store.state_digest();
    drop(store);
    let store = StrataStore::open(&dir).expect("reopen verifies the checkpoint");
    assert_eq!(store.state_digest(), digest);
    assert_eq!(store.checkpoints(), &[sealed]);
    std::fs::remove_dir_all(&dir).ok();
}

/// A native review of an imported card folds onto the imported state (not
/// a fresh card), keeps the counters, and survives restart and checkpoint
/// verification.
#[test]
fn native_review_continues_from_the_imported_card() {
    let dir = import_real("continue");
    let id = ROWS[2].0;
    let mut store = StrataStore::open(&dir).expect("open");
    let imported = store.card_state(id).expect("imported card");
    let reviewed_at = SOURCE_CLOCK_MS + DAY_MS;
    store.review_at(id, 3, Some(reviewed_at)).expect("review");
    let after = store.card_state(id).expect("card");
    assert!(after.last_seq > imported.last_seq);
    assert_eq!(after.review_count, imported.review_count + 1);
    assert_eq!(after.lapse_count, imported.lapse_count);
    assert_eq!(store.reviewed_at_ms(id), Some(reviewed_at));

    let kernel = Kernel::<ReviewEvent>::for_version(ALGO_V2).unwrap();
    let event = ReviewEvent {
        card_id: handle_of(id),
        rating: 3,
        event_seq: after.last_seq,
    };
    let mut continued = State::default();
    continued.cards.insert(handle_of(id), imported.clone());
    kernel.apply(&mut continued, &event);
    assert_eq!(continued.cards[&handle_of(id)], after);
    let mut fresh = State::default();
    kernel.apply(&mut fresh, &event);
    assert_ne!(
        fresh.cards[&handle_of(id)].stability_q,
        after.stability_q,
        "the review must not restart the card"
    );

    store.seal_checkpoint().expect("seal");
    let digest = store.state_digest();
    drop(store);
    let reopened = StrataStore::open(&dir).expect("reopen verifies imports + review");
    assert_eq!(reopened.card_state(id), Some(after));
    assert_eq!(reopened.state_digest(), digest);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rfc3339_helper_matches_the_source_clock() {
    assert_eq!(rfc3339(SOURCE_CLOCK_MS), "2026-09-29T05:52:21.211+00:00");
    assert_eq!(rfc3339(0), "1970-01-01T00:00:00.000+00:00");
}
