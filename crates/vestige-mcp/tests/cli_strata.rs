//! Real `vestige` binary against a Strata data directory: every subcommand
//! either runs on the log or refuses with `unavailable_in_4_0` /
//! `similarity_disabled` and names what works, and no refusal writes.

use std::path::Path;
use std::process::{Command, Output};
use std::sync::Arc;

use serde_json::Value;
use tempfile::TempDir;
use vestige_core::{IngestInput, Storage};

struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl Run {
    fn text(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }
}

fn vestige(data_dir: &Path, args: &[&str]) -> Run {
    let output: Output = Command::new(env!("CARGO_BIN_EXE_vestige"))
        .arg("--data-dir")
        .arg(data_dir)
        .args(args)
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .env_remove("VESTIGE_DATA_DIR")
        .output()
        .expect("spawn vestige");
    Run {
        ok: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn path_arg(path: &Path) -> &str {
    path.to_str().expect("utf-8 temp path")
}

fn open(dir: &Path) -> Arc<Storage> {
    vestige_mcp::strata_memory::open(dir).expect("open strata log")
}

fn put(storage: &Arc<Storage>, content: &str, tags: &[&str]) -> String {
    storage
        .ingest_in_scope(
            IngestInput {
                content: content.to_string(),
                tags: tags.iter().map(|t| t.to_string()).collect(),
                ..Default::default()
            },
            "user",
        )
        .expect("ingest")
        .id
}

fn link(storage: &Arc<Storage>, source: &str, target: &str, link_type: &str) {
    let now = chrono::Utc::now();
    storage
        .save_connection(&vestige_core::ConnectionRecord {
            source_id: source.to_string(),
            target_id: target.to_string(),
            strength: 1.0,
            link_type: link_type.to_string(),
            created_at: now,
            last_activated: now,
            activation_count: 0,
        })
        .expect("save edge");
}

/// Three memories in `user`: alpha and beta carry the tag, alpha -> beta is
/// a recorded edge, gamma is unlinked.
struct Seeded {
    _dir: TempDir,
    alpha: String,
    beta: String,
    gamma: String,
}

impl Seeded {
    fn path(&self) -> &Path {
        self._dir.path()
    }
}

fn seed() -> Seeded {
    let dir = TempDir::new().expect("temp dir");
    let storage = open(dir.path());
    let alpha = put(
        &storage,
        "CLI_STRATA_ALPHA login handler failed",
        &["cli-tag"],
    );
    let beta = put(
        &storage,
        "CLI_STRATA_BETA auth timeout flipped",
        &["cli-tag"],
    );
    let gamma = put(&storage, "CLI_STRATA_GAMMA unrelated note", &[]);
    link(&storage, &alpha, &beta, "derived_from");
    drop(storage);
    Seeded {
        _dir: dir,
        alpha,
        beta,
        gamma,
    }
}

fn node_count(dir: &Path) -> usize {
    open(dir).get_stats().expect("stats").total_nodes as usize
}

fn edge_count(dir: &Path) -> usize {
    open(dir).get_all_connections().expect("edges").len()
}

#[test]
fn backup_copies_the_strata_log_and_never_vestige_db() {
    let seeded = seed();
    // An upgraded store keeps its v3 file beside the published log.
    let v3 = seeded.path().join("vestige.db");
    let v3_bytes = b"SQLite format 3\0CLI_STRATA_V3_ONLY".to_vec();
    std::fs::write(&v3, &v3_bytes).unwrap();

    let out = TempDir::new().unwrap();
    let dest = out.path().join("bk");
    let run = vestige(seeded.path(), &["backup", path_arg(&dest)]);
    assert!(run.ok, "{}", run.text());
    assert!(
        run.stdout.contains("Sealing and copying the Strata log"),
        "{}",
        run.text()
    );
    assert!(!run.stdout.contains("SQLite snapshot"), "{}", run.text());
    assert!(run.stdout.contains("not the live store"), "{}", run.text());

    let segments = std::fs::read_dir(dest.join("log"))
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "seg"))
        .count();
    assert!(segments > 0, "backup has no log segments");
    assert!(!dest.join("vestige.db").exists());
    for entry in walk(&dest) {
        assert_ne!(
            std::fs::read(&entry).unwrap(),
            v3_bytes,
            "{entry:?} is vestige.db"
        );
    }
    assert_eq!(
        std::fs::read(&v3).unwrap(),
        v3_bytes,
        "vestige.db was modified"
    );

    // The copy is a working, verifiable log with every memory.
    let verify = Command::new(env!("CARGO_BIN_EXE_vestige"))
        .arg("strata-verify")
        .arg(&dest)
        .output()
        .unwrap();
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stdout)
    );
    assert_eq!(node_count(&dest), 3);

    // A second backup never mixes into an existing one.
    let again = vestige(seeded.path(), &["backup", path_arg(&dest)]);
    assert!(!again.ok);
    assert!(again.stderr.contains("already exists"), "{}", again.text());

    // Nor into the live log.
    let inside = seeded.path().join("log").join("bk");
    let nested = vestige(seeded.path(), &["backup", path_arg(&inside)]);
    assert!(!nested.ok);
    assert!(
        nested.stderr.contains("inside the live log"),
        "{}",
        nested.text()
    );
    assert!(!inside.exists());
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}

#[test]
fn backup_refuses_a_directory_with_no_store() {
    let empty = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let run = vestige(empty.path(), &["backup", path_arg(&out.path().join("bk"))]);
    assert!(!run.ok);
    assert!(run.stderr.contains("nothing to back up"), "{}", run.text());
    assert!(!empty.path().join("log").exists(), "a store was created");
}

#[test]
fn recall_resolves_exact_handles_and_refuses_free_text() {
    let seeded = seed();
    let dir = seeded.path();

    let by_id = vestige(dir, &["recall", "--handle", &seeded.alpha]);
    assert!(by_id.ok, "{}", by_id.text());
    assert!(
        by_id.stdout.contains("CLI_STRATA_ALPHA"),
        "{}",
        by_id.text()
    );
    assert!(by_id.stdout.contains("kind=memory"), "{}", by_id.text());
    assert!(by_id.stdout.contains("derived_from"), "{}", by_id.text());

    let json = vestige(dir, &["recall", "--handle", &seeded.alpha, "--json"]);
    assert!(json.ok, "{}", json.text());
    let value: Value = serde_json::from_str(&json.stdout).unwrap();
    assert_eq!(value["nodes"][0]["id"], seeded.alpha.as_str());
    assert_eq!(value["neighbors"][0]["to"], seeded.beta.as_str());

    let by_tag = vestige(dir, &["recall", "--handle", "cli-tag"]);
    assert!(by_tag.ok, "{}", by_tag.text());
    assert!(by_tag.stdout.contains("kind=tag"), "{}", by_tag.text());
    assert!(
        by_tag.stdout.contains("CLI_STRATA_BETA"),
        "{}",
        by_tag.text()
    );
    assert!(
        !by_tag.stdout.contains("CLI_STRATA_GAMMA"),
        "{}",
        by_tag.text()
    );

    // Seeded ids share a long prefix: a prefix is never guessed between them.
    let prefix = &seeded.alpha[..8];
    let ambiguous = vestige(dir, &["recall", "--handle", prefix]);
    assert!(!ambiguous.ok);
    assert!(
        ambiguous.stderr.contains("ambiguous"),
        "{}",
        ambiguous.text()
    );

    let unknown = vestige(dir, &["recall", "--handle", "no_such_handle"]);
    assert!(!unknown.ok);
    assert!(
        unknown.stderr.contains("handle_required"),
        "{}",
        unknown.text()
    );

    let free = vestige(dir, &["recall", "what failed near cli-tag"]);
    assert!(!free.ok);
    assert!(
        free.stderr.contains("similarity_disabled"),
        "{}",
        free.text()
    );
    assert!(
        free.stderr.contains("--handle cli-tag (tag, 2 memories)"),
        "{}",
        free.text()
    );
}

#[test]
fn backfill_refuses_and_names_causal_walk() {
    let seeded = seed();
    let run = vestige(seeded.path(), &["backfill", "--failure-id", &seeded.alpha]);
    assert!(!run.ok);
    assert!(run.stderr.contains("unavailable_in_4_0"), "{}", run.text());
    assert!(
        run.stderr.contains(&format!(
            "vestige causal-walk --logged-write {}",
            seeded.alpha
        )),
        "{}",
        run.text()
    );
}

#[test]
fn causal_walk_on_strata_walks_recorded_edges_only_and_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let (effect, cause, decoy) =
        vestige_mcp::tools::causal_walk::seed_recorded_cause(dir.path()).expect("seed");
    let edges_before = edge_count(dir.path());

    let json = vestige(
        dir.path(),
        &["causal-walk", "--logged-write", &effect, "--json"],
    );
    assert!(json.ok, "{}", json.text());
    let value: Value = serde_json::from_str(&json.stdout).unwrap();
    let causes: Vec<&str> = value["causes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["id"].as_str())
        .collect();
    assert_eq!(causes, vec![cause.as_str()], "{value}");
    assert!(!causes.contains(&decoy.as_str()));

    // Promote is the CLI default; a Strata walk still persists nothing.
    let human = vestige(dir.path(), &["causal-walk", "--logged-write", &effect]);
    assert!(human.ok, "{}", human.text());
    assert!(human.stdout.contains(&cause), "{}", human.text());
    assert!(
        human.stdout.contains("persists nothing"),
        "{}",
        human.text()
    );
    assert_eq!(edge_count(dir.path()), edges_before);

    let named = vestige(dir.path(), &["causal-walk", "--failing-test", "t_login"]);
    assert!(!named.ok);
    assert!(
        named.stderr.contains("unavailable_in_4_0"),
        "{}",
        named.text()
    );
    assert!(named.stderr.contains("--logged-write"), "{}", named.text());
}

#[test]
fn portable_and_sync_refuse_and_write_nothing() {
    let seeded = seed();
    let out = TempDir::new().unwrap();

    let archive = out.path().join("sub").join("p.json");
    let export = vestige(seeded.path(), &["portable-export", path_arg(&archive)]);
    assert!(!export.ok);
    assert!(
        export.stderr.contains("unavailable_in_4_0"),
        "{}",
        export.text()
    );
    assert!(
        export.stderr.contains("vestige export"),
        "{}",
        export.text()
    );
    assert!(
        !out.path().join("sub").exists(),
        "portable-export created files"
    );

    let sync_file = out.path().join("sync.json");
    let sync = vestige(seeded.path(), &["sync", path_arg(&sync_file)]);
    assert!(!sync.ok);
    assert!(
        sync.stderr.contains("unavailable_in_4_0"),
        "{}",
        sync.text()
    );
    assert!(!sync_file.exists(), "sync created the archive");

    let portable = out.path().join("portable.json");
    std::fs::write(&portable, portable_archive()).unwrap();
    let import = vestige(seeded.path(), &["portable-import", path_arg(&portable)]);
    assert!(!import.ok);
    assert!(
        import.stderr.contains("unavailable_in_4_0"),
        "{}",
        import.text()
    );
    assert_eq!(node_count(seeded.path()), 3);
}

fn portable_archive() -> String {
    serde_json::json!({
        "archiveFormat": vestige_core::PORTABLE_ARCHIVE_FORMAT,
        "vestigeVersion": "3.1.1",
        "schemaVersion": 20,
        "exportedAt": "2026-09-01T00:00:00Z",
        "mode": "exact",
        "tables": [],
    })
    .to_string()
}

#[test]
fn restore_reingests_exports_and_refuses_everything_else() {
    let seeded = seed();
    let out = TempDir::new().unwrap();

    // A Strata backup is restored by copying log/, not by this command.
    let backup = out.path().join("bk");
    assert!(vestige(seeded.path(), &["backup", path_arg(&backup)]).ok);
    let from_dir = vestige(seeded.path(), &["restore", path_arg(&backup)]);
    assert!(!from_dir.ok);
    assert!(
        from_dir.stderr.contains("unavailable_in_4_0"),
        "{}",
        from_dir.text()
    );
    assert!(from_dir.stderr.contains("log/"), "{}", from_dir.text());

    let portable = out.path().join("portable.json");
    std::fs::write(&portable, portable_archive()).unwrap();
    let from_portable = vestige(seeded.path(), &["restore", path_arg(&portable)]);
    assert!(!from_portable.ok);
    assert!(
        from_portable.stderr.contains("unavailable_in_4_0"),
        "{}",
        from_portable.text()
    );

    let sqlite = out.path().join("old.db");
    std::fs::write(&sqlite, b"SQLite format 3\0rest").unwrap();
    let from_sqlite = vestige(seeded.path(), &["restore", path_arg(&sqlite)]);
    assert!(!from_sqlite.ok);
    assert!(
        from_sqlite.stderr.contains("upgrade"),
        "{}",
        from_sqlite.text()
    );
    assert_eq!(node_count(seeded.path()), 3);

    // `vestige export` json re-ingests into another store as new records.
    let exported = out.path().join("export.json");
    assert!(vestige(seeded.path(), &["export", path_arg(&exported)]).ok);
    let target = TempDir::new().unwrap();
    let restored = vestige(target.path(), &["restore", path_arg(&exported)]);
    assert!(restored.ok, "{}", restored.text());
    assert!(
        restored.stdout.contains("new record"),
        "{}",
        restored.text()
    );
    assert!(
        !restored.stdout.contains("embeddings"),
        "{}",
        restored.text()
    );
    assert_eq!(node_count(target.path()), 3);
}

#[test]
fn ingest_backdating_ingest_git_and_gc_refuse_before_writing() {
    let seeded = seed();
    let dir = seeded.path();

    let backdated = vestige(dir, &["ingest", "CLI_STRATA_OLD", "--ago-days", "3"]);
    assert!(!backdated.ok);
    assert!(
        backdated.stderr.contains("unavailable_in_4_0"),
        "{}",
        backdated.text()
    );
    let stamped = vestige(
        dir,
        &[
            "ingest",
            "CLI_STRATA_OLD",
            "--created-at",
            "2026-01-01T00:00:00Z",
        ],
    );
    assert!(!stamped.ok);
    assert_eq!(node_count(dir), 3, "a refused ingest left a memory behind");

    let plain = vestige(dir, &["ingest", "CLI_STRATA_NOW"]);
    assert!(plain.ok, "{}", plain.text());
    assert_eq!(node_count(dir), 4);

    let git = vestige(dir, &["ingest-git", path_arg(dir)]);
    assert!(!git.ok);
    assert!(git.stderr.contains("unavailable_in_4_0"), "{}", git.text());

    let gc = vestige(dir, &["gc", "--yes", "--min-retention", "1.1"]);
    assert!(!gc.ok);
    assert!(gc.stderr.contains("withheld"), "{}", gc.text());
    let dry = vestige(dir, &["gc", "--dry-run", "--min-retention", "1.1"]);
    assert!(dry.ok, "{}", dry.text());
    assert!(
        dry.stdout.contains("Below threshold: 4 / 4"),
        "{}",
        dry.text()
    );
    assert!(!dry.stdout.contains("would be deleted"), "{}", dry.text());
    assert_eq!(node_count(dir), 4);
}

#[test]
fn compose_runs_both_ghostlink_lenses_with_proofs() {
    let seeded = seed();
    let sorted = |a: &str, b: &str| {
        let mut pair = [a.to_string(), b.to_string()];
        pair.sort();
        (pair[0].clone(), pair[1].clone())
    };
    let pairs_of = |answer: &Value| -> Vec<(String, String)> {
        answer["candidates"]
            .as_array()
            .unwrap_or_else(|| panic!("no candidates: {answer}"))
            .iter()
            .map(|c| {
                sorted(
                    c["firstId"].as_str().unwrap(),
                    c["secondId"].as_str().unwrap(),
                )
            })
            .collect()
    };

    // Bridge (default): alpha -derived_from-> beta is one typed hop and was
    // never woven, so it is the one pair, carried with its path.
    let bridge = vestige(seeded.path(), &["compose", "--limit", "10", "--json"]);
    assert!(bridge.ok, "{}", bridge.text());
    let bridge: Value = serde_json::from_str(&bridge.stdout).unwrap();
    assert_eq!(bridge["lens"], "bridge", "{bridge}");
    assert_eq!(
        pairs_of(&bridge),
        vec![sorted(&seeded.alpha, &seeded.beta)],
        "{bridge}"
    );
    let only = &bridge["candidates"][0];
    assert_eq!(only["proof"]["hops"], 1, "{only}");
    assert!(
        only["reason"]
            .as_str()
            .unwrap()
            .contains("1 typed-edge hop"),
        "{only}"
    );

    // Divergent: the linked pair is never eligible; gamma, joined to nothing,
    // is paired once on the page as a forced juxtaposition with no score.
    let divergent = vestige(
        seeded.path(),
        &["compose", "--lens", "divergent", "--limit", "10", "--json"],
    );
    assert!(divergent.ok, "{}", divergent.text());
    let divergent: Value = serde_json::from_str(&divergent.stdout).unwrap();
    let pairs = pairs_of(&divergent);
    assert!(
        !pairs.contains(&sorted(&seeded.alpha, &seeded.beta)),
        "linked pair listed: {divergent}"
    );
    assert_eq!(pairs.len(), 1, "each memory once per page: {divergent}");
    assert!(
        pairs[0].0 == seeded.gamma || pairs[0].1 == seeded.gamma,
        "{divergent}"
    );
    assert!(divergent["candidates"][0]["score"].is_null(), "{divergent}");

    // Human output names the lens and the reason; an unknown lens fails.
    let human = vestige(seeded.path(), &["compose"]);
    assert!(human.ok, "{}", human.text());
    assert!(human.stdout.contains("bridge lens"), "{}", human.text());
    assert!(human.stdout.contains("typed-edge hop"), "{}", human.text());
    let bad = vestige(seeded.path(), &["compose", "--lens", "nearest"]);
    assert!(!bad.ok, "{}", bad.text());
    assert!(bad.text().contains("unknown lens"), "{}", bad.text());
}

#[test]
fn health_consolidate_and_upgrade_describe_the_strata_log() {
    let seeded = seed();
    let dir = seeded.path();

    let health = vestige(dir, &["health"]);
    assert!(health.ok, "{}", health.text());
    assert!(
        health.stdout.contains("exact handles only"),
        "{}",
        health.text()
    );
    assert!(
        !health.stdout.contains("Embedding Coverage"),
        "{}",
        health.text()
    );
    assert!(
        !health.stdout.contains("keyword search only"),
        "{}",
        health.text()
    );
    assert!(!health.stdout.contains("consolidat"), "{}", health.text());

    let consolidate = vestige(dir, &["consolidate"]);
    assert!(consolidate.ok, "{}", consolidate.text());
    assert!(
        consolidate.stdout.contains("no-op"),
        "{}",
        consolidate.text()
    );

    let v3 = dir.join("vestige.db");
    std::fs::write(&v3, b"SQLite format 3\0kept").unwrap();
    for args in [&["upgrade", "--dry-run"][..], &["upgrade"][..]] {
        let upgrade = vestige(dir, args);
        assert!(upgrade.ok, "{}", upgrade.text());
        assert!(
            upgrade.stdout.contains("Already upgraded"),
            "{}",
            upgrade.text()
        );
    }
    assert_eq!(std::fs::read(&v3).unwrap(), b"SQLite format 3\0kept");
}
