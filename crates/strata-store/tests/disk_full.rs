//! A full volume refuses the write; it does not abort the process, and the
//! store keeps serving and recording once space is back.
//!
//! The simulated disk-full state is process-wide, so this file holds a
//! single test.

use std::path::PathBuf;

use strata::failpoints;
use strata_store::{IngestInput, StoreError, StrataStore};

fn input(content: &str) -> IngestInput {
    IngestInput {
        content: content.to_string(),
        source: None,
        source_updated_at_ms: None,
        node_type: String::new(),
        tags: Vec::new(),
        created_at_ms: Some(1_700_000_000_000),
        valid_from_ms: None,
        valid_until_ms: None,
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("strata-store-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn a_full_disk_refuses_the_write_and_the_store_keeps_working() {
    // Each budget stops the same write at a different frame: the proposal,
    // the gate verdict, the admitting effect, or the data frame itself.
    for budget in 0..=3u32 {
        let dir = temp_dir(&format!("full-{budget}"));
        let mut store = StrataStore::open(&dir).expect("open");
        let before = store.ingest(input("kept before the disk filled")).unwrap();

        failpoints::frame_writes_succeed(Some(budget));
        let refused = store.ingest(input("refused while the disk is full"));
        failpoints::frame_writes_succeed(None);
        match refused {
            Err(StoreError::Log(e)) => assert!(e.is_storage_full(), "budget {budget}: {e}"),
            other => panic!("budget {budget}: expected a storage-full error, got {other:?}"),
        }
        assert_eq!(
            store.node_count(),
            1,
            "budget {budget}: a refused write leaves no node"
        );

        // Space is back: the same store writes again, and a reopen replays
        // exactly what was acknowledged.
        let after = store.ingest(input("kept after space returned")).unwrap();
        assert_ne!(before, after);
        assert_eq!(store.node_count(), 2);
        drop(store);
        let reopened = StrataStore::open(&dir).expect("reopen");
        assert_eq!(reopened.node_count(), 2, "budget {budget}");
        assert!(reopened.get_node(&before).is_some());
        assert!(reopened.get_node(&after).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
