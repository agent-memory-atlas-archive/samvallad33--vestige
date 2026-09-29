//! Real v3 fixture upgrades: happy path, injected import/verify failures, and
//! a SIGKILL mid-upgrade followed by a relaunch.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use vestige_mcp::auto_upgrade::{
    self, LOG_DIR_NAME, STAGING_DIR_NAME, UPGRADE_LOG_NAME, UpgradeOptions, UpgradeStatus,
    V311_RELEASE,
};

fn fixture_db() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../strata-migrate/tests/fixtures/v3.1.1-sample.sqlite")
}

fn sha256_file(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap();
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn plant(dir: &Path) -> PathBuf {
    let db = dir.join("vestige.db");
    fs::copy(fixture_db(), &db).unwrap();
    db
}

fn assert_failure_message(err: &str, log_path: &Path) {
    assert!(err.contains("upgrade failed"), "{err}");
    assert!(err.contains("untouched"), "{err}");
    assert!(err.contains("v3.1.1"), "{err}");
    assert!(err.contains(V311_RELEASE), "{err}");
    assert!(err.contains(&log_path.display().to_string()), "{err}");
    let logged = fs::read_to_string(log_path).unwrap();
    assert!(logged.contains("upgrade failed"), "{logged}");
    assert!(logged.contains(V311_RELEASE), "{logged}");
}

fn snapshot(log_dir: &Path) -> strata_migrate::snapshot::Snapshot {
    let log = strata::StrataLog::open(log_dir).unwrap();
    strata_migrate::read_snapshot(&log).unwrap()
}

fn knowledge_ids(db: &Path) -> Vec<String> {
    let conn = rusqlite::Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    let mut stmt = conn
        .prepare("SELECT id FROM knowledge_nodes ORDER BY id")
        .unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

fn assert_fixture_landed(db: &Path, log_dir: &Path) {
    let verified = strata_verify::migration::verify_migrated_log(log_dir).unwrap();
    assert!(verified.ok, "{verified:?}");
    let snap = snapshot(log_dir);
    let ids = knowledge_ids(db);
    assert!(ids.len() >= 4, "fixture lost its memories");
    for id in &ids {
        let node = snap
            .nodes
            .iter()
            .find(|node| &node.legacy_id == id)
            .unwrap_or_else(|| panic!("dropped memory {id}"));
        if id.starts_with("11111111") {
            let stability = node
                .legacy
                .iter()
                .find(|(key, _)| key == "fsrs_cards.stability")
                .map(|(_, value)| value.as_str())
                .unwrap_or("");
            let parsed: f64 = stability.parse().unwrap_or(f64::NAN);
            assert!((parsed - 12.25).abs() < 1e-9, "stability {stability}");
        }
    }
    let causal = snap
        .edges
        .iter()
        .find(|edge| edge.legacy_link_type == "causal")
        .expect("causal link dropped");
    assert!(causal.legacy_inferred);
    assert_eq!(causal.link_type, "derived_from");
    let semantic = snap
        .edges
        .iter()
        .find(|edge| edge.legacy_link_type == "semantic")
        .expect("semantic link dropped");
    assert!(semantic.legacy_inferred);
    assert_eq!(semantic.link_type, "derived_from");
    let touched = snap
        .edges
        .iter()
        .find(|edge| edge.legacy_link_type == "touched")
        .expect("touched link dropped");
    assert!(!touched.legacy_inferred);
    assert_eq!(touched.link_type, "touched");
}

#[test]
fn happy_path_on_real_v3_fixture_keeps_the_source_hash() {
    let dir = tempfile::tempdir().unwrap();
    let db = plant(dir.path());
    let before = sha256_file(&db);
    // A crashed previous attempt must be discarded, not resumed.
    let staging = dir.path().join(STAGING_DIR_NAME);
    fs::create_dir_all(&staging).unwrap();
    fs::write(staging.join("leftover.seg"), b"not a log").unwrap();

    let status = auto_upgrade::upgrade_if_needed(&db).unwrap();
    let UpgradeStatus::StrataReady { log_dir } = status else {
        panic!("expected an installed strata log");
    };
    assert_eq!(before, sha256_file(&db), "upgrade modified the v3 file");
    assert!(!staging.exists(), "staging survived a successful swap");
    assert!(log_dir.join("strata.key").is_file() || dir_has_seg(&log_dir));
    let backups: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().contains(".v3-backup-"))
        .map(|entry| entry.path())
        .collect();
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert_eq!(
        before,
        sha256_file(&backups[0]),
        "backup is not a byte copy"
    );
    assert_fixture_landed(&db, &log_dir);

    let again = auto_upgrade::upgrade_if_needed(&db).unwrap();
    assert!(matches!(again, UpgradeStatus::StrataReady { .. }));
    assert_eq!(before, sha256_file(&db));
    let backups_after = fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().contains(".v3-backup-"))
        .count();
    assert_eq!(backups_after, 1, "relaunch took another backup");
}

#[test]
fn import_failure_leaves_the_v3_hash_and_names_v311() {
    let dir = tempfile::tempdir().unwrap();
    let db = plant(dir.path());
    let file = fs::OpenOptions::new().write(true).open(&db).unwrap();
    file.set_len(4096).unwrap();
    drop(file);
    let before = sha256_file(&db);
    let err = auto_upgrade::upgrade_if_needed(&db).unwrap_err();
    let text = err.to_string();
    assert_failure_message(&text, &dir.path().join(UPGRADE_LOG_NAME));
    assert!(text.contains("import failed"), "{text}");
    assert_eq!(before, sha256_file(&db));
    assert!(!dir.path().join(STAGING_DIR_NAME).exists());
    assert!(!dir.path().join(LOG_DIR_NAME).exists());
}

#[test]
fn verify_failure_leaves_the_v3_hash_and_names_v311() {
    let dir = tempfile::tempdir().unwrap();
    let db = plant(dir.path());
    let before = sha256_file(&db);
    let err = auto_upgrade::upgrade_with(
        &db,
        UpgradeOptions {
            after_import: Some(Box::new(|staging| {
                let seg = fs::read_dir(staging)?
                    .flatten()
                    .map(|entry| entry.path())
                    .find(|path| path.extension().and_then(|ext| ext.to_str()) == Some("seg"))
                    .ok_or_else(|| {
                        std::io::Error::new(std::io::ErrorKind::NotFound, "no segment to tamper")
                    })?;
                let mut bytes = fs::read(&seg)?;
                let index = bytes.len() / 2;
                bytes[index] ^= 0xff;
                fs::write(&seg, bytes)?;
                Ok(())
            })),
        },
    )
    .unwrap_err();
    let text = err.to_string();
    assert_failure_message(&text, &dir.path().join(UPGRADE_LOG_NAME));
    assert!(
        text.contains("strata-verify failed"),
        "verify injection did not fail closed: {text}"
    );
    assert_eq!(before, sha256_file(&db));
    assert!(!dir.path().join(STAGING_DIR_NAME).exists());
    assert!(!dir.path().join(LOG_DIR_NAME).exists());
}

struct Running {
    child: std::process::Child,
    stdout: Arc<Mutex<String>>,
    stderr: Arc<Mutex<String>>,
}

fn spawn_mcp(data_dir: &Path) -> Running {
    let mut child = Command::new(env!("CARGO_BIN_EXE_vestige-mcp"))
        .arg("--data-dir")
        .arg(data_dir)
        .env("HOME", data_dir)
        .env("VESTIGE_DASHBOARD_ENABLED", "false")
        .env("VESTIGE_HTTP_ENABLED", "0")
        .env("VESTIGE_AUTOPILOT_ENABLED", "0")
        .env("RUST_LOG", "error")
        .env_remove("VESTIGE_DATA_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn vestige-mcp");
    let stdout = Arc::new(Mutex::new(String::new()));
    let stderr = Arc::new(Mutex::new(String::new()));
    let mut out_pipe = child.stdout.take().expect("stdout");
    let out_buf = Arc::clone(&stdout);
    std::thread::spawn(move || {
        let mut tmp = String::new();
        let _ = out_pipe.read_to_string(&mut tmp);
        *out_buf.lock().unwrap() = tmp;
    });
    let mut err_pipe = child.stderr.take().expect("stderr");
    let err_buf = Arc::clone(&stderr);
    std::thread::spawn(move || {
        let mut tmp = String::new();
        let _ = err_pipe.read_to_string(&mut tmp);
        *err_buf.lock().unwrap() = tmp;
    });
    Running {
        child,
        stdout,
        stderr,
    }
}

fn dir_has_seg(dir: &Path) -> bool {
    fs::read_dir(dir).ok().is_some_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("seg"))
    })
}

#[test]
fn failed_import_process_exits_on_stderr_and_leaves_stdout_empty() {
    let dir = tempfile::tempdir().unwrap();
    let db = plant(dir.path());
    let file = fs::OpenOptions::new().write(true).open(&db).unwrap();
    file.set_len(4096).unwrap();
    drop(file);
    let before = sha256_file(&db);
    let mut running = spawn_mcp(dir.path());
    let status = running
        .child
        .wait_timeout_ext(Duration::from_secs(60))
        .expect("corrupt import should exit");
    assert_ne!(status.code(), Some(0), "corrupt import exited 0");
    std::thread::sleep(Duration::from_millis(50));
    let stdout = running.stdout.lock().unwrap().clone();
    let stderr = running.stderr.lock().unwrap().clone();
    assert!(
        stdout.trim().is_empty(),
        "upgrade wrote to stdout (MCP): {stdout}"
    );
    assert_failure_message(&stderr, &dir.path().join(UPGRADE_LOG_NAME));
    assert_eq!(before, sha256_file(&db));
    assert!(!dir.path().join(STAGING_DIR_NAME).exists());
}

trait WaitTimeout {
    fn wait_timeout_ext(&mut self, limit: Duration) -> Option<std::process::ExitStatus>;
}

impl WaitTimeout for std::process::Child {
    fn wait_timeout_ext(&mut self, limit: Duration) -> Option<std::process::ExitStatus> {
        let started = Instant::now();
        loop {
            if let Some(status) = self.try_wait().unwrap() {
                return Some(status);
            }
            if started.elapsed() > limit {
                let _ = self.kill();
                let _ = self.wait();
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn reset_attempt(data: &Path, db: &Path) {
    let _ = fs::remove_dir_all(data.join(LOG_DIR_NAME));
    let _ = fs::remove_dir_all(data.join(STAGING_DIR_NAME));
    let _ = fs::remove_dir_all(data.join(auto_upgrade::VERIFY_DIR_NAME));
    for entry in fs::read_dir(data).unwrap().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.contains(".v3-backup-") || name == UPGRADE_LOG_NAME || name == "receipt-signing.key"
        {
            let _ = fs::remove_file(entry.path());
        }
    }
    if db.exists() {
        let _ = fs::remove_file(db);
    }
    fs::copy(fixture_db(), db).unwrap();
}

fn inflate_nodes(db: &Path, extra: usize) {
    let conn = rusqlite::Connection::open(db).unwrap();
    let mut stmt = conn
        .prepare("SELECT * FROM knowledge_nodes LIMIT 1")
        .unwrap();
    let cols: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|c| (*c).to_string())
        .collect();
    let mut rows = stmt.query([]).unwrap();
    let row = rows.next().unwrap().unwrap();
    let mut values: Vec<rusqlite::types::Value> = Vec::new();
    for i in 0..cols.len() {
        values.push(row.get(i).unwrap());
    }
    drop(rows);
    drop(stmt);
    let placeholders = vec!["?"; cols.len()].join(",");
    let sql = format!(
        "INSERT INTO knowledge_nodes ({}) VALUES ({placeholders})",
        cols.join(",")
    );
    let id_idx = cols.iter().position(|c| c == "id").unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    for n in 0..extra {
        values[id_idx] = rusqlite::types::Value::Text(format!("aaaaaaaa-bbbb-4ccc-8ddd-{n:012x}"));
        tx.execute(&sql, rusqlite::params_from_iter(values.iter()))
            .unwrap();
    }
    tx.commit().unwrap();
}

#[test]
fn sigkill_mid_upgrade_then_relaunch_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    fs::create_dir_all(&data).unwrap();
    let db = data.join("vestige.db");
    let mut extra = 8_000usize;
    let killed = loop {
        if extra > 64_000 {
            panic!("could not SIGKILL mid-upgrade; import finished before a segment appeared");
        }
        reset_attempt(&data, &db);
        inflate_nodes(&db, extra);
        let before = sha256_file(&db);
        let mut running = spawn_mcp(&data);
        let started = Instant::now();
        let mut saw_segment = false;
        while started.elapsed() < Duration::from_secs(180) {
            if dir_has_seg(&data.join(STAGING_DIR_NAME)) {
                saw_segment = true;
                break;
            }
            if running.child.try_wait().unwrap().is_some() {
                break;
            }
            if dir_has_seg(&data.join(LOG_DIR_NAME)) && !data.join(STAGING_DIR_NAME).exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let _ = running.child.kill();
        let _ = running.child.wait();
        assert_eq!(
            before,
            sha256_file(&db),
            "SIGKILL path modified the v3 file"
        );
        if saw_segment && !dir_has_seg(&data.join(LOG_DIR_NAME)) {
            break before;
        }
        extra *= 2;
    };

    let before = killed;
    let mut again = spawn_mcp(&data);
    let started = Instant::now();
    let log_dir = data.join(LOG_DIR_NAME);
    loop {
        if dir_has_seg(&log_dir) && !data.join(STAGING_DIR_NAME).exists() {
            break;
        }
        if let Some(status) = again.child.try_wait().unwrap() {
            std::thread::sleep(Duration::from_millis(50));
            panic!(
                "relaunch exited {status} before the strata log was installed. stderr: {}",
                again.stderr.lock().unwrap()
            );
        }
        if started.elapsed() > Duration::from_secs(180) {
            let _ = again.child.kill();
            let _ = again.child.wait();
            panic!(
                "relaunch did not finish the upgrade. stderr: {}",
                again.stderr.lock().unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        again.child.try_wait().unwrap().is_none(),
        "relaunch exited after installing the log"
    );
    assert_eq!(before, sha256_file(&db), "relaunch modified the v3 file");
    assert_fixture_landed(&db, &log_dir);
    let ids = knowledge_ids(&db);
    let snap = snapshot(&log_dir);
    let imported = snap
        .nodes
        .iter()
        .filter(|node| ids.iter().any(|id| id == &node.legacy_id))
        .count();
    assert_eq!(imported, ids.len(), "relaunch dropped or duplicated nodes");
    let _ = again.child.kill();
    let _ = again.child.wait();
    std::thread::sleep(Duration::from_millis(50));
    let stdout = again.stdout.lock().unwrap().clone();
    assert!(
        stdout.trim().is_empty(),
        "upgrade wrote to stdout (MCP): {stdout}"
    );
}
