//! Receipt signing-seed sidecar loader. Filesystem only; no SQLite.

use std::path::Path;

use super::{Result, StorageError};

#[cfg(unix)]
fn validate_sidecar_directory(directory: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::symlink_metadata(directory).map_err(|error| {
        StorageError::Init(format!("stat receipt signing-key directory: {error}"))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StorageError::Init(
            "receipt signing-key directory must be a non-symlink directory".into(),
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(StorageError::Init(
            "receipt signing-key directory permissions must not grant group/other access".into(),
        ));
    }
    Ok(())
}

/// Load a provisioned 32-byte seed after revalidating type, size, symlink, and
/// Unix permission boundaries. Callers should minimize its lifetime.
#[cfg(unix)]
pub fn load_receipt_signing_seed(path: &Path) -> Result<[u8; 32]> {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| StorageError::Init(format!("stat signing-key sidecar: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(StorageError::Init(
            "receipt signing-key sidecar must be a regular non-symlink file".into(),
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(StorageError::Init(
            "receipt signing-key sidecar permissions must not grant group/other access".into(),
        ));
    }
    if let Some(directory) = path.parent() {
        validate_sidecar_directory(directory)?;
    }
    let mut seed = [0_u8; 32];
    let mut file = std::fs::File::open(path)
        .map_err(|error| StorageError::Init(format!("open signing-key sidecar: {error}")))?;
    file.read_exact(&mut seed)
        .map_err(|error| StorageError::Init(format!("read signing-key seed: {error}")))?;
    let mut trailing = [0_u8; 1];
    if file
        .read(&mut trailing)
        .map_err(|error| StorageError::Init(format!("read signing-key trailer: {error}")))?
        != 0
    {
        return Err(StorageError::Init(
            "receipt signing-key sidecar must contain exactly 32 bytes".into(),
        ));
    }
    Ok(seed)
}

#[cfg(not(unix))]
pub fn load_receipt_signing_seed(_path: &Path) -> Result<[u8; 32]> {
    Err(StorageError::Init(
        "secure receipt signing-key sidecar loading currently requires Unix permission semantics"
            .into(),
    ))
}
