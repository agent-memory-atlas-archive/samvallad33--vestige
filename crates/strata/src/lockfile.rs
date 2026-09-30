//! Single-writer directory lock: `strata.lock`, created with `O_EXCL`,
//! holding the owner pid as 8 little-endian bytes.
//!
//! Stale detection is best-effort: `kill(pid, 0)` — return 0 or EPERM means
//! the pid is alive, ESRCH means it is gone. The probe/unlink pair is racy
//! (TOCTOU): two processes can both observe a dead owner and both try to take
//! over; the second `O_EXCL` create loses, which bounds the damage to one
//! spurious `Locked` error. An unparseable or empty lock file is treated as
//! held (conservative against double-writers); remove it by hand if a writer
//! crashed between create and the pid write.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::StrataError;
use crate::sync::{self, SyncPurpose};

pub(crate) const LOCK_NAME: &str = "strata.lock";

pub(crate) struct DirLock {
    path: PathBuf,
}

impl DirLock {
    pub(crate) fn acquire(dir: &Path) -> Result<DirLock, StrataError> {
        let path = dir.join(LOCK_NAME);
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true) // O_EXCL
            .open(&path)
        {
            Ok(f) => write_pid_and_sync(f, path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let bytes = fs::read(&path)?;
                let pid = parse_pid(&bytes).ok_or(StrataError::Locked { pid: 0 })?;
                if pid_alive(pid) {
                    Err(StrataError::Locked { pid })
                } else {
                    // Best-effort stale takeover; see the module docs for the race.
                    let _ = fs::remove_file(&path);
                    match OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create_new(true)
                        .open(&path)
                    {
                        Ok(f) => write_pid_and_sync(f, path),
                        Err(e) => Err(e.into()),
                    }
                }
            }
            Err(e) => Err(e.into()),
        }
    }
}

fn write_pid_and_sync(mut f: std::fs::File, path: PathBuf) -> Result<DirLock, StrataError> {
    let pid = std::process::id() as u64;
    f.write_all(&pid.to_le_bytes())?;
    f.flush()?;
    sync::sync_file(&f, SyncPurpose::Metadata)?;
    Ok(DirLock { path })
}

fn parse_pid(bytes: &[u8]) -> Option<u64> {
    if bytes.len() != 8 {
        return None;
    }
    let mut b = [0u8; 8];
    b.copy_from_slice(bytes);
    Some(u64::from_le_bytes(b))
}

fn pid_alive(pid: u64) -> bool {
    if pid == 0 {
        return true; // conservative
    }
    #[cfg(unix)]
    {
        // Safety: kill(2) with signal 0 performs the permission/existence
        // check only; no signal is delivered.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if rc == 0 {
            return true;
        }
        let err = std::io::Error::last_os_error();
        !matches!(err.raw_os_error(), Some(libc::ESRCH))
    }
    #[cfg(windows)]
    {
        windows_pid_alive(pid)
    }
    #[cfg(not(any(unix, windows)))]
    {
        true // conservative on unsupported platforms
    }
}

/// `OpenProcess` + `GetExitCodeProcess`. Answering "alive" for every pid, as
/// this used to, left a killed process's lock in place for good: every later
/// open failed with `Locked` until the file was deleted by hand.
#[cfg(windows)]
fn windows_pid_alive(pid: u64) -> bool {
    type Handle = *mut core::ffi::c_void;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    const ERROR_ACCESS_DENIED: u32 = 5;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
        fn GetExitCodeProcess(process: Handle, exit_code: *mut u32) -> i32;
        fn CloseHandle(object: Handle) -> i32;
        fn GetLastError() -> u32;
    }

    // Windows process ids are 32-bit; a larger value names no process.
    let Ok(pid) = u32::try_from(pid) else {
        return false;
    };
    // Safety: plain Win32 calls on values we own; the handle is closed before
    // returning and `code` outlives the call that writes it.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            // Access denied: the process exists and belongs to someone else.
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut code = 0u32;
        let queried = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        // A failed query is treated as alive (conservative, like pid 0).
        queried == 0 || code == STILL_ACTIVE
    }
}

impl Drop for DirLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
