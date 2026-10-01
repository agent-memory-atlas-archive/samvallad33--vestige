//! v3 importer. Holds the store lock across the staged import and the rename
//! onto `log/`. The v3 file is only read.

use std::env;
use std::fs::{self, File, TryLockError};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use vestige_upgrade::{
    LOG_DIR_NAME, UpgradeStatus, strata_log_ready, upgrade_if_needed, write_stderr,
};

/// How long to wait for another process to release `.serve.lock`. The same
/// variable bounds `vestige-mcp`'s own election.
const DEFAULT_LOCK_WAIT: Duration = Duration::from_secs(120);
const LOCK_WAIT_ENV: &str = "VESTIGE_ATTACH_WAIT_SECS";
const LOCK_POLL: Duration = Duration::from_millis(100);

/// What waiting for the store lock came to.
enum Held {
    /// This process holds the lock for the import.
    Lock(File),
    /// A Strata log is already installed: nothing to import, no lock needed.
    LogInstalled,
}

fn main() -> ExitCode {
    let mut data_dir: Option<PathBuf> = None;
    // Arguments stay `OsString`: a data-dir path need not be valid UTF-8.
    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--help" | "-h") => {
                println!(
                    "vestige-upgrade --data-dir <DIR>\n\n\
                     Import <DIR>/vestige.db into a Strata log. The v3 file is not modified."
                );
                return ExitCode::SUCCESS;
            }
            Some("--version" | "-V") => {
                println!("vestige-upgrade {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            Some("--data-dir") => {
                let Some(value) = args.next() else {
                    write_stderr("vestige-upgrade: --data-dir needs a path");
                    return ExitCode::from(2);
                };
                data_dir = Some(PathBuf::from(value));
            }
            _ => {
                write_stderr(&format!(
                    "vestige-upgrade: unknown argument {}",
                    arg.to_string_lossy()
                ));
                return ExitCode::from(2);
            }
        }
    }

    let data_dir = data_dir.unwrap_or_else(default_data_dir);
    if let Err(err) = fs::create_dir_all(&data_dir) {
        write_stderr(&format!(
            "vestige-upgrade: failed to create {}: {err}",
            data_dir.display()
        ));
        return ExitCode::from(1);
    }
    // Same exclusive lock `vestige-mcp` holds while serving. Released on exit,
    // including SIGKILL. The staging rename runs under this lock.
    let _store_lock = match hold_store_lock(&data_dir, lock_wait()) {
        Ok(Held::Lock(file)) => file,
        Ok(Held::LogInstalled) => return ExitCode::SUCCESS,
        Err(code) => return code,
    };

    let db_path = data_dir.join("vestige.db");
    match upgrade_if_needed(&db_path) {
        Ok(UpgradeStatus::NoV3 | UpgradeStatus::StrataReady { .. }) => ExitCode::SUCCESS,
        Err(err) => {
            write_stderr(&err.to_string());
            ExitCode::from(1)
        }
    }
}

fn default_data_dir() -> PathBuf {
    if let Some(value) = env::var_os("VESTIGE_DATA_DIR")
        && !value.is_empty()
    {
        return PathBuf::from(value);
    }
    directories::ProjectDirs::from("com", "vestige", "core")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `VESTIGE_ATTACH_WAIT_SECS`, else [`DEFAULT_LOCK_WAIT`].
fn lock_wait() -> Duration {
    env::var(LOCK_WAIT_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_LOCK_WAIT)
}

/// Take the store lock, waiting at most `wait` for its holder. A Strata log
/// that is already installed ends the wait: there is nothing left to import.
fn hold_store_lock(data_dir: &Path, wait: Duration) -> Result<Held, ExitCode> {
    let path = data_dir.join(".serve.lock");
    let mut options = File::options();
    options.read(true).write(true).create(true).truncate(false);
    // Owner-only, like vestige-mcp's: a lock other users can open is one
    // they can hold to lock the owner out.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = match options.open(&path) {
        Ok(file) => file,
        Err(err) => {
            write_stderr(&format!(
                "vestige-upgrade: failed to create {}: {err}",
                path.display()
            ));
            return Err(ExitCode::from(1));
        }
    };
    let deadline = Instant::now() + wait;
    let mut announced = false;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Held::Lock(file)),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(err)) => {
                write_stderr(&format!(
                    "vestige-upgrade: failed to lock {}: {err}",
                    path.display()
                ));
                return Err(ExitCode::from(1));
            }
        }
        if strata_log_ready(&data_dir.join(LOG_DIR_NAME)) {
            return Ok(Held::LogInstalled);
        }
        if Instant::now() >= deadline {
            write_stderr(&format!(
                "vestige-upgrade: another vestige process holds {} and did not release it within {}s. \
                 Set {LOCK_WAIT_ENV} to wait longer. The v3 data is untouched.",
                path.display(),
                wait.as_secs()
            ));
            return Err(ExitCode::from(1));
        }
        if !announced {
            write_stderr(&format!(
                "vestige-upgrade: {} is held by another vestige process; waiting for it",
                path.display()
            ));
            announced = true;
        }
        std::thread::sleep(LOCK_POLL);
    }
}
