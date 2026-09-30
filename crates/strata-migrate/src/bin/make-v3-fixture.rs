//! Generate the committed v3.1.1 migration fixture (synthetic rows only).
//!
//!     cargo run --manifest-path crates/strata-migrate/Cargo.toml \
//!         --bin make-v3-fixture -- crates/strata-migrate/tests/fixtures/v3.1.1-sample.sqlite
//!
//! The output is a v3.1.1-schema SQLite file (schema_version = 38) with a
//! handful of synthetic rows, a VALID two-entry `receipt_envelopes` chain
//! (digests computed with the real DSSE digest functions), one V40-style
//! `walk_receipts` row, and two `node_embeddings` rows whose values are
//! never read by the migrator (they are only counted as dropped_vectors).
//! NEVER point this at real user data, and never migrate a real store INTO
//! the fixture path.

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("v3.1.1-sample.sqlite"));
    if out.exists() {
        anyhow::bail!("refusing to overwrite {}", out.display());
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    build_fixture(&out)?;
    println!("fixture written: {}", out.display());
    Ok(())
}

/// Build the fixture at `path`. Public to the crate for future tooling.
fn build_fixture(path: &std::path::Path) -> anyhow::Result<()> {
    let conn = rusqlite::Connection::open(path)?;

    conn.execute_batch(
        r#"
        CREATE TABLE schema_version (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
        );
        INSERT INTO schema_version (version, applied_at) VALUES (38, '2026-09-28T00:00:00Z');

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

        CREATE TABLE sync_tombstones (
            table_name TEXT NOT NULL,
            row_id TEXT NOT NULL,
            deleted_at TEXT NOT NULL,
            reason TEXT,
            PRIMARY KEY (table_name, row_id)
        );

        CREATE TABLE deletion_tombstones (
            memory_id TEXT PRIMARY KEY,
            deleted_at TEXT NOT NULL,
            reason TEXT,
            node_type TEXT NOT NULL,
            tags TEXT NOT NULL DEFAULT '[]',
            edges_pruned INTEGER NOT NULL DEFAULT 0,
            insights_rewritten INTEGER NOT NULL DEFAULT 0,
            insights_deleted INTEGER NOT NULL DEFAULT 0,
            children_orphaned INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE node_embeddings (
            node_id TEXT PRIMARY KEY,
            embedding BLOB NOT NULL,
            dimensions INTEGER NOT NULL DEFAULT 768,
            model TEXT NOT NULL DEFAULT 'BAAI/bge-base-en-v1.5',
            created_at TEXT NOT NULL
        );

        CREATE TABLE receipt_envelopes (
            receipt_id               TEXT PRIMARY KEY,
            chain_id                 TEXT NOT NULL,
            sequence                 INTEGER NOT NULL,
            previous_entry_digest    TEXT,
            payload_type             TEXT NOT NULL,
            envelope_json            TEXT NOT NULL,
            payload_digest           TEXT NOT NULL,
            entry_digest             TEXT NOT NULL UNIQUE,
            signing_key_id           TEXT NOT NULL,
            signer_key_fingerprint   TEXT NOT NULL,
            issued_at                TEXT NOT NULL,
            stored_at                TEXT NOT NULL
        );

        CREATE TABLE walk_receipts (
            receipt_id     TEXT PRIMARY KEY,
            digest         TEXT NOT NULL,
            canonical_json TEXT NOT NULL,
            engine_version TEXT NOT NULL,
            created_at     TEXT NOT NULL
        );
        "#,
    )?;

    // ---- synthetic nodes -------------------------------------------------
    let nodes = [
        (
            "11111111-1111-4111-8111-111111111111",
            "Synthetic fact: migration fixtures are never real user data",
            "fact",
        ),
        (
            "22222222-2222-4222-8222-222222222222",
            "Synthetic fact: the v3 file stays byte-identical after migration",
            "fact",
        ),
        (
            "33333333-3333-4333-8333-333333333333",
            "Synthetic procedure: run migrate-to-strata once per store",
            "procedure",
        ),
        (
            "44444444-4444-4444-8444-444444444444",
            "Synthetic superseded note kept for lineage provenance",
            "note",
        ),
    ];
    for (id, content, node_type) in nodes {
        conn.execute(
            "INSERT INTO knowledge_nodes
             (id, content, node_type, created_at, updated_at, last_accessed, tags, source, superseded_by)
             VALUES (?1, ?2, ?3, '2026-01-15T10:00:00+00:00', '2026-02-20T11:30:00+00:00',
                     '2026-03-01T09:15:00+00:00', ?4, 'fixture', ?5)",
            rusqlite::params![
                id,
                content,
                node_type,
                if id.starts_with('1') {
                    r#"["fixture","synthetic"]"#.to_string()
                } else {
                    "[]".to_string()
                },
                if id.starts_with('4') {
                    Some("22222222-2222-4222-8222-222222222222".to_string())
                } else {
                    Option::<String>::None
                },
            ],
        )?;
    }

    // ---- synthetic edges: legacy types + one in-vocabulary type ----------
    let edges = [
        (
            "11111111-1111-4111-8111-111111111111",
            "22222222-2222-4222-8222-222222222222",
            0.9,
            "causal",
        ),
        (
            "22222222-2222-4222-8222-222222222222",
            "33333333-3333-4333-8333-333333333333",
            0.5,
            "semantic",
        ),
        (
            "11111111-1111-4111-8111-111111111111",
            "33333333-3333-4333-8333-333333333333",
            0.7,
            "touched",
        ),
    ];
    for (source, target, strength, link_type) in edges {
        conn.execute(
            "INSERT INTO memory_connections
             (source_id, target_id, strength, link_type, created_at, last_activated, activation_count)
             VALUES (?1, ?2, ?3, ?4, '2026-01-16T08:00:00+00:00', '2026-03-02T08:00:00+00:00', 2)",
            rusqlite::params![source, target, strength, link_type],
        )?;
    }

    // ---- one FSRS card ----------------------------------------------------
    conn.execute(
        "INSERT INTO fsrs_cards
         (memory_id, difficulty, stability, state, reps, lapses, last_review, due_date, elapsed_days, scheduled_days)
         VALUES ('11111111-1111-4111-8111-111111111111', 4.5, 12.25, 'review', 5, 2,
                 '2026-03-01T09:00:00+00:00', '2026-04-01T09:00:00+00:00', 3, 14)",
        [],
    )?;

    // ---- tombstones in both tables ----------------------------------------
    conn.execute(
        "INSERT INTO sync_tombstones (table_name, row_id, deleted_at, reason)
         VALUES ('knowledge_nodes', '99999999-9999-4999-8999-999999999999',
                 '2026-02-01T00:00:00+00:00', 'fixture deletion')",
        [],
    )?;
    conn.execute(
        "INSERT INTO deletion_tombstones
         (memory_id, deleted_at, reason, node_type, tags, edges_pruned, insights_rewritten,
          insights_deleted, children_orphaned)
         VALUES ('88888888-8888-4888-8888-888888888888', '2026-02-02T00:00:00+00:00',
                 'fixture purge', 'note', '[]', 1, 0, 1, 0)",
        [],
    )?;

    // ---- two dropped vectors ----------------------------------------------
    for id in [
        "11111111-1111-4111-8111-111111111111",
        "22222222-2222-4222-8222-222222222222",
    ] {
        conn.execute(
            "INSERT INTO node_embeddings (node_id, embedding, dimensions, model, created_at)
             VALUES (?1, ?2, 4, 'fixture-model', '2026-01-15T10:01:00+00:00')",
            rusqlite::params![id, vec![0u8; 16]],
        )?;
    }

    // ---- valid two-envelope receipt chain ----------------------------------
    // Digests are computed with the production DSSE digest functions so the
    // migrator's chain verification passes on the committed fixture.
    use base64::Engine as _;
    use vestige_core::storage::receipt_attestation::{entry_digest, payload_digest};

    let envelopes: [(&str, i64, Option<String>, &[u8]); 2] = [
        (
            "aaaaaaa1-0000-4000-8000-000000000001",
            0,
            None,
            b"synthetic receipt payload zero".as_slice(),
        ),
        (
            "aaaaaaa1-0000-4000-8000-000000000002",
            1,
            None, // previous filled below from entry 0
            b"synthetic receipt payload one".as_slice(),
        ),
    ];
    let mut prev_entry = String::new();
    for (receipt_id, sequence, _placeholder, payload) in envelopes {
        let payload_type = "https://vestige.dev/receipt/v1";
        let signature = [0xA5u8; 64];
        let key_id = "fixture-signing-key";
        let pd = payload_digest(payload);
        let ed = entry_digest(payload_type, payload, key_id, &signature);
        let previous = if sequence == 0 {
            None
        } else {
            Some(prev_entry.clone())
        };
        let envelope = serde_json::json!({
            "payloadType": payload_type,
            "payload": base64::engine::general_purpose::STANDARD.encode(payload),
            "signatures": [{ "keyid": key_id, "sig": base64::engine::general_purpose::STANDARD.encode(signature) }]
        });
        conn.execute(
            "INSERT INTO receipt_envelopes
             (receipt_id, chain_id, sequence, previous_entry_digest, payload_type, envelope_json,
              payload_digest, entry_digest, signing_key_id, signer_key_fingerprint, issued_at, stored_at)
             VALUES (?1, 'fixture-chain', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            rusqlite::params![
                receipt_id,
                sequence,
                previous,
                payload_type,
                envelope.to_string(),
                pd,
                ed,
                key_id,
                "f".repeat(64),
                "2026-03-05T12:00:00+00:00"
            ],
        )?;
        prev_entry = ed;
    }

    // ---- one V40 walk receipt ----------------------------------------------
    conn.execute(
        "INSERT INTO walk_receipts (receipt_id, digest, canonical_json, engine_version, created_at)
         VALUES ('bbbbbbb1-0000-4000-8000-000000000001',
                 'c0ffee', '{\"kind\":\"walk\",\"synthetic\":true}', 'v3-walk-1',
                 '2026-03-10T14:00:00+00:00')",
        [],
    )?;

    conn.pragma_update(None, "user_version", 38)?;
    Ok(())
}
