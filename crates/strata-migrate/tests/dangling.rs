//! Rows that older v3 builds left pointing at deleted memories are skipped
//! and counted, not treated as corruption.

use std::path::Path;

use strata::StrataLog;
use strata_migrate::{migrate, migrate_with_options, read_snapshot, MigrateOptions};

const GONE: &str = "00000000-0000-4000-8000-00000000dead";

fn build(db: &Path) -> Vec<String> {
    let conn = rusqlite::Connection::open(db).expect("raw sqlite");
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = OFF;
        CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
        INSERT INTO schema_version (version, applied_at) VALUES (38, '2026-09-01T00:00:00+00:00');
        CREATE TABLE knowledge_nodes (
            id TEXT PRIMARY KEY,
            content TEXT NOT NULL,
            node_type TEXT NOT NULL DEFAULT 'fact',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            last_accessed TEXT NOT NULL,
            tags TEXT DEFAULT '[]',
            source TEXT,
            superseded_by TEXT
        );
        CREATE TABLE memory_connections (
            source_id TEXT NOT NULL,
            target_id TEXT NOT NULL,
            strength REAL NOT NULL,
            link_type TEXT NOT NULL,
            created_at TEXT NOT NULL,
            last_activated TEXT NOT NULL,
            activation_count INTEGER DEFAULT 0,
            PRIMARY KEY (source_id, target_id)
        );
        CREATE TABLE fsrs_cards (
            memory_id TEXT PRIMARY KEY,
            difficulty REAL NOT NULL DEFAULT 5.0,
            stability REAL NOT NULL DEFAULT 1.0,
            state TEXT NOT NULL DEFAULT 'new',
            reps INTEGER DEFAULT 0,
            lapses INTEGER DEFAULT 0,
            last_review TEXT,
            due_date TEXT,
            elapsed_days INTEGER DEFAULT 0,
            scheduled_days INTEGER DEFAULT 0
        );
        "#,
    )
    .unwrap();
    let now = "2026-09-01T00:00:00+00:00";
    let ids: Vec<String> = (0..3)
        .map(|i| {
            let id = format!("00000000-0000-4000-8000-00000000000{}", i + 1);
            conn.execute(
                "INSERT INTO knowledge_nodes (id, content, node_type, created_at, updated_at, last_accessed, tags)
                 VALUES (?1, ?2, 'fact', ?3, ?3, ?3, '[]')",
                rusqlite::params![id, format!("dangling fixture memory {i}"), now],
            )
            .expect("insert node");
            id
        })
        .collect();
    let edge = |s: &str, t: &str| {
        conn.execute(
            "INSERT INTO memory_connections (source_id, target_id, strength, link_type,
                 created_at, last_activated, activation_count)
             VALUES (?1, ?2, 0.5, 'touched', ?3, ?3, 1)",
            rusqlite::params![s, t, now],
        )
        .expect("plant edge");
    };
    edge(&ids[0], &ids[1]);
    edge(GONE, &ids[1]);
    edge(&ids[0], GONE);
    let card = |id: &str| {
        conn.execute(
            "INSERT INTO fsrs_cards (memory_id, difficulty, stability, state, reps, lapses,
                 last_review, due_date, elapsed_days, scheduled_days)
             VALUES (?1, 5.0, 3.2, 'review', 3, 1, ?2, ?2, 1, 1)",
            rusqlite::params![id, now],
        )
        .expect("plant card");
    };
    card(&ids[2]);
    card(GONE);
    ids
}

#[test]
fn dangling_edges_and_cards_are_skipped_and_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let db = tmp.path().join("dangling.db");
    let ids = build(&db);

    let dry = migrate_with_options(
        &db,
        &tmp.path().join("dry"),
        MigrateOptions {
            dry_run: true,
            ..Default::default()
        },
    )
    .expect("dry run tolerates dangling rows");
    assert_eq!(dry.edges, 1);
    assert_eq!(dry.skipped_dangling_edges, 2);
    assert_eq!(dry.skipped_dangling_cards, 1);

    let dir = tmp.path().join("strata");
    let report = migrate(&db, &dir).expect("dangling rows must not block the upgrade");
    assert!(report.verify_passed, "{report:?}");
    assert_eq!(report.edges, 1);
    assert_eq!(report.skipped_dangling_edges, 2);
    assert_eq!(report.skipped_dangling_cards, 1);
    assert_eq!(report.fsrs_events, 3, "only the live card's series");

    let snapshot = read_snapshot(&StrataLog::open(&dir).unwrap()).unwrap();
    assert_eq!(snapshot.nodes.len(), ids.len());
    assert_eq!(snapshot.edges.len(), 1);
}
