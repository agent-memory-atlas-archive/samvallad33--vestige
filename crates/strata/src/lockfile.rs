//! Single-writer directory lock on `strata.lock`.
//!
//! The lock is an OS advisory lock (`File::try_lock`) held on an open
//! descriptor for the life of the log. The kernel releases it when its holder
//! exits, SIGKILL and power loss included, so a leftover file never names a
//! holder: whether the log is held is answered by the lock itself, never by
//! what the file contains. That removes every question a pid file leaves open
//! (a process killed between creating the file and writing the pid, a pid
//! recycled by an unrelated process, a crashed owner).
//!
//! The file also carries the holder's pid as 8 little-endian bytes, written
//! after the lock is taken. It is informational only, used to name the holder
//! in [`StrataError::Locked`]; it is never trusted to decide ownership.
//!
//! The file is never unlinked. Removing it while it is held would let a second
//! process create a fresh file and lock that one, and two writers would then
//! both believe they own the log. Releasing the lock is closing the file.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::error::StrataError;
use crate::sync::{self, SyncPurpose};

pub(crate) const LOCK_NAME: &str = "strata.lock";

pub(crate) struct DirLock {
    /// Holding the open file is holding the lock.
    _file: File,
}

impl DirLock {
    pub(crate) fn acquire(dir: &Path) -> Result<DirLock, StrataError> {
        let path = dir.join(LOCK_NAME);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        match file.try_lock() {
            Ok(()) => {
                write_pid_and_sync(&mut file)?;
                Ok(DirLock { _file: file })
            }
            Err(TryLockError::WouldBlock) => Err(StrataError::Locked {
                pid: read_pid(&mut file).unwrap_or(0),
            }),
            Err(TryLockError::Error(e)) => Err(e.into()),
        }
    }
}

fn write_pid_and_sync(f: &mut File) -> Result<(), StrataError> {
    let pid = u64::from(std::process::id());
    f.set_len(0)?;
    f.seek(SeekFrom::Start(0))?;
    f.write_all(&pid.to_le_bytes())?;
    f.flush()?;
    sync::sync_file(f, SyncPurpose::Metadata)?;
    Ok(())
}

/// The pid the holder recorded. `None` when it has not written it yet, or
/// when the platform will not let a second handle read a locked file.
fn read_pid(f: &mut File) -> Option<u64> {
    let mut bytes = Vec::with_capacity(8);
    f.seek(SeekFrom::Start(0)).ok()?;
    f.take(9).read_to_end(&mut bytes).ok()?;
    parse_pid(&bytes)
}

fn parse_pid(bytes: &[u8]) -> Option<u64> {
    let b: [u8; 8] = bytes.try_into().ok()?;
    Some(u64::from_le_bytes(b))
}
