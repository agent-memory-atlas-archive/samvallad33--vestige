//! Which key a verify trusts.
//!
//! A receipt's embedded ed25519 key is trusted only when it equals the
//! public half of `receipt-signing.key` (the migrator writes that file
//! beside the log). Segment trailers are checked against `strata.key`
//! inside the log directory. The fingerprint is blake3 of the 32-byte
//! verifying key, hex.

use std::path::{Path, PathBuf};

use ed25519_dalek::{SigningKey, VerifyingKey};
use strata_migrate::records::RECEIPT_KEY_FILE;

/// blake3 hex of a 32-byte ed25519 verifying key.
pub(crate) fn fingerprint(public_key: &[u8; 32]) -> String {
    blake3::hash(public_key).to_hex().to_string()
}

pub(crate) struct KeyUse {
    pub fingerprint: String,
    /// `receipt-signing.key`, `strata.key`, or `none`.
    pub pin: &'static str,
    pub note: String,
}

/// No migration receipt. The key that was used is `strata.key`.
pub(crate) fn segment_only(segment_key: &[u8; 32]) -> KeyUse {
    KeyUse {
        fingerprint: fingerprint(segment_key),
        pin: "strata.key",
        note: "no migration receipt. Folder pin is strata.key in the log directory. A trailer signature must match it. No receipt-signing.key was required.".into(),
    }
}

/// Receipt present. Trust the embedded key only when the pin file matches.
pub(crate) fn require_receipt_pin(log_dir: &Path, embedded: &[u8; 32]) -> Result<KeyUse, String> {
    let Some(path) = find_receipt_pin(log_dir) else {
        return Err(
            "no receipt-signing.key pinned beside the log; refusing to trust the verifying key embedded in the receipt"
                .into(),
        );
    };
    let pinned = read_seed_key(&path)?;
    // The embedded key is not a pin. The file on disk is.
    if pinned.to_bytes() != *embedded {
        return Err(format!(
            "embedded receipt key does not match pinned {} ({})",
            RECEIPT_KEY_FILE,
            path.display()
        ));
    }
    Ok(KeyUse {
        fingerprint: fingerprint(&pinned.to_bytes()),
        pin: "receipt-signing.key",
        note: format!(
            "embedded receipt key matches pinned {} ({})",
            RECEIPT_KEY_FILE,
            path.display()
        ),
    })
}

/// Fingerprint of the pin file when it exists, even if the embedded key
/// does not match it.
pub(crate) fn pinned_fingerprint(log_dir: &Path) -> Option<String> {
    let path = find_receipt_pin(log_dir)?;
    let vk = read_seed_key(&path).ok()?;
    Some(fingerprint(&vk.to_bytes()))
}

/// `receipt-signing.key` inside the log directory, or beside it (where
/// migrate writes the file).
fn find_receipt_pin(log_dir: &Path) -> Option<PathBuf> {
    let inside = log_dir.join(RECEIPT_KEY_FILE);
    if inside.is_file() {
        return Some(inside);
    }
    let beside = log_dir.parent()?.join(RECEIPT_KEY_FILE);
    beside.is_file().then_some(beside)
}

fn read_seed_key(path: &Path) -> Result<VerifyingKey, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let seed: [u8; 32] = bytes
        .try_into()
        .map_err(|_| format!("{} is not 32 bytes", path.display()))?;
    Ok(SigningKey::from_bytes(&seed).verifying_key())
}
