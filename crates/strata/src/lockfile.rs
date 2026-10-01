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
use std::time::{Duration, Instant};

use crate::error::StrataError;
use crate::sync::{self, Site, SyncPurpose};

pub(crate) const LOCK_NAME: &str = "strata.lock";

/// How long a held lock is retried before the log is reported as locked.
///
/// A lock can look held for an instant after its holder released it: a thread
/// that spawns a child process shares every open descriptor with the child
/// from fork until exec, and the kernel keeps the lock until the last copy
/// closes. A second opener that arrives in that window is not facing a second
/// writer. Waiting a few polls tells the two apart; a real holder keeps the
/// lock, so the refusal only arrives this much later.
const CONTENTION_BUDGET: Duration = Duration::from_millis(500);
const CONTENTION_POLL: Duration = Duration::from_millis(5);

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
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => {
                    write_pid_and_sync(&mut file)?;
                    return Ok(DirLock { _file: file });
                }
                Err(TryLockError::WouldBlock) if started.elapsed() < CONTENTION_BUDGET => {
                    std::thread::sleep(CONTENTION_POLL);
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(StrataError::Locked {
                        pid: read_pid(&mut file).unwrap_or(0),
                    });
                }
                Err(TryLockError::Error(e)) => return Err(e.into()),
            }
        }
    }
}

fn write_pid_and_sync(f: &mut File) -> Result<(), StrataError> {
    // A full volume refuses the lock write; dropping the handle releases
    // the OS lock, so nothing is left that blocks the next open.
    sync::guard_space(Site::Lock)?;
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

#[cfg(test)]
mod contention_tests {
    use super::*;
    #[cfg(unix)]
    use std::sync::atomic::{AtomicBool, Ordering};
    #[cfg(unix)]
    use std::sync::Arc;

    /// The refusal names this process as the holder. On Windows a second
    /// handle cannot read a locked file, so the pid may be unknown (0).
    fn assert_holder(pid: u64) {
        let me = u64::from(std::process::id());
        if cfg!(windows) {
            assert!(pid == me || pid == 0, "unexpected holder pid {pid}");
        } else {
            assert_eq!(pid, me);
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("strata-lock-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn a_lock_released_a_moment_after_the_attempt_started_is_taken() {
        let dir = scratch("late-release");
        let holder = DirLock::acquire(&dir).expect("holder");
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            drop(holder);
        });
        let taken = DirLock::acquire(&dir);
        release.join().expect("release thread");
        assert!(
            taken.is_ok(),
            "a holder that lets go within the budget must not refuse the opener"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_lock_that_stays_held_is_still_refused_with_its_holder() {
        let dir = scratch("still-held");
        let _holder = DirLock::acquire(&dir).expect("holder");
        let started = Instant::now();
        match DirLock::acquire(&dir) {
            Err(StrataError::Locked { pid }) => assert_holder(pid),
            other => panic!("a held lock must refuse, got {:?}", other.err()),
        }
        assert!(
            started.elapsed() >= CONTENTION_BUDGET,
            "the refusal waits out the budget"
        );
        assert!(
            started.elapsed() < CONTENTION_BUDGET * 8,
            "and not much longer"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn reacquiring_survives_children_spawned_by_sibling_threads() {
        let dir = scratch("sibling-spawns");
        let stop = Arc::new(AtomicBool::new(false));
        let spawners: Vec<_> = (0..4)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        let _ = std::process::Command::new("true").status();
                    }
                })
            })
            .collect();
        for round in 0..300 {
            let first = DirLock::acquire(&dir).unwrap_or_else(|e| panic!("round {round}: {e:?}"));
            drop(first);
            let second = DirLock::acquire(&dir).unwrap_or_else(|e| panic!("round {round}: {e:?}"));
            drop(second);
        }
        stop.store(true, Ordering::Relaxed);
        for t in spawners {
            t.join().expect("spawner");
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
