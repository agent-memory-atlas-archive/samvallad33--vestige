//! First-launch v3 → strata upgrade.
//!
//! The `vestige-upgrade` binary calls [`upgrade_if_needed`]. The v3 file is
//! only ever read. The import is `strata_migrate::migrate_with_options` into
//! `log/`: that function stages the log, holds `File::try_lock` until the
//! receipt is sealed, verifies with the v3 cross-check in `strata-verify`,
//! and renames. A dead owner's staging directory is wiped there, so SIGKILL
//! recovery is the same code. This crate decides that a v3 file needs that
//! import, copies the sqlite family, and records progress on stderr.
//! [`upgrade_with`] is the only path that calls `vestige_core::detect_v3`.
//!
//! v3 intentions have no migration frame. After the staged log verifies,
//! `carry_intentions` admits them into it through the store's normal
//! `UpsertIntentions` path (PROPOSE, GATE, EFFECT, so each row has a
//! receipt), reopens it to check every row replays, and only then does the
//! rename publish `log/`. A failure there discards staging like any other
//! import failure, so an installed `log/` always carries them.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use strata_migrate::{AnchorRow, Carryover, IntentionRow, MigrateOptions};
use strata_store::{AnchorRecord, EffectAction, IntentionRecord, StrataStore};

/// Installed strata log. Same relative path `StrataStore` opens.
/// Staging for this destination is `log` plus [`strata_migrate::STAGING_SUFFIX`]:
/// `<data-dir>/log.strata-staging`.
pub const LOG_DIR_NAME: &str = "log";
/// Append-only upgrade record. The failure message names this path.
pub const UPGRADE_LOG_NAME: &str = "upgrade.log";
/// Last v3 release operators can keep running when 4.0 cannot upgrade.
pub const V311_RELEASE: &str = "https://github.com/samvallad33/vestige/releases/tag/v3.1.1";

/// Backup plus staging log, relative to the sqlite family size.
const SPACE_FACTOR: u64 = 3;

/// Intention rows per admitted `UpsertIntentions` write. Bounds one data
/// frame; every row still gets its own receipt.
const INTENTION_BATCH: usize = 256;

/// What the boot path should do after the upgrade attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeStatus {
    /// No v3 SQLite file at the guarded path.
    NoV3,
    /// A strata log is installed (this launch, or a previous one).
    StrataReady { log_dir: PathBuf },
}

/// Upgrade failed. Display text is the process's stderr message.
#[derive(Debug)]
pub struct UpgradeError {
    log_path: PathBuf,
    detail: String,
}

impl std::fmt::Display for UpgradeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "upgrade failed: {detail}\nThe v3 data is untouched.\nYou can keep using v3.1.1: {V311_RELEASE}\nLog: {log}",
            detail = self.detail,
            log = self.log_path.display(),
        )
    }
}

impl std::error::Error for UpgradeError {}

/// Test seam: runs after a real import and before verify.
pub type AfterImport = Box<dyn FnOnce(&Path) -> io::Result<()>>;

/// Production passes [`UpgradeOptions::default`].
#[derive(Default)]
pub struct UpgradeOptions {
    pub after_import: Option<AfterImport>,
}

/// Upgrade `db_path` when it is a v3 store and no strata log is installed yet.
pub fn upgrade_if_needed(db_path: &Path) -> Result<UpgradeStatus, UpgradeError> {
    upgrade_with(db_path, UpgradeOptions::default())
}

/// Same as [`upgrade_if_needed`], with a post-import seam for tests.
pub fn upgrade_with(
    db_path: &Path,
    mut options: UpgradeOptions,
) -> Result<UpgradeStatus, UpgradeError> {
    let data_dir = data_dir_of(db_path);
    if !upgrade_relevant(&data_dir, db_path) {
        return Ok(UpgradeStatus::NoV3);
    }

    let log_dir = data_dir.join(LOG_DIR_NAME);
    let log_path = data_dir.join(UPGRADE_LOG_NAME);
    if let Some(status) = installed_log(&log_dir, &log_path) {
        return Ok(status);
    }

    let detected = match vestige_core::detect_v3(db_path) {
        Ok(v) => v,
        Err(e) => return Err(fail(&log_path, format!("v3 detection failed: {e}"))),
    };
    let Some(v3) = detected else {
        return Ok(UpgradeStatus::NoV3);
    };

    note(
        &log_path,
        &format!(
            "vestige: v3 store detected at {}{}; upgrading to strata",
            v3.path.display(),
            // Without rusqlite the detector only sees the header, where
            // Vestige leaves user_version at 0; the import reads the real
            // schema_version table.
            if v3.schema_version == 0 {
                String::new()
            } else {
                format!(" (schema {})", v3.schema_version)
            }
        ),
    );

    if let Err(detail) = ensure_space(&data_dir, db_path) {
        return Err(fail(&log_path, detail));
    }

    let log_path_hook = log_path.clone();
    let backup_db = db_path.to_path_buf();
    let backup_dir = data_dir.clone();
    let backup_log = log_path.clone();
    let carry_dir = data_dir.clone();
    let carry_log = log_path.clone();
    let after_import = options.after_import.take();
    let report = match strata_migrate::migrate_with_options(
        db_path,
        &log_dir,
        MigrateOptions {
            dry_run: false,
            accept_wal_snapshot: true,
            before_import: Some(Box::new(move |_staging| {
                sweep_partial_backups(&backup_dir);
                let backup = backup_sqlite_family(&backup_db)
                    .map_err(|err| format!("backup failed: {err}"))?;
                note(
                    &backup_log,
                    &format!(
                        "vestige: backed up {} -> {}",
                        backup_db.display(),
                        backup.display()
                    ),
                );
                Ok(())
            })),
            before_publish: Some(Box::new(move |staging| {
                note(
                    &log_path_hook,
                    &format!("vestige: importing into {}", staging.display()),
                );
                if let Some(hook) = after_import {
                    hook(staging).map_err(|err| format!("import failed: {err}"))?;
                }
                note(&log_path_hook, "vestige: verifying strata log");
                match strata_verify::migration::verify_migrated_log(staging) {
                    Ok(verified) if verified.ok => Ok(()),
                    Ok(verified) => Err(format!(
                        "strata-verify failed: {}",
                        verified.failures.join("; ")
                    )),
                    Err(err) => Err(format!("strata-verify failed: {err}")),
                }
            })),
            carry_over: Some(Box::new(move |staging, carryover: &Carryover| {
                if !carryover.intentions.is_empty() {
                    note(
                        &carry_log,
                        &format!(
                            "vestige: admitting {} intentions into {}",
                            carryover.intentions.len(),
                            staging.display()
                        ),
                    );
                }
                let carried = carry_intentions(&carry_dir, staging, &carryover.intentions)
                    .map_err(|err| format!("intention carry-over failed: {err}"))?;
                let (anchors, skipped) = carry_anchors(&carry_dir, staging, &carryover.anchors)
                    .map_err(|err| format!("code anchor carry-over failed: {err}"))?;
                if !carryover.anchors.is_empty() {
                    note(
                        &carry_log,
                        &format!(
                            "vestige: carried {anchors} code anchors ({skipped} belonged to suppressed or superseded memories)"
                        ),
                    );
                }
                Ok(carried)
            })),
            ..MigrateOptions::default()
        },
    ) {
        Ok(report) => report,
        Err(err) => return Err(fail(&log_path, format!("import failed: {err}"))),
    };
    if !report.verify_passed {
        return Err(fail(
            &log_path,
            "import failed: strata-migrate replay verification failed",
        ));
    }

    note(
        &log_path,
        &format!(
            "vestige: strata log ready at {} ({} memories, {} links, {} intentions imported)",
            log_dir.display(),
            report.nodes,
            report.edges,
            report.intentions_carried
        ),
    );
    if report.skipped_dangling_edges > 0 || report.skipped_dangling_cards > 0 {
        note(
            &log_path,
            &format!(
                "vestige: skipped {} links and {} review cards that pointed at deleted memories",
                report.skipped_dangling_edges, report.skipped_dangling_cards
            ),
        );
    }
    Ok(UpgradeStatus::StrataReady { log_dir })
}

/// Admit v3 intentions into the staged log at `staging` and prove they
/// replay. Returns how many rows the staged log holds.
///
/// Writes go through [`StrataStore::upsert_intentions`], the path the
/// intention tool uses, under the default policy the server pins. The store
/// is then reopened from the log alone: every row must come back
/// field-for-field with an admitted effect behind it, and no other intention
/// may be present. `data_dir` is where the published store keeps
/// `store.meta`, so this open fails wherever the server's would.
///
/// Staging is fresh on every attempt and a published `log/` is never
/// carried into again, so a relaunch cannot admit a row twice.
fn carry_intentions(data_dir: &Path, staging: &Path, rows: &[IntentionRow]) -> Result<u64, String> {
    if rows.is_empty() {
        return Ok(0);
    }
    let records: Vec<IntentionRecord> = rows.iter().map(intention_record).collect();
    let open = || {
        StrataStore::open_log_with_policy(data_dir, staging, strata_store::default_policy())
            .map_err(|err| format!("open staged store: {err}"))
    };
    {
        let mut store = open()?;
        for batch in records.chunks(INTENTION_BATCH) {
            store
                .upsert_intentions(batch.to_vec())
                .map_err(|err| format!("admit intentions: {err}"))?;
        }
    }

    let store = open()?;
    let proved: BTreeSet<String> = store
        .prove_effects()
        .map_err(|err| format!("prove intentions: {err}"))?
        .into_iter()
        .filter(|proof| proof.action == EffectAction::Intention)
        .map(|proof| proof.node_id)
        .collect();
    for record in &records {
        if store.get_intention(&record.id).as_ref() != Some(record) {
            return Err(format!("intention {} did not replay intact", record.id));
        }
        if !proved.contains(&record.id) {
            return Err(format!("intention {} has no admitted effect", record.id));
        }
    }
    let held = store.intentions().len();
    if held != records.len() {
        return Err(format!(
            "staged log holds {held} intentions, v3 had {}",
            records.len()
        ));
    }
    Ok(held as u64)
}

/// Admit v3 code anchors into the staged log through the store's anchor
/// write, the path the codebase tool uses, and prove they replay intact.
/// Anchors of a memory that is not live on the staged log (v3-suppressed or
/// superseded) are skipped: the store holds anchors only for live memories.
/// Returns `(carried, skipped)`.
fn carry_anchors(
    data_dir: &Path,
    staging: &Path,
    rows: &[AnchorRow],
) -> Result<(u64, u64), String> {
    if rows.is_empty() {
        return Ok((0, 0));
    }
    let open = || {
        StrataStore::open_log_with_policy(data_dir, staging, strata_store::default_policy())
            .map_err(|err| format!("open staged store: {err}"))
    };
    let mut store = open()?;
    let (live, skipped): (Vec<&AnchorRow>, Vec<&AnchorRow>) = rows.iter().partition(|row| {
        store
            .get_node(&row.node_id)
            .is_some_and(|node| node.is_live())
    });
    let records: Vec<AnchorRecord> = live.iter().map(|row| anchor_record(row)).collect();
    for batch in records.chunks(INTENTION_BATCH) {
        store
            .record_anchors(batch.to_vec())
            .map_err(|err| format!("admit anchors: {err}"))?;
    }
    drop(store);
    let store = open()?;
    for record in &records {
        if store.anchor(&record.id).as_ref() != Some(record) {
            return Err(format!("anchor {} did not replay intact", record.id));
        }
    }
    Ok((records.len() as u64, skipped.len() as u64))
}

fn anchor_record(row: &AnchorRow) -> AnchorRecord {
    AnchorRecord {
        id: row.id.clone(),
        node_id: row.node_id.clone(),
        file_path: row.file_path.clone(),
        symbol: row.symbol.clone(),
        symbol_kind: row.symbol_kind.clone(),
        start_line: row.start_line,
        end_line: row.end_line,
        span_lines: row.span_lines,
        content_hash: row.content_hash.clone(),
        captured_at_ms: row.captured_at_ms,
        last_verified_at_ms: row.last_verified_at_ms,
        last_status: row.last_status.clone(),
    }
}

fn intention_record(row: &IntentionRow) -> IntentionRecord {
    IntentionRecord {
        id: row.id.clone(),
        content: row.content.clone(),
        trigger_type: row.trigger_type.clone(),
        trigger_data: row.trigger_data.clone(),
        priority: row.priority,
        status: row.status.clone(),
        created_at_ms: row.created_at_ms,
        deadline_ms: row.deadline_ms,
        fulfilled_at_ms: row.fulfilled_at_ms,
        reminder_count: row.reminder_count,
        last_reminded_at_ms: row.last_reminded_at_ms,
        notes: row.notes.clone(),
        tags: row.tags.clone(),
        related_memories: row.related_memories.clone(),
        snoozed_until_ms: row.snoozed_until_ms,
        source_type: row.source_type.clone(),
        source_data: row.source_data.clone(),
        scope: row.scope.clone(),
    }
}

fn installed_log(log_dir: &Path, log_path: &Path) -> Option<UpgradeStatus> {
    if !strata_log_ready(log_dir) {
        return None;
    }
    note(
        log_path,
        &format!(
            "vestige: strata log already present at {}; skipping v3 upgrade",
            log_dir.display()
        ),
    );
    Some(UpgradeStatus::StrataReady {
        log_dir: log_dir.to_path_buf(),
    })
}

fn upgrade_relevant(data_dir: &Path, db_path: &Path) -> bool {
    db_path.exists() || data_dir.join(LOG_DIR_NAME).exists() || staging_directory(data_dir).exists()
}

/// `<data-dir>/log.strata-staging`. Same path [`strata_migrate`] publishes into
/// before renaming onto `log/`.
pub fn staging_directory(data_dir: &Path) -> PathBuf {
    let mut name = std::ffi::OsString::from(LOG_DIR_NAME);
    name.push(strata_migrate::STAGING_SUFFIX);
    data_dir.join(name)
}

fn data_dir_of(db_path: &Path) -> PathBuf {
    match db_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

pub fn strata_log_ready(log_dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(log_dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        name == "strata.key" || Path::new(&name).extension().and_then(|e| e.to_str()) == Some("seg")
    })
}

fn sweep_partial_backups(data_dir: &Path) {
    let Ok(entries) = fs::read_dir(data_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.contains(".v3-backup-") && name.ends_with(".partial") {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn ensure_space(data_dir: &Path, db_path: &Path) -> Result<(), String> {
    let need = sqlite_family_bytes(db_path).saturating_mul(SPACE_FACTOR);
    match free_bytes(data_dir) {
        Some(free) if free < need => Err(format!(
            "not enough free disk space ({free} bytes free, {need} bytes required)"
        )),
        _ => Ok(()),
    }
}

fn sqlite_family_bytes(db_path: &Path) -> u64 {
    sidecar_paths(db_path)
        .iter()
        .map(|path| fs::metadata(path).map(|m| m.len()).unwrap_or(0))
        .fold(0u64, u64::saturating_add)
}

fn sidecar_paths(db_path: &Path) -> Vec<PathBuf> {
    let mut paths = vec![db_path.to_path_buf()];
    for suffix in ["-wal", "-shm"] {
        let mut name = db_path.as_os_str().to_os_string();
        name.push(suffix);
        paths.push(PathBuf::from(name));
    }
    paths
}

fn backup_sqlite_family(db_path: &Path) -> io::Result<PathBuf> {
    // A retried upgrade (a client killed the first launch mid-import) must
    // not stack another full copy per attempt: reuse a finished backup whose
    // files are byte-identical to the store as it is now.
    if let Some(existing) = matching_backup(db_path)? {
        return Ok(existing);
    }
    let stamp = format!(
        "{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
        std::process::id()
    );
    let mut primary = None;
    for src in sidecar_paths(db_path) {
        if !src.exists() {
            continue;
        }
        let file_name = src.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "sqlite path has no file name")
        })?;
        let file_name = file_name.to_string_lossy();
        let parent = src.parent().unwrap_or_else(|| Path::new("."));
        let dest = parent.join(format!("{file_name}.v3-backup-{stamp}"));
        let partial = parent.join(format!("{file_name}.v3-backup-{stamp}.partial"));
        if let Err(e) = copy_file_fsync(&src, &partial) {
            let _ = fs::remove_file(&partial);
            return Err(e);
        }
        if let Err(e) = fs::rename(&partial, &dest) {
            let _ = fs::remove_file(&partial);
            return Err(e);
        }
        let _ = fsync_dir(parent);
        if src == db_path {
            primary = Some(dest);
        }
    }
    primary.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "v3 database disappeared before backup",
        )
    })
}

/// A finished backup (`<db>.v3-backup-<stamp>`, not `.partial`) whose primary
/// file and every present sidecar match the live family byte for byte.
fn matching_backup(db_path: &Path) -> io::Result<Option<PathBuf>> {
    let Some(parent) = db_path.parent() else {
        return Ok(None);
    };
    let Some(file_name) = db_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    else {
        return Ok(None);
    };
    let prefix = format!("{file_name}.v3-backup-");
    let Ok(entries) = fs::read_dir(parent) else {
        return Ok(None);
    };
    let mut stamps: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .filter_map(|name| name.strip_prefix(&prefix).map(str::to_owned))
        .filter(|stamp| !stamp.ends_with(".partial"))
        .collect();
    stamps.sort();
    'candidate: for stamp in stamps.iter().rev() {
        for src in sidecar_paths(db_path) {
            let Some(name) = src.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue 'candidate;
            };
            let copy = parent.join(format!("{name}.v3-backup-{stamp}"));
            match (src.exists(), copy.exists()) {
                (false, false) => continue,
                (true, true) if same_bytes(&src, &copy)? => continue,
                _ => continue 'candidate,
            }
        }
        return Ok(Some(parent.join(format!("{file_name}.v3-backup-{stamp}"))));
    }
    Ok(None)
}

fn same_bytes(a: &Path, b: &Path) -> io::Result<bool> {
    if fs::metadata(a)?.len() != fs::metadata(b)?.len() {
        return Ok(false);
    }
    let (mut a, mut b) = (
        io::BufReader::new(File::open(a)?),
        io::BufReader::new(File::open(b)?),
    );
    let (mut left, mut right) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    loop {
        let n = io::Read::read(&mut a, &mut left)?;
        if n == 0 {
            return Ok(true);
        }
        io::Read::read_exact(&mut b, &mut right[..n])?;
        if left[..n] != right[..n] {
            return Ok(false);
        }
    }
}

fn copy_file_fsync(src: &Path, dst: &Path) -> io::Result<()> {
    let mut input = File::open(src)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    // The backup holds every memory: owner-only, like the v3 store itself.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut output = options.open(dst)?;
    io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    Ok(())
}

fn fsync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

#[cfg(unix)]
fn free_bytes(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // Safety: `stat` is a valid out-pointer and `c_path` is a NUL-terminated path.
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };
    if rc != 0 {
        return None;
    }
    // Safety: `statvfs` returned 0, so it wrote a complete `statvfs`.
    let stat = unsafe { stat.assume_init() };
    // The field widths differ by platform: macOS has a u32 `f_bavail` and a
    // u64 `f_frsize`, Linux has both as u64. Widen both so one expression
    // compiles everywhere; on Linux the conversions are no-ops.
    #[allow(clippy::useless_conversion)]
    let frsize = u64::from(stat.f_frsize);
    #[allow(clippy::useless_conversion)]
    let avail = u64::from(stat.f_bavail);
    if frsize == 0 {
        return None;
    }
    Some(avail.saturating_mul(frsize))
}

#[cfg(not(unix))]
fn free_bytes(_dir: &Path) -> Option<u64> {
    None
}

fn note(log_path: &Path, line: &str) {
    eprintln!("{line}");
    let _ = io::stderr().flush();
    append_log(log_path, line);
}

fn append_log(log_path: &Path, line: &str) {
    let _ = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        writeln!(file, "{line}")?;
        file.sync_all()?;
        Ok(())
    })();
}

fn fail(log_path: &Path, detail: impl Into<String>) -> UpgradeError {
    let err = UpgradeError {
        log_path: log_path.to_path_buf(),
        detail: detail.into(),
    };
    append_log(log_path, &err.to_string());
    err
}
