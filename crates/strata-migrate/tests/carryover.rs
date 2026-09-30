//! v3 `intentions` rows reach `MigrateOptions::carry_over` decoded from the
//! snapshot the receipt hashes. The committed fixture has no intentions
//! table, so each test plants one with the v3 DDL (V3 plus V37 `scope`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use strata_migrate::{migrate_with_options, Carryover, IntentionRow, MigrateOptions};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v3.1.1-sample.sqlite")
}

/// v3 `MIGRATION_V3_UP` intentions table plus `MIGRATION_V37_UP`.
const INTENTIONS_DDL: &str = r#"
CREATE TABLE intentions (
    id TEXT PRIMARY KEY,
    content TEXT NOT NULL,
    trigger_type TEXT NOT NULL,
    trigger_data TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 2,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL,
    deadline TEXT,
    fulfilled_at TEXT,
    reminder_count INTEGER DEFAULT 0,
    last_reminded_at TEXT,
    notes TEXT,
    tags TEXT DEFAULT '[]',
    related_memories TEXT DEFAULT '[]',
    snoozed_until TEXT,
    source_type TEXT NOT NULL DEFAULT 'api',
    source_data TEXT
);
ALTER TABLE intentions ADD COLUMN scope TEXT;
"#;

fn plant(dir: &Path) -> PathBuf {
    let db = dir.join("vestige.db");
    fs::copy(fixture_path(), &db).unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(INTENTIONS_DDL).unwrap();
    conn.execute(
        "INSERT INTO intentions (id, content, trigger_type, trigger_data, priority, status,
            created_at, deadline, tags, related_memories, source_type, scope)
         VALUES ('int-a', 'Post the launch thread', 'time',
            '{\"type\":\"time\",\"at\":\"2026-03-12T00:00:00Z\"}', 4, 'active',
            '2026-01-27T04:45:43.353514+00:00', '2026-03-12T00:00:00+00:00',
            '[\"launch\"]', '[\"11111111-1111-4111-8111-111111111111\"]', 'mcp', 'vestige')",
        [],
    )
    .unwrap();
    // Defaults only: priority, status, reminder_count, tags, source_type.
    conn.execute(
        "INSERT INTO intentions (id, content, trigger_type, trigger_data, created_at)
         VALUES ('int-b', 'Legacy manual row', 'manual', '{}', '2026-02-04 01:18:28')",
        [],
    )
    .unwrap();
    db
}

fn int_a() -> IntentionRow {
    IntentionRow {
        id: "int-a".into(),
        content: "Post the launch thread".into(),
        trigger_type: "time".into(),
        trigger_data: r#"{"type":"time","at":"2026-03-12T00:00:00Z"}"#.into(),
        priority: 4,
        status: "active".into(),
        // v3 wrote microseconds; the row keeps milliseconds.
        created_at_ms: 1_769_489_143_353,
        deadline_ms: Some(1_773_273_600_000),
        fulfilled_at_ms: None,
        reminder_count: 0,
        last_reminded_at_ms: None,
        notes: None,
        tags: vec!["launch".into()],
        related_memories: vec!["11111111-1111-4111-8111-111111111111".into()],
        snoozed_until_ms: None,
        source_type: "mcp".into(),
        source_data: None,
        scope: Some("vestige".into()),
    }
}

fn int_b() -> IntentionRow {
    IntentionRow {
        id: "int-b".into(),
        content: "Legacy manual row".into(),
        trigger_type: "manual".into(),
        trigger_data: "{}".into(),
        priority: 2,
        status: "active".into(),
        created_at_ms: 1_770_167_908_000,
        deadline_ms: None,
        fulfilled_at_ms: None,
        reminder_count: 0,
        last_reminded_at_ms: None,
        notes: None,
        tags: Vec::new(),
        related_memories: Vec::new(),
        snoozed_until_ms: None,
        source_type: "api".into(),
        source_data: None,
        scope: None,
    }
}

#[test]
fn without_a_hook_intentions_stay_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let db = plant(dir.path());
    let report =
        migrate_with_options(&db, &dir.path().join("log"), MigrateOptions::default()).unwrap();
    assert!(report.verify_passed);
    assert_eq!(report.intentions_carried, 0);
    assert!(
        report.skipped_tables.iter().any(|t| t == "intentions"),
        "{:?}",
        report.skipped_tables
    );
}

#[test]
fn hook_receives_decoded_rows_and_clears_the_skip() {
    let dir = tempfile::tempdir().unwrap();
    let db = plant(dir.path());
    let dest = dir.path().join("log");
    let seen: Arc<Mutex<Option<(PathBuf, Carryover)>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let report = migrate_with_options(
        &db,
        &dest,
        MigrateOptions {
            carry_over: Some(Box::new(move |staging, carryover| {
                assert!(
                    staging.join("strata.key").is_file(),
                    "hook ran before the staged log existed"
                );
                *sink.lock().unwrap() = Some((staging.to_path_buf(), carryover.clone()));
                Ok(carryover.intentions.len() as u64)
            })),
            ..Default::default()
        },
    )
    .unwrap();

    let (staging, carryover) = seen.lock().unwrap().take().expect("hook ran");
    assert_ne!(staging, dest, "hook must run on staging, before the rename");
    assert_eq!(carryover.intentions, vec![int_a(), int_b()]);
    assert_eq!(report.intentions_carried, 2);
    assert!(
        !report.skipped_tables.iter().any(|t| t == "intentions"),
        "{:?}",
        report.skipped_tables
    );
    assert!(dest.join("strata.key").is_file());
}

#[test]
fn a_short_carry_leaves_no_log() {
    let dir = tempfile::tempdir().unwrap();
    let db = plant(dir.path());
    let dest = dir.path().join("log");
    let err = migrate_with_options(
        &db,
        &dest,
        MigrateOptions {
            carry_over: Some(Box::new(|_, carryover| {
                Ok(carryover.intentions.len() as u64 - 1)
            })),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("admitted 1 of 2"), "{err}");
    assert!(!dest.exists(), "a short carry published the log");
    assert!(!dir.path().join("log.strata-staging").exists());
}

#[test]
fn a_corrupt_intention_timestamp_stops_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let db = plant(dir.path());
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute(
            "UPDATE intentions SET deadline = 'next week' WHERE id = 'int-b'",
            [],
        )
        .unwrap();
    let dest = dir.path().join("log");
    let err = migrate_with_options(
        &db,
        &dest,
        MigrateOptions {
            carry_over: Some(Box::new(|_, carryover| {
                Ok(carryover.intentions.len() as u64)
            })),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("next week"), "{err}");
    assert!(!dest.exists());
}
