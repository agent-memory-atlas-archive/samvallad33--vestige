//! Strata opener for protocol unit tests.
//!
//! `protocol/stdio.rs` calls `vestige_core::open_storage` and is not modified.
//! This module installs a hook, compiled only into the test harness when
//! `legacy-sqlite` is off, so those tests open a Strata log instead.

use std::path::PathBuf;

use ctor::ctor;

#[ctor]
fn install_strata_open_storage() {
    vestige_core::install_open_storage_hook(open_strata);
}

fn open_strata(
    path: Option<PathBuf>,
) -> Result<std::sync::Arc<vestige_core::Storage>, vestige_core::LegacySqliteDisabled> {
    let dir = path
        .as_deref()
        .and_then(|path| path.parent())
        .map(|parent| parent.to_path_buf())
        .unwrap_or_else(std::env::temp_dir);
    match crate::strata_memory::open(&dir) {
        Ok(storage) => Ok(storage),
        Err(err) => panic!("protocol test Strata open failed: {err}"),
    }
}
