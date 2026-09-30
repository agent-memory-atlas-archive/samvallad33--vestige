//! Durability syncs.
//!
//! Linux uses `fdatasync` (`File::sync_data`). macOS uses
//! `fcntl(F_FULLFSYNC)` because on APFS a plain `fsync` only asks the kernel
//! to push its page cache toward the device — it does not make the drive
//! acknowledge the write to stable storage, so a power loss can still lose
//! the data. `F_FULLFSYNC` forces that acknowledgement and is what a
//! fail-stop WAL needs.

use std::fs::File;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncPurpose {
    /// Segment data sync — the only kind counted by the test failpoint.
    Segment,
    /// Watermark / lock / metadata sync.
    Metadata,
}

// Failpoint hooks (test-only): SYNC_COUNT counts segment syncs since the last
// reset; setting FAIL_ON_SYNC_N to n makes the n-th segment sync return an
// injected error so tests can drive the deliberate-panic path.
#[cfg(test)]
pub(crate) static SYNC_COUNT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
pub(crate) static FAIL_ON_SYNC_N: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
pub(crate) fn reset_failpoints() {
    use std::sync::atomic::Ordering;
    SYNC_COUNT.store(0, Ordering::SeqCst);
    FAIL_ON_SYNC_N.store(0, Ordering::SeqCst);
}

pub(crate) fn sync_file(file: &File, purpose: SyncPurpose) -> io::Result<()> {
    #[cfg(test)]
    if purpose == SyncPurpose::Segment {
        use std::sync::atomic::Ordering;
        let n = SYNC_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
        let fail_at = FAIL_ON_SYNC_N.load(Ordering::SeqCst);
        if fail_at != 0 && n == fail_at {
            return Err(io::Error::other("strata failpoint: injected sync failure"));
        }
    }
    let _ = purpose; // only read by the test hook above
    platform_sync(file)
}

fn platform_sync(file: &File) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        file.sync_data() // fdatasync
    }
    #[cfg(target_os = "macos")]
    {
        fcntl_fullfsync(file)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        file.sync_all() // best-effort fallback off the supported platforms
    }
}

/// Durability barrier for directory entries (segment creation, watermark rename).
///
/// Unix: fsync the directory. Windows has no equivalent: `File::open` on a
/// directory fails with ERROR_ACCESS_DENIED (os error 5), which made every
/// `StrataLog::open` fail there. NTFS records the entry change in its
/// metadata journal; there is no per-directory barrier to call, so on
/// Windows this is a no-op and a new entry is as durable as NTFS makes it.
pub(crate) fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn fcntl_fullfsync(file: &File) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    // Safety: fcntl(2) on an fd we own; F_FULLFSYNC takes no argument and
    // transfers no ownership.
    let rc = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) };
    if rc == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
