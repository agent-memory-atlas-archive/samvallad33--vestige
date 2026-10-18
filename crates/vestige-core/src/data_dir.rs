//! The platform default data directory, used when neither `--data-dir` nor
//! `VESTIGE_DATA_DIR` names one.
//!
//! macOS and Windows keep the location every release has used. On Linux the
//! `directories` crate names the folder after the application alone, so
//! earlier releases stored data in `$XDG_DATA_HOME/core`, a generic name other
//! software can share (#433). New Linux installs use `$XDG_DATA_HOME/vestige`.
//! A store that already lives in the old folder keeps being used there, so an
//! upgrade never makes anyone's memory appear to vanish.

use std::path::{Path, PathBuf};

use directories::{BaseDirs, ProjectDirs};

use crate::actor::ACTOR_KEY_FILE;

/// The default every earlier release used on every platform.
pub fn legacy_default_data_dir() -> Option<PathBuf> {
    ProjectDirs::from("com", "vestige", "core").map(|dirs| dirs.data_dir().to_path_buf())
}

/// The default data directory for this platform.
pub fn default_data_dir() -> Option<PathBuf> {
    let legacy = legacy_default_data_dir()?;
    if cfg!(target_os = "linux") {
        let preferred = BaseDirs::new()?.data_dir().join("vestige");
        Some(choose_data_dir(preferred, legacy))
    } else {
        Some(legacy)
    }
}

/// `preferred` unless only `legacy` already holds a store.
fn choose_data_dir(preferred: PathBuf, legacy: PathBuf) -> PathBuf {
    if !holds_store(&preferred) && holds_store(&legacy) {
        legacy
    } else {
        preferred
    }
}

/// A Vestige store: a v3 database the upgrade still has to find, or a Strata
/// log beside one of the files only Vestige writes there.
fn holds_store(dir: &Path) -> bool {
    dir.join("vestige.db").is_file()
        || (dir.join("log").is_dir()
            && ["store.meta", ACTOR_KEY_FILE, "receipt-signing.key"]
                .iter()
                .any(|name| dir.join(name).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strata_store(dir: &Path) {
        std::fs::create_dir_all(dir.join("log")).unwrap();
        std::fs::write(dir.join("store.meta"), b"meta").unwrap();
    }

    #[test]
    fn a_fresh_machine_gets_the_vestige_folder() {
        let root = tempfile::tempdir().unwrap();
        let (preferred, legacy) = (root.path().join("vestige"), root.path().join("core"));
        assert_eq!(choose_data_dir(preferred.clone(), legacy), preferred);
    }

    #[test]
    fn an_existing_store_in_the_old_folder_stays_in_use() {
        let root = tempfile::tempdir().unwrap();
        let (preferred, legacy) = (root.path().join("vestige"), root.path().join("core"));
        strata_store(&legacy);
        assert_eq!(choose_data_dir(preferred, legacy.clone()), legacy);
    }

    #[test]
    fn a_v3_database_in_the_old_folder_stays_in_use() {
        let root = tempfile::tempdir().unwrap();
        let (preferred, legacy) = (root.path().join("vestige"), root.path().join("core"));
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("vestige.db"), b"v3").unwrap();
        assert_eq!(choose_data_dir(preferred, legacy.clone()), legacy);
    }

    #[test]
    fn the_new_folder_wins_once_it_holds_a_store() {
        let root = tempfile::tempdir().unwrap();
        let (preferred, legacy) = (root.path().join("vestige"), root.path().join("core"));
        strata_store(&legacy);
        strata_store(&preferred);
        assert_eq!(choose_data_dir(preferred.clone(), legacy), preferred);
    }

    #[test]
    fn another_apps_core_folder_is_not_a_store() {
        let root = tempfile::tempdir().unwrap();
        let (preferred, legacy) = (root.path().join("vestige"), root.path().join("core"));
        std::fs::create_dir_all(legacy.join("log")).unwrap();
        std::fs::write(legacy.join("settings.json"), b"{}").unwrap();
        assert_eq!(choose_data_dir(preferred.clone(), legacy), preferred);
    }

    #[test]
    fn other_platforms_keep_the_existing_default() {
        if !cfg!(target_os = "linux") {
            assert_eq!(default_data_dir(), legacy_default_data_dir());
        }
    }
}
