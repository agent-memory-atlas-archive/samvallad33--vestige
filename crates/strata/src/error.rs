//! Error surface. [`StrataError::Halt`] is the fail-stop boundary: it means
//! durable history is damaged and the log must not be written or truncated
//! any further.

use std::fmt;
use std::io;

/// Details of a [`StrataError::Halt`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HaltDetail {
    /// Highest sequence number acknowledged durable before the damage was found.
    pub last_acked_seq: u64,
    /// Segment number the damage was found in.
    pub segment: u32,
    /// Byte offset of the first bad frame (or of the header, if that failed).
    pub offset: u64,
    /// Human-readable description of the check that failed.
    pub reason: String,
}

#[derive(Debug)]
pub enum StrataError {
    /// Corruption at or below the acked watermark. Never truncated.
    Halt(HaltDetail),
    /// The directory lock is held by a live writer.
    Locked {
        pid: u64,
    },
    /// On-disk metadata (key file, head.state, segment names) is unusable.
    Corrupt(String),
    Io(io::Error),
}

impl StrataError {
    /// True when the write was refused because the volume (or the caller's
    /// quota on it) is full. Nothing of the refused write was kept, the log
    /// is unchanged, and the same call can succeed once space is freed.
    pub fn is_storage_full(&self) -> bool {
        matches!(self, StrataError::Io(e) if crate::log::is_storage_full(e))
    }
}

impl fmt::Display for StrataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StrataError::Halt(d) => write!(
                f,
                "strata halt: {} (segment {}, offset {}, last_acked_seq {}) \
                 — damage at/below the acked watermark, refusing to truncate history",
                d.reason, d.segment, d.offset, d.last_acked_seq
            ),
            StrataError::Locked { pid } => {
                write!(f, "strata directory locked by pid {pid}")
            }
            StrataError::Corrupt(m) => write!(f, "strata metadata corrupt: {m}"),
            StrataError::Io(e) => write!(f, "strata io error: {e}"),
        }
    }
}

impl std::error::Error for StrataError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StrataError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for StrataError {
    fn from(e: io::Error) -> Self {
        StrataError::Io(e)
    }
}
