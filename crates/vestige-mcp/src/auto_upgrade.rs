//! First-launch v3 → strata upgrade.
//!
//! `vestige-mcp` and the `vestige` CLI both call [`upgrade_if_needed`] before
//! opening a store. The v3 file is only ever read. Import goes to a staging
//! directory. [`strata_verify::migration::verify_migrated_log`] reads that
//! directory in place and creates nothing. Only a passing verify renames
//! staging onto `log/`. Any failure deletes staging and leaves the v3 file
//! byte-identical.
//!
//! The winner creates `.strata-upgrade-staging` with `create_dir` and holds
//! `File::try_lock` on a file inside it. The kernel drops that lock when the
//! process dies, including SIGKILL. A later process that can take the lock
//! wipes the staging directory and retries. A process that finds the lock
//! held waits until the directory is renamed onto `log/` or a strata log is
//! already there, then opens that result.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use strata_migrate::MigrateOptions;

/// Staging directory inside the data dir. Removed on failure and on the next
/// launch, so a crash mid-upgrade retries instead of appending.
pub const STAGING_DIR_NAME: &str = ".strata-upgrade-staging";
/// Installed strata log. Same relative path `StrataStore` opens.
pub const LOG_DIR_NAME: &str = "log";
/// Lock file inside the staging directory. Held with `File::try_lock` for the
/// whole attempt. Not a pid file: the kernel releases it when the holder dies.
pub const LOCK_FILE_NAME: &str = ".upgrade.lock";
/// Append-only upgrade record. The failure message names this path.
pub const UPGRADE_LOG_NAME: &str = "upgrade.log";
/// Last v3 release operators can keep running when 4.0 cannot upgrade.
pub const V311_RELEASE: &str = "https://github.com/samvallad33/vestige/releases/tag/v3.1.1";

/// Backup plus staging log, relative to the sqlite family size.
const SPACE_FACTOR: u64 = 3;

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

    let staging = data_dir.join(STAGING_DIR_NAME);
    let log_dir = data_dir.join(LOG_DIR_NAME);
    let log_path = data_dir.join(UPGRADE_LOG_NAME);

    loop {
        if let Some(status) = installed_log(&log_dir, &log_path) {
            return Ok(status);
        }
        match fs::create_dir(&staging) {
            Ok(()) => {
                let lock = match lock_new_staging(&staging) {
                    Ok(lock) => lock,
                    Err(e) => {
                        let _ = remove_dir_if_exists(&staging);
                        return Err(UpgradeError {
                            log_path,
                            detail: format!("could not lock staging: {e}"),
                        });
                    }
                };
                return upgrade_holding_lock(db_path, &mut options, &data_dir, lock);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                match wait_or_reclaim(&staging, &log_dir, &log_path)? {
                    Wait::Ready(status) => return Ok(status),
                    Wait::Retry => continue,
                }
            }
            Err(e) => {
                return Err(UpgradeError {
                    log_path,
                    detail: format!("could not create staging: {e}"),
                });
            }
        }
    }
}

enum Wait {
    Ready(UpgradeStatus),
    Retry,
}

/// `create_dir` won. `lock` is the exclusive `try_lock` on the file inside it.
fn upgrade_holding_lock(
    db_path: &Path,
    options: &mut UpgradeOptions,
    data_dir: &Path,
    lock: File,
) -> Result<UpgradeStatus, UpgradeError> {
    let staging = data_dir.join(STAGING_DIR_NAME);
    let log_dir = data_dir.join(LOG_DIR_NAME);
    let log_path = data_dir.join(UPGRADE_LOG_NAME);
    sweep_partial_backups(data_dir);

    if strata_log_ready(&log_dir) {
        drop(lock);
        let _ = remove_dir_if_exists(&staging);
        return Ok(
            installed_log(&log_dir, &log_path).unwrap_or(UpgradeStatus::StrataReady { log_dir })
        );
    }

    let detected = match vestige_core::detect_v3(db_path) {
        Ok(v) => v,
        Err(e) => {
            return Err(fail_holding(
                lock,
                &log_path,
                &staging,
                format!("v3 detection failed: {e}"),
            ));
        }
    };
    let Some(v3) = detected else {
        drop(lock);
        let _ = remove_dir_if_exists(&staging);
        return Ok(UpgradeStatus::NoV3);
    };

    note(
        &log_path,
        &format!(
            "vestige: v3 store detected at {} (schema {}); upgrading to strata",
            v3.path.display(),
            v3.schema_version
        ),
    );

    if let Err(detail) = ensure_space(data_dir, db_path) {
        return Err(fail_holding(lock, &log_path, &staging, detail));
    }

    let backup = match backup_sqlite_family(db_path) {
        Ok(path) => path,
        Err(e) => {
            return Err(fail_holding(
                lock,
                &log_path,
                &staging,
                format!("backup failed: {e}"),
            ));
        }
    };
    note(
        &log_path,
        &format!(
            "vestige: backed up {} -> {}",
            db_path.display(),
            backup.display()
        ),
    );

    note(
        &log_path,
        &format!("vestige: importing into {}", staging.display()),
    );
    let migrate_options = MigrateOptions {
        dry_run: false,
        // Unattended upgrade of a store that still has a WAL. The snapshot
        // copy is the migrator's own path; the original bytes are not written.
        accept_wal_snapshot: true,
        seed: None,
    };
    let report = match strata_migrate::migrate_with_options(db_path, &staging, migrate_options) {
        Ok(report) => report,
        Err(e) => {
            return Err(fail_holding(
                lock,
                &log_path,
                &staging,
                format!("import failed: {e}"),
            ));
        }
    };

    if let Some(hook) = options.after_import.take()
        && let Err(e) = hook(&staging)
    {
        return Err(fail_holding(
            lock,
            &log_path,
            &staging,
            format!("import failed: {e}"),
        ));
    }

    note(&log_path, "vestige: verifying strata log");
    let mut problems = Vec::new();
    if !report.verify_passed {
        problems.push("strata-migrate replay verification failed".to_string());
    }
    match strata_verify::migration::verify_migrated_log(&staging) {
        Ok(verified) if verified.ok => {}
        Ok(verified) => problems.push(format!(
            "strata-verify failed: {}",
            verified.failures.join("; ")
        )),
        Err(e) => problems.push(format!("strata-verify failed: {e}")),
    }
    if !problems.is_empty() {
        return Err(fail_holding(lock, &log_path, &staging, problems.join("; ")));
    }

    // Hold the lock across the rename so a waiter cannot treat the directory
    // as abandoned and delete it. The lock file moves with the directory.
    if let Err(e) = fs::rename(&staging, &log_dir) {
        return Err(fail_holding(
            lock,
            &log_path,
            &staging,
            format!("could not swap staging into place: {e}"),
        ));
    }
    let _ = fs::remove_file(log_dir.join(LOCK_FILE_NAME));
    drop(lock);
    if let Err(e) = fsync_dir(data_dir) {
        note(
            &log_path,
            &format!("vestige: directory fsync after swap failed ({e}); log is in place"),
        );
    }
    note(
        &log_path,
        &format!("vestige: strata log ready at {}", log_dir.display()),
    );
    Ok(UpgradeStatus::StrataReady { log_dir })
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

fn lock_new_staging(staging: &Path) -> io::Result<File> {
    let path = staging.join(LOCK_FILE_NAME);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "staging lock already held",
        )),
        Err(TryLockError::Error(err)) => Err(err),
    }
}

enum Held {
    Acquired(File),
    Busy,
    Missing,
}

fn hold_or_err(staging: &Path, log_path: &Path) -> Result<Held, UpgradeError> {
    try_hold(staging).map_err(|err| UpgradeError {
        log_path: log_path.to_path_buf(),
        detail: format!("could not lock staging: {err}"),
    })
}

fn try_hold(staging: &Path) -> io::Result<Held> {
    let path = staging.join(LOCK_FILE_NAME);
    let file = match OpenOptions::new().read(true).write(true).open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Held::Missing),
        Err(e) => return Err(e),
    };
    match file.try_lock() {
        Ok(()) => Ok(Held::Acquired(file)),
        Err(TryLockError::WouldBlock) => Ok(Held::Busy),
        Err(TryLockError::Error(err)) => Err(err),
    }
}

/// How long a loser looks for the lock file after `create_dir` and before
/// `try_lock`. This covers that gap only. A held lock is not abandoned on a timer.
const LOCK_FILE_APPEAR: Duration = Duration::from_millis(200);

fn wait_or_reclaim(staging: &Path, log_dir: &Path, log_path: &Path) -> Result<Wait, UpgradeError> {
    match hold_or_err(staging, log_path)? {
        Held::Acquired(lock) => {
            let _ = remove_dir_if_exists(staging);
            drop(lock);
            return Ok(Wait::Retry);
        }
        Held::Missing if !lock_file_appears(staging, log_dir) => {
            return reclaim_if_abandoned(staging, log_dir, log_path);
        }
        Held::Missing | Held::Busy => {}
    }
    eprintln!(
        "vestige: waiting for the in-progress strata upgrade in {}",
        staging.display()
    );
    let _ = io::stderr().flush();
    loop {
        if let Some(status) = installed_log(log_dir, log_path) {
            return Ok(Wait::Ready(status));
        }
        if !staging.exists() {
            return Ok(Wait::Retry);
        }
        match hold_or_err(staging, log_path)? {
            Held::Acquired(lock) => {
                let _ = remove_dir_if_exists(staging);
                drop(lock);
                return Ok(Wait::Retry);
            }
            Held::Busy => thread::sleep(Duration::from_millis(20)),
            Held::Missing => {
                if !lock_file_appears(staging, log_dir) {
                    return reclaim_if_abandoned(staging, log_dir, log_path);
                }
            }
        }
    }
}

fn lock_file_appears(staging: &Path, log_dir: &Path) -> bool {
    let path = staging.join(LOCK_FILE_NAME);
    let started = std::time::Instant::now();
    while started.elapsed() < LOCK_FILE_APPEAR {
        if path.exists() {
            return true;
        }
        if !staging.exists() || strata_log_ready(log_dir) {
            return path.exists();
        }
        thread::sleep(Duration::from_millis(2));
    }
    path.exists()
}

fn reclaim_if_abandoned(
    staging: &Path,
    log_dir: &Path,
    log_path: &Path,
) -> Result<Wait, UpgradeError> {
    if let Some(status) = installed_log(log_dir, log_path) {
        return Ok(Wait::Ready(status));
    }
    if !staging.exists() {
        return Ok(Wait::Retry);
    }
    // The directory exists and nobody holds the lock file. The creator died
    // between `create_dir` and `try_lock`, or the file never appeared.
    let _ = remove_dir_if_exists(staging);
    Ok(Wait::Retry)
}

fn upgrade_relevant(data_dir: &Path, db_path: &Path) -> bool {
    db_path.exists()
        || data_dir.join(LOG_DIR_NAME).exists()
        || data_dir.join(STAGING_DIR_NAME).exists()
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

fn copy_file_fsync(src: &Path, dst: &Path) -> io::Result<()> {
    let mut input = File::open(src)?;
    let mut output = OpenOptions::new().write(true).create_new(true).open(dst)?;
    io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    Ok(())
}

fn fsync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

fn remove_dir_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
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
    let frsize = stat.f_frsize;
    let avail = stat.f_bavail;
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

fn fail_holding(
    lock: File,
    log_path: &Path,
    staging: &Path,
    detail: impl Into<String>,
) -> UpgradeError {
    let err = fail(log_path, staging, detail);
    drop(lock);
    err
}

fn fail(log_path: &Path, staging: &Path, detail: impl Into<String>) -> UpgradeError {
    let _ = remove_dir_if_exists(staging);
    let err = UpgradeError {
        log_path: log_path.to_path_buf(),
        detail: detail.into(),
    };
    append_log(log_path, &err.to_string());
    err
}
