//! Read-only check of a live strata-store directory: `log/*.seg`, and
//! `store.meta` when a checkpoint has been sealed. An unsealed store has
//! no anchor file; the segment chain is still checked. The root is not
//! opened as a log, so this never mints `strata.key` or a segment.

use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::readonly;

const META_NAME: &str = "store.meta";
const LOG_DIR: &str = "log";
const META_MAGIC: [u8; 8] = *b"STRSTME1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct LiveVerifyReport {
    pub ok: bool,
    pub frames_total: u64,
    pub segments: u32,
    pub failures: Vec<String>,
}

pub(crate) fn is_live_store(dir: &Path) -> bool {
    // A fresh store has segments and no anchor until the first seal.
    readonly::dir_has_segments(&dir.join(LOG_DIR))
}

/// `store.meta`, when a checkpoint has sealed one, must carry its magic.
pub(crate) fn store_meta_failures(dir: &Path) -> Vec<String> {
    let meta_path = dir.join(META_NAME);
    if !meta_path.is_file() {
        return Vec::new();
    }
    match fs::read(&meta_path) {
        Ok(bytes) if bytes.len() < META_MAGIC.len() || bytes[..META_MAGIC.len()] != META_MAGIC => {
            vec!["store.meta magic is not STRSTME1".into()]
        }
        Ok(_) => Vec::new(),
        Err(err) => vec![format!("read store.meta: {err}")],
    }
}
