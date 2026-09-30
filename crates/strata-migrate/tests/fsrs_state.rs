//! v3 scheduling state carried from `knowledge_nodes` (no `fsrs_cards`).
//!
//! Real v3 stores keep FSRS state on `knowledge_nodes` and leave
//! `fsrs_cards` empty. These tests build a real v3 store through the public
//! storage API, plant scheduling columns the way v3 leaves them, migrate,
//! and check every imported card against v3's own retrievability function.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use strata::StrataLog;
use strata_kernel::canonical::{from_q32_32, to_q32_32};
use strata_kernel::fsrs::{CardPhase, CardState, FsrsFold, ALGO_V2};
use strata_migrate::records::{decode_params, KIND_FSRS_STATE, KIND_PARAMS};
use strata_migrate::{
    migrate, migrate_with_options, read_snapshot, FsrsStateRecord, MigrateOptions, PARAMS_ID,
};
use vestige_core::{IngestInput, SqliteMemoryStore};

const DAY_MS: i64 = 86_400_000;
/// The personalized decay on the real store this regression came from.
const W20: f64 = 0.080_337_243_220_494_4;

/// One planted `knowledge_nodes` row.
struct Planted {
    stability: f64,
    difficulty: f64,
    reps: i64,
    lapses: i64,
    learning_state: &'static str,
    sentiment_magnitude: f64,
    last_accessed: DateTime<Utc>,
}

/// The source clock every row is planted relative to.
fn source_clock() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-09-29T05:52:21.211917+00:00")
        .unwrap()
        .with_timezone(&Utc)
}

fn planted_rows() -> Vec<Planted> {
    let now = source_clock();
    vec![
        // The common case: v3's initial stability, never reviewed, 16 days old.
        Planted {
            stability: 2.3065,
            difficulty: 2.118_103_970_459_01,
            reps: 0,
            lapses: 0,
            learning_state: "new",
            sentiment_magnitude: 0.0,
            last_accessed: now - Duration::hours(16 * 24 + 7),
        },
        // Emotional memory: v3 stretched stability by 1 + 0.5 * magnitude.
        Planted {
            stability: 6.980_591_070_601_1,
            difficulty: 2.118_103_970_459_01,
            reps: 0,
            lapses: 0,
            learning_state: "new",
            sentiment_magnitude: 0.8,
            last_accessed: now - Duration::days(41),
        },
        // Reviewed card with more lapses than reps (clamped), 2 hours old.
        Planted {
            stability: 1.4,
            difficulty: 7.400_274_371_982_88,
            reps: 3,
            lapses: 5,
            learning_state: "review",
            sentiment_magnitude: 0.0,
            last_accessed: now - Duration::hours(2),
        },
        // Maximum v3 stability, accessed at the source clock itself.
        Planted {
            stability: 36_500.0,
            difficulty: 5.0,
            reps: 1,
            lapses: 0,
            learning_state: "relearning",
            sentiment_magnitude: 0.1,
            last_accessed: now,
        },
    ]
}

/// A real v3 store with the planted scheduling state, `fsrs_config.w20`
/// set, and no `fsrs_cards` rows. Returns the node ids in planted order.
fn build_v3_store(db: &Path) -> Vec<String> {
    let storage = SqliteMemoryStore::new(Some(db.to_path_buf())).expect("open store");
    let rows = planted_rows();
    let mut ids = Vec::new();
    for (index, _) in rows.iter().enumerate() {
        let node = storage
            .ingest(IngestInput {
                content: format!("scheduling fixture memory {index}"),
                node_type: "fact".to_string(),
                ..Default::default()
            })
            .expect("ingest");
        ids.push(node.id);
    }
    {
        let conn = rusqlite::Connection::open(db).expect("raw sqlite");
        let created = (source_clock() - Duration::days(60)).to_rfc3339();
        for (id, row) in ids.iter().zip(&rows) {
            conn.execute(
                "UPDATE knowledge_nodes SET stability = ?1, difficulty = ?2, reps = ?3,
                     lapses = ?4, learning_state = ?5, sentiment_magnitude = ?6,
                     last_accessed = ?7, created_at = ?8, updated_at = ?8
                 WHERE id = ?9",
                rusqlite::params![
                    row.stability,
                    row.difficulty,
                    row.reps,
                    row.lapses,
                    row.learning_state,
                    row.sentiment_magnitude,
                    row.last_accessed.to_rfc3339(),
                    created,
                    id,
                ],
            )
            .expect("plant scheduling");
        }
        conn.execute("DELETE FROM fsrs_cards", [])
            .expect("no cards");
        conn.execute(
            "INSERT OR REPLACE INTO fsrs_config (key, value, updated_at) VALUES ('w20', ?1, ?2)",
            rusqlite::params![W20, created],
        )
        .expect("personalized w20");
    }
    drop(storage);
    ids
}

/// v3's own view of a row at `at`: `apply_decay`'s retrievability.
fn v3_retrievability(row: &Planted, at: DateTime<Utc>) -> f64 {
    let days = (at - row.last_accessed).num_seconds() as f64 / 86_400.0;
    vestige_core::retrievability_with_decay(
        row.stability * (1.0 + row.sentiment_magnitude * 0.5),
        days,
        W20,
    )
}

/// The store's card for a record, as `strata-store` folds it.
fn card_of(record: &FsrsStateRecord, last_seq: u64) -> CardState {
    CardState {
        stability_q: record.stability_q,
        difficulty_q: record.difficulty_q,
        last_seq,
        review_count: record.review_count,
        lapse_count: record.lapse_count,
        phase: record.phase,
    }
}

/// Strata's retrievability for an imported card at `at`.
fn strata_retrievability(record: &FsrsStateRecord, at: DateTime<Utc>) -> f64 {
    FsrsFold::retrievability_at_review(
        &card_of(record, 1),
        Some(record.reviewed_at_ms),
        at.timestamp_millis(),
        1,
        ALGO_V2,
    )
    .unwrap()
}

fn fresh(tag: &str) -> (tempfile::TempDir, PathBuf, Vec<String>) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let db = tmp.path().join(format!("{tag}.db"));
    let ids = build_v3_store(&db);
    (tmp, db, ids)
}

#[test]
fn knowledge_nodes_only_source_imports_one_card_per_memory() {
    let (tmp, db, ids) = fresh("kn-only");
    let dry = migrate_with_options(
        &db,
        &tmp.path().join("dry"),
        MigrateOptions {
            dry_run: true,
            ..Default::default()
        },
    )
    .expect("dry run");
    assert_eq!(dry.fsrs_states, 4, "dry run plans one state per memory");

    let dir = tmp.path().join("strata");
    let report = migrate(&db, &dir).expect("migrate");
    assert!(report.verify_passed, "{report:?}");
    assert_eq!(report.nodes, 4);
    assert_eq!(
        report.fsrs_events, 0,
        "no fsrs_cards rows, no rating series"
    );
    assert_eq!(report.fsrs_states, 4);

    let log = StrataLog::open(&dir).expect("reopen");
    let snapshot = read_snapshot(&log).expect("snapshot");
    assert!(snapshot.reviews.is_empty());
    assert_eq!(snapshot.fsrs_states.len(), 4);
    assert_eq!(snapshot.params.as_ref().unwrap().params_id, PARAMS_ID);
    assert_eq!(PARAMS_ID, "v4-migrate/2");

    let frames = log.read_frames(1).expect("frames");
    let params = frames.iter().find(|f| f.kind == KIND_PARAMS).unwrap();
    assert_eq!(decode_params(&params.payload).unwrap().params_id, PARAMS_ID);
    let receipt_seq = frames
        .iter()
        .find(|f| f.kind == strata_migrate::KIND_MIGRATION_RECEIPT)
        .unwrap()
        .seq;
    assert!(
        frames
            .iter()
            .filter(|f| f.kind == KIND_FSRS_STATE)
            .all(|f| f.seq < receipt_seq),
        "state frames are migration frames, before the receipt"
    );

    let rows = planted_rows();
    let clock = source_clock();
    for (id, row) in ids.iter().zip(&rows) {
        let node = snapshot.nodes.iter().find(|n| &n.legacy_id == id).unwrap();
        let record = snapshot
            .fsrs_states
            .iter()
            .find(|r| &r.legacy_id == id)
            .expect("one state per memory");
        assert_eq!(record.kernel_id, node.kernel_id);
        assert_eq!(record.algo_version, ALGO_V2);
        assert_eq!(record.reviewed_at_ms, row.last_accessed.timestamp_millis());
        assert_eq!(record.reviewed_at_ms, node.last_accessed_ms);
        assert_eq!(record.v3_decay_q, to_q32_32(W20));
        assert_eq!(record.review_count as i64, row.reps);
        assert_eq!(record.lapse_count as i64, row.lapses.min(row.reps));
        assert_eq!(record.difficulty_q, to_q32_32(row.difficulty));

        // Fit clock: the source clock, or a day after a younger access.
        let expected_fit = clock
            .timestamp_millis()
            .max(row.last_accessed.timestamp_millis() + DAY_MS);
        assert_eq!(record.fitted_at_ms, expected_fit);

        // Strata's retrievability equals v3's at the fit clock.
        let at = DateTime::from_timestamp_millis(record.fitted_at_ms).unwrap();
        let v3 = v3_retrievability(row, at);
        let strata = strata_retrievability(record, at);
        assert!(
            (strata - v3).abs() < 1e-6,
            "{id}: strata {strata} vs v3 {v3} at the fit clock"
        );
        // The log keeps millisecond clocks; v3 kept microseconds. That moves
        // v3's whole-second age by at most one second.
        assert!((from_q32_32(record.v3_retrievability_q) - v3).abs() < 1e-6);
    }

    let phases: Vec<CardPhase> = ids
        .iter()
        .map(|id| {
            snapshot
                .fsrs_states
                .iter()
                .find(|r| &r.legacy_id == id)
                .unwrap()
                .phase
        })
        .collect();
    assert_eq!(
        phases,
        vec![
            CardPhase::Learning,
            CardPhase::Learning,
            CardPhase::Review,
            CardPhase::Relearning
        ]
    );
}

/// Twelve hours after the source clock. A card at least a day old reads
/// what v3 read at the fit clock, within the half day of decay since. A card
/// younger than a day reads 1.0: Strata counts review age in whole days, so
/// the gap there is v3's first-day decay, at most `1 - R_v3(1 day)`.
#[test]
fn imported_cards_track_v3_near_the_upgrade() {
    let (tmp, db, ids) = fresh("near");
    let dir = tmp.path().join("strata");
    migrate(&db, &dir).expect("migrate");
    let snapshot = read_snapshot(&StrataLog::open(&dir).unwrap()).unwrap();
    let rows = planted_rows();
    let later = source_clock() + Duration::hours(12);
    for (id, row) in ids.iter().zip(&rows) {
        let record = snapshot
            .fsrs_states
            .iter()
            .find(|r| &r.legacy_id == id)
            .unwrap();
        let v3 = v3_retrievability(row, later);
        let strata = strata_retrievability(record, later);
        if later - row.last_accessed >= Duration::days(1) {
            assert!(
                (strata - v3).abs() < 0.01,
                "{id}: strata {strata} vs v3 {v3} twelve hours after the source clock"
            );
        } else {
            assert_eq!(strata, 1.0, "{id}: under a day old");
            let one_day = v3_retrievability(row, row.last_accessed + Duration::days(1));
            assert!(v3 >= one_day && 1.0 - v3 <= 1.0 - one_day, "{id}: {v3}");
        }
    }
}

#[test]
fn imported_states_are_identical_across_runs() {
    let (tmp, db, _) = fresh("twice");
    migrate(&db, &tmp.path().join("a")).expect("first");
    migrate(&db, &tmp.path().join("b")).expect("second");
    let a = read_snapshot(&StrataLog::open(tmp.path().join("a")).unwrap()).unwrap();
    let b = read_snapshot(&StrataLog::open(tmp.path().join("b")).unwrap()).unwrap();
    assert_eq!(a.fsrs_states.len(), 4);
    assert_eq!(a.fsrs_states, b.fsrs_states);
}

/// A memory with an `fsrs_cards` row keeps its rating series and gets no
/// state frame. The rest still get one each.
#[test]
fn fsrs_cards_rows_keep_their_series_and_skip_the_state() {
    let (tmp, db, ids) = fresh("carded");
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute(
            "INSERT INTO fsrs_cards (memory_id, difficulty, stability, state, reps, lapses,
                 last_review, due_date, elapsed_days, scheduled_days)
             VALUES (?1, 5.0, 3.2, 'review', 3, 1, ?2, ?2, 1, 1)",
            rusqlite::params![ids[0], source_clock().to_rfc3339()],
        )
        .unwrap();
    }
    let dir = tmp.path().join("strata");
    let report = migrate(&db, &dir).expect("migrate");
    assert!(report.verify_passed);
    assert_eq!(report.fsrs_events, 3);
    assert_eq!(report.fsrs_states, 3);
    let snapshot = read_snapshot(&StrataLog::open(&dir).unwrap()).unwrap();
    assert!(snapshot.fsrs_states.iter().all(|r| r.legacy_id != ids[0]));
    assert_eq!(snapshot.reviews.len(), 3);
}

/// The committed v3.1.1 fixture predates the scheduling columns: its
/// uncarded rows import with v3's column defaults (stability 1.0,
/// difficulty 5.0, never reviewed).
#[test]
fn rows_without_scheduling_columns_import_v3_defaults() {
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v3.1.1-sample.sqlite");
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("strata");
    let report = migrate(&fixture, &dir).expect("migrate fixture");
    assert!(report.verify_passed);
    assert_eq!(report.fsrs_states, 3, "4 nodes, 1 carded");
    let snapshot = read_snapshot(&StrataLog::open(&dir).unwrap()).unwrap();
    assert_eq!(snapshot.fsrs_states.len(), 3);
    for record in &snapshot.fsrs_states {
        assert_eq!(record.difficulty_q, to_q32_32(5.0));
        assert_eq!(record.review_count, 0);
        assert_eq!(record.lapse_count, 0);
        assert_eq!(record.phase, CardPhase::Learning);
        assert_eq!(
            record.v3_decay_q,
            to_q32_32(vestige_core::fsrs::DEFAULT_DECAY)
        );
        let at = DateTime::from_timestamp_millis(record.fitted_at_ms).unwrap();
        let reviewed = DateTime::from_timestamp_millis(record.reviewed_at_ms).unwrap();
        let days = (at - reviewed).num_seconds() as f64 / 86_400.0;
        let v3 =
            vestige_core::retrievability_with_decay(1.0, days, vestige_core::fsrs::DEFAULT_DECAY);
        assert!((strata_retrievability(record, at) - v3).abs() < 1e-6);
    }
    assert!(snapshot
        .fsrs_states
        .iter()
        .all(|r| r.legacy_id != "11111111-1111-4111-8111-111111111111"));
}

/// One node stamped far in the future must not move the fit clock of the
/// rest of the store: cards are fitted no later than the moment of import.
#[test]
fn a_future_dated_node_does_not_move_the_fit_clock() {
    let (tmp, db, ids) = fresh("future");
    let future = Utc::now() + Duration::days(3650);
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute(
            "UPDATE knowledge_nodes SET updated_at = ?1 WHERE id = ?2",
            rusqlite::params![future.to_rfc3339(), ids[3]],
        )
        .unwrap();
    }
    let dir = tmp.path().join("strata");
    let report = migrate(&db, &dir).expect("migrate");
    let after = Utc::now().timestamp_millis();
    assert!(report.verify_passed);

    let snapshot = read_snapshot(&StrataLog::open(&dir).unwrap()).unwrap();
    let rows = planted_rows();
    let at = DateTime::from_timestamp_millis(after).unwrap();
    // The two memories at least a day old read what v3 reads now.
    for index in [0usize, 1] {
        let record = snapshot
            .fsrs_states
            .iter()
            .find(|r| r.legacy_id == ids[index])
            .unwrap();
        assert!(
            record.fitted_at_ms <= after,
            "{}: fitted {} after the import finished at {after}",
            ids[index],
            record.fitted_at_ms
        );
        let v3 = v3_retrievability(&rows[index], at);
        let strata = strata_retrievability(record, at);
        assert!(
            (strata - v3).abs() < 0.01,
            "{}: strata {strata} vs v3 {v3} at import time",
            ids[index]
        );
    }
}
