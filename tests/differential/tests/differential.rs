//! Differential harness tests: cross-engine parity on a fixed script,
//! seeded fuzz parity, per-engine byte-identical reruns, and end-to-end
//! `verify_store` on the materialized strata store.

use vestige_differential_tests::{Op, Script, gen_script, run_sqlite, run_strata};

/// A fixed, hand-auditable script (25 ops: 8 ingests, 6 edges, 10 reviews,
/// 1 suppress). Ratings are drawn from {1,3,4} so the documented Hard-review
/// phase delta does not apply and FULL phase parity is assertable.
fn fixed_script() -> Script {
    let ingest = |i: usize| Op::Ingest {
        content: format!("fixed differential node {i}"),
        node_type: if i.is_multiple_of(2) { "fact" } else { "decision" }.to_string(),
        tags: vec!["fixed".to_string(), format!("n{i}")],
    };
    Script {
        seed: 0,
        ops: vec![
            ingest(0),
            ingest(1),
            ingest(2),
            ingest(3),
            ingest(4),
            ingest(5),
            ingest(6),
            ingest(7),
            Op::Edge {
                src: 0,
                dst: 1,
                link: "derived_from".into(),
            },
            Op::Review {
                node: 0,
                rating: 3,
                elapsed_seq: 2,
            },
            Op::Review {
                node: 1,
                rating: 1,
                elapsed_seq: 3,
            },
            Op::Edge {
                src: 2,
                dst: 0,
                link: "evidence_of".into(),
            },
            Op::Review {
                node: 2,
                rating: 4,
                elapsed_seq: 1,
            },
            Op::Review {
                node: 0,
                rating: 4,
                elapsed_seq: 5,
            },
            Op::Review {
                node: 3,
                rating: 1,
                elapsed_seq: 2,
            },
            Op::Edge {
                src: 3,
                dst: 4,
                link: "corrects".into(),
            },
            Op::Review {
                node: 4,
                rating: 3,
                elapsed_seq: 4,
            },
            Op::Review {
                node: 1,
                rating: 3,
                elapsed_seq: 6,
            },
            Op::Review {
                node: 5,
                rating: 1,
                elapsed_seq: 2,
            },
            Op::Review {
                node: 5,
                rating: 3,
                elapsed_seq: 3,
            },
            Op::Edge {
                src: 5,
                dst: 6,
                link: "touched".into(),
            },
            Op::Review {
                node: 6,
                rating: 4,
                elapsed_seq: 1,
            },
            Op::Review {
                node: 2,
                rating: 1,
                elapsed_seq: 7,
            },
            Op::Review {
                node: 3,
                rating: 3,
                elapsed_seq: 5,
            },
            Op::Edge {
                src: 7,
                dst: 0,
                link: "anchored_to".into(),
            },
            Op::Review {
                node: 7,
                rating: 3,
                elapsed_seq: 2,
            },
            Op::Suppress { node: 4 },
            Op::Review {
                node: 0,
                rating: 1,
                elapsed_seq: 4,
            },
        ],
    }
}

#[test]
fn fixed_script_cross_engine_full_parity() {
    let script = fixed_script();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let sqlite = run_sqlite(&script, a.path());
    let (strata, report) = run_strata(&script, b.path());
    sqlite.assert_parity(&strata);
    sqlite.assert_phase_parity(&strata); // ratings in {1,3,4}: no Hard delta
    assert!(report.ok(), "materialized store must verify: {report:?}");
    assert_eq!(sqlite.node_count, 8);
    assert_eq!(sqlite.edges_sorted.len(), 5);
    assert_eq!(sqlite.suppressed_sorted.len(), 1);
    assert_eq!(sqlite.reviews.len(), 8); // nodes 0..=7 are all reviewed
}

#[test]
fn fixed_script_two_runs_byte_identical_per_engine() {
    let script = fixed_script();
    let d1 = tempfile::tempdir().unwrap();
    let d2 = tempfile::tempdir().unwrap();
    let d3 = tempfile::tempdir().unwrap();
    let d4 = tempfile::tempdir().unwrap();
    let sqlite_a = run_sqlite(&script, d1.path());
    let sqlite_b = run_sqlite(&script, d2.path());
    let (strata_a, report_a) = run_strata(&script, d3.path());
    let (strata_b, report_b) = run_strata(&script, d4.path());
    assert_eq!(
        sqlite_a.canonical_bytes(),
        sqlite_b.canonical_bytes(),
        "two SQLite runs must be byte-identical (incl. quantized FSRS stream)"
    );
    assert_eq!(
        strata_a.canonical_bytes(),
        strata_b.canonical_bytes(),
        "two strata runs must be byte-identical (incl. quantized FSRS stream)"
    );
    assert!(report_a.ok() && report_b.ok());
}

#[test]
fn fuzz_seeded_scripts_parity_and_byte_identical_reruns() {
    for seed in [7u64, 42, 2026] {
        let script = gen_script(seed, 20);
        assert!(
            script.ops.len() >= 20,
            "seed {seed}: script must have >= 20 ops (has {})",
            script.ops.len()
        );
        // Two independent runs per engine in fresh directories.
        let dirs: Vec<_> = (0..4).map(|_| tempfile::tempdir().unwrap()).collect();
        let sqlite_a = run_sqlite(&script, dirs[0].path());
        let sqlite_b = run_sqlite(&script, dirs[1].path());
        let (strata_a, report_a) = run_strata(&script, dirs[2].path());
        let (strata_b, report_b) = run_strata(&script, dirs[3].path());

        assert_eq!(
            sqlite_a.canonical_bytes(),
            sqlite_b.canonical_bytes(),
            "seed {seed}: SQLite runs must be byte-identical"
        );
        assert_eq!(
            strata_a.canonical_bytes(),
            strata_b.canonical_bytes(),
            "seed {seed}: strata runs must be byte-identical"
        );
        sqlite_a.assert_parity(&strata_a);
        assert!(report_a.ok() && report_b.ok(), "seed {seed}: {report_a:?}");
    }
}

#[test]
fn strata_store_materializes_and_verifies_green_every_fuzz_seed() {
    for seed in 1u64..=6 {
        let script = gen_script(seed, 20);
        let dir = tempfile::tempdir().unwrap();
        let (snapshot, report) = run_strata(&script, dir.path());
        assert!(report.ok(), "seed {seed}: {report:?}");
        assert!(report.log_tail_ok);
        assert!(report.checkpoint_chain_ok);
        assert!(report.state_root_matches);
        assert!(report.gate_verdicts_rederived);
        assert!(report.gaps.is_empty());
        assert_eq!(
            snapshot.extras.get("verify_ok").map(String::as_str),
            Some("true")
        );
    }
}

#[test]
fn script_json_round_trip_matches_documented_shape() {
    let json = r#"{"seed": 9, "ops": [
        {"op": "ingest", "content": "c1", "node_type": "fact", "tags": ["t"]},
        {"op": "ingest", "content": "c2", "node_type": "decision", "tags": []},
        {"op": "edge", "src": 0, "dst": 1, "link": "derived_from"},
        {"op": "review", "node": 0, "rating": 3, "elapsed_seq": 2},
        {"op": "suppress", "node": 1}
    ]}"#;
    let script = Script::from_json(json).expect("documented JSON shape parses");
    assert_eq!(script.ops.len(), 5);
    // And it runs green on both engines.
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let sqlite = run_sqlite(&script, a.path());
    let (strata, report) = run_strata(&script, b.path());
    sqlite.assert_parity(&strata);
    assert!(report.ok());
    // A bare array parses too.
    let bare = r#"[{"op": "ingest", "content": "x", "node_type": "fact", "tags": []}]"#;
    assert_eq!(Script::from_json(bare).unwrap().ops.len(), 1);
}
