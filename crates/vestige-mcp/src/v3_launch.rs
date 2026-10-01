//! 4.0 notices a v3 store only when `vestige.db` exists. It never opens that
//! file. The import runs in `vestige-upgrade`.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

/// Last v3 release operators can keep running when 4.0 cannot upgrade.
pub const V311_RELEASE: &str = "https://github.com/samvallad33/vestige/releases/tag/v3.1.1";

/// Why the launch path stopped before opening a Strata log.
#[derive(Debug)]
pub enum LaunchError {
    /// `vestige.db` is present and `vestige-upgrade` is not installed.
    MissingTool { message: String },
    /// `vestige-upgrade` ran and exited non-zero. Its own stderr has the detail.
    UpgradeFailed { status: ExitStatus },
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::MissingTool { message } => f.write_str(message),
            LaunchError::UpgradeFailed { status } => {
                write!(f, "vestige-upgrade exited {status}")
            }
        }
    }
}

impl std::error::Error for LaunchError {}

impl LaunchError {
    pub fn code(&self) -> i32 {
        match self {
            LaunchError::MissingTool { .. } => 1,
            LaunchError::UpgradeFailed { status } => status.code().unwrap_or(1),
        }
    }
}

/// True when `path` is an existing file. Uses `metadata` (stat), never `open`.
pub fn db_present(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file())
}

/// Run `vestige-upgrade` when `vestige.db` exists. Leave the file untouched
/// when the tool is missing.
pub fn upgrade_or_refuse(db_path: &Path) -> Result<(), LaunchError> {
    if !db_present(db_path) {
        return Ok(());
    }
    // The v3 file is kept after a successful upgrade, so it is present on
    // every later launch. Once the Strata log is published there is nothing
    // to upgrade: start without spawning the helper, which a later install
    // may not even ship.
    if strata_log_published(db_path) {
        return Ok(());
    }
    let Some(bin) = find_upgrade() else {
        return Err(LaunchError::MissingTool {
            message: refusal(db_path),
        });
    };
    let data_dir = db_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let status = Command::new(&bin)
        .arg("--data-dir")
        .arg(data_dir)
        .status()
        .map_err(|err| LaunchError::MissingTool {
            message: format!(
                "failed to run {}: {err}\n{}",
                bin.display(),
                refusal(db_path)
            ),
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(LaunchError::UpgradeFailed { status })
    }
}

/// True once `<data-dir>/log` holds a segment or its `strata.key`. The
/// upgrade publishes by renaming a verified staging directory onto `log/`,
/// so a present log means the upgrade completed. Stat and directory reads
/// only; the v3 file is never opened.
pub fn strata_log_published(db_path: &Path) -> bool {
    let data_dir = db_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::read_dir(data_dir.join("log")).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            let name = entry.file_name();
            name == "strata.key" || Path::new(&name).extension().is_some_and(|ext| ext == "seg")
        })
    })
}

fn refusal(db_path: &Path) -> String {
    format!(
        "v3 store detected at {} because vestige.db exists.\n\
         vestige-upgrade was not found next to this binary or on PATH.\n\
         Install vestige-upgrade from the release archive (it ships beside vestige), \
         or keep using v3.1.1: {V311_RELEASE}\n\
         The v3 data is untouched.",
        db_path.display()
    )
}

fn upgrade_file_name() -> &'static str {
    if cfg!(windows) {
        "vestige-upgrade.exe"
    } else {
        "vestige-upgrade"
    }
}

/// Sibling of this executable, then each `PATH` directory.
fn find_upgrade() -> Option<PathBuf> {
    find_upgrade_from(
        std::env::current_exe().ok().as_deref(),
        std::env::var_os("PATH"),
    )
}

fn find_upgrade_from(exe: Option<&Path>, path: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let name = upgrade_file_name();
    if let Some(exe) = exe {
        // The real binary's directory first: the executable path can be the
        // symlink it was launched through, which has no helper beside it.
        let resolved = exe.canonicalize().ok();
        for dir in resolved
            .as_deref()
            .and_then(Path::parent)
            .into_iter()
            .chain(exe.parent())
        {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let path = path?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// After a completed upgrade the kept v3 file must not send every launch
    /// through the helper: a published log starts without it.
    #[test]
    fn published_log_starts_without_the_helper() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("vestige.db");
        std::fs::write(&db, b"SQLite format 3\0kept v3 store").unwrap();
        std::fs::create_dir_all(dir.path().join("log")).unwrap();
        std::fs::write(dir.path().join("log").join("00000000-ab.seg"), b"segment").unwrap();

        assert!(upgrade_or_refuse(&db).is_ok());
        // The helper logs every run it makes; none happened.
        assert!(!dir.path().join("upgrade.log").exists());
        assert_eq!(
            std::fs::read(&db).unwrap(),
            b"SQLite format 3\0kept v3 store"
        );
    }

    #[test]
    fn a_log_dir_without_segments_is_not_published() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("vestige.db");
        std::fs::write(&db, b"SQLite format 3\0").unwrap();
        std::fs::create_dir_all(dir.path().join("log")).unwrap();
        assert!(!strata_log_published(&db));
    }

    /// A launcher symlink to the real binary must still find the helper that
    /// ships beside the real binary (where `current_exe` reports the link).
    #[cfg(unix)]
    #[test]
    fn a_symlinked_launcher_finds_the_helper_beside_the_real_binary() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        let links = dir.path().join("links");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(&links).unwrap();
        std::fs::write(real.join("vestige-mcp"), b"").unwrap();
        std::fs::write(real.join(upgrade_file_name()), b"").unwrap();
        let link = links.join("vestige-mcp");
        std::os::unix::fs::symlink(real.join("vestige-mcp"), &link).unwrap();

        let found = find_upgrade_from(Some(&link), None).expect("helper beside the real binary");
        assert_eq!(
            found.canonicalize().unwrap(),
            real.join(upgrade_file_name()).canonicalize().unwrap()
        );
    }

    /// A helper next to the invoked path still wins over `PATH`.
    #[test]
    fn a_helper_beside_the_invoked_path_is_found() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("vestige-mcp"), b"").unwrap();
        std::fs::write(dir.path().join(upgrade_file_name()), b"").unwrap();
        let found = find_upgrade_from(Some(&dir.path().join("vestige-mcp")), None).unwrap();
        assert_eq!(found, dir.path().join(upgrade_file_name()));
    }
}
