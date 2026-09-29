//! On-disk store layout for `strata-verify`: byte-exact framing, the writer,
//! and the decoders. This module IS the contract the sibling `strata-store`
//! crate must emit; the differential harness uses the writer to materialize
//! stores from the gate/kernel crates directly.
//!
//! # Layout
//!
//! ```text
//! <dir>/kernel.log          framed kernel records:
//!                            magic:"STVFv1\0\0" then per record
//!                            seq:u64le | event_hash:[u8;32] | len:u32le | payload
//! <dir>/kernel.checkpoints  borsh(Vec<Checkpoint>)
//! <dir>/kernel.head         32 bytes = checkpoint_hash(head checkpoint)
//! <dir>/gate.log            framed gate records:
//!                            magic:"STVFv1\0\0" then per record
//!                            seq:u64le | kind:u8 | len:u32le | payload
//! <dir>/gate.policy         borsh(Policy)
//! <dir>/gate.head           32 bytes = blake3(kind_u8 || seq_le_8 || payload)
//!                            of the LAST gate frame (the anchor recipe
//!                            mirrors strata_gate::log::MemLog::append)
//! ```
//!
//! The length prefix is 4 bytes little-endian ([`FRAME_LEN_BYTES`]). Payloads
//! are stored and re-hashed as raw bytes, so tampering flips a recomputed
//! hash instead of being re-serialized away.

use std::path::{Path, PathBuf};

use strata_gate::Policy;
use strata_gate::record::RecordKind;
use strata_kernel::checkpoint::Checkpoint;

/// Length-prefix width of every frame (u32 LE).
pub const FRAME_LEN_BYTES: usize = 4;
/// Magic bytes at the start of `kernel.log` and `gate.log`, binding the
/// framing format.
pub const MAGIC_BYTES: [u8; 8] = *b"STVFv1\0\0";
/// File names inside a store dir.
pub const KERNEL_LOG: &str = "kernel.log";
/// File names inside a store dir.
pub const KERNEL_CHECKPOINTS: &str = "kernel.checkpoints";
/// File names inside a store dir.
pub const KERNEL_HEAD: &str = "kernel.head";
/// File names inside a store dir.
pub const GATE_LOG: &str = "gate.log";
/// File names inside a store dir.
pub const GATE_POLICY: &str = "gate.policy";
/// File names inside a store dir.
pub const GATE_HEAD: &str = "gate.head";

/// Kernel frame header size: `seq:u64le | event_hash:32 | len:u32le`.
pub const KERNEL_HEADER_LEN: usize = 8 + 32 + FRAME_LEN_BYTES;
/// Gate frame header size: `seq:u64le | kind:u8 | len:u32le`.
pub const GATE_HEADER_LEN: usize = 8 + 1 + FRAME_LEN_BYTES;

/// Resolved paths of one store directory.
#[derive(Debug, Clone)]
pub struct StorePaths {
    /// Store root.
    pub dir: PathBuf,
    /// `<dir>/kernel.log`
    pub kernel_log: PathBuf,
    /// `<dir>/kernel.checkpoints`
    pub kernel_checkpoints: PathBuf,
    /// `<dir>/kernel.head`
    pub kernel_head: PathBuf,
    /// `<dir>/gate.log`
    pub gate_log: PathBuf,
    /// `<dir>/gate.policy`
    pub gate_policy: PathBuf,
    /// `<dir>/gate.head`
    pub gate_head: PathBuf,
}

/// Resolve the six artifact paths for the store at `dir`.
pub fn store_paths(dir: &Path) -> StorePaths {
    StorePaths {
        kernel_log: dir.join(KERNEL_LOG),
        kernel_checkpoints: dir.join(KERNEL_CHECKPOINTS),
        kernel_head: dir.join(KERNEL_HEAD),
        gate_log: dir.join(GATE_LOG),
        gate_policy: dir.join(GATE_POLICY),
        gate_head: dir.join(GATE_HEAD),
        dir: dir.to_path_buf(),
    }
}

/// One framed kernel-log record, decoded.
#[derive(Debug, Clone)]
pub struct KernelRecord {
    /// Frame seq (must equal `event.seq()`).
    pub seq: u64,
    /// Stored per-event hash: `blake3(payload)`.
    pub event_hash: [u8; 32],
    /// Raw borsh payload bytes, exactly as stored.
    pub payload: Vec<u8>,
}

/// One framed gate-log record, decoded.
#[derive(Debug, Clone)]
pub struct GateFrame {
    /// Frame seq (dense from 0 in append order).
    pub seq: u64,
    /// Record kind.
    pub kind: RecordKind,
    /// Raw payload bytes (borsh of the kind-specific record).
    pub payload: Vec<u8>,
}

/// Frame hash recipe, identical to `strata_gate::log::MemLog::append`:
/// `blake3(kind_u8 || seq_le_8 || payload)`.
pub fn gate_frame_hash(seq: u64, kind: RecordKind, payload: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&[kind.to_u8()]);
    hasher.update(&seq.to_le_bytes());
    hasher.update(payload);
    *hasher.finalize().as_bytes()
}

/// Everything needed to materialize one store directory. The differential
/// harness and the tamper tests build this; [`write_store`] serializes it.
#[derive(Debug, Clone)]
pub struct StoreFiles {
    /// Kernel records in seq order.
    pub kernel_records: Vec<KernelRecord>,
    /// Checkpoint chain (genesis first, head last).
    pub checkpoints: Vec<Checkpoint>,
    /// Anchor: `checkpoint_hash` of the head checkpoint.
    pub kernel_head: [u8; 32],
    /// Gate frames in seq order (dense from 0).
    pub gate_frames: Vec<GateFrame>,
    /// The pinned policy the gates were decided under.
    pub policy: Policy,
    /// Anchor: frame hash of the last gate frame.
    pub gate_head: [u8; 32],
}

/// Encode one kernel frame: `seq:u64le | hash:32 | len:u32le | payload`.
pub fn encode_kernel_frame(seq: u64, event_hash: &[u8; 32], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(KERNEL_HEADER_LEN + payload.len());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(event_hash);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// Encode one gate frame: `seq:u64le | kind:u8 | len:u32le | payload`.
pub fn encode_gate_frame(seq: u64, kind: RecordKind, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(GATE_HEADER_LEN + payload.len());
    out.extend_from_slice(&seq.to_le_bytes());
    out.push(kind.to_u8());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// One decoded frame span: `(body_start, body_end, seq)`.
type FrameSpan = (usize, usize, u64);

/// Decode failure: the offending frame's seq (when the header decoded) and
/// a description.
pub type FrameError = (Option<u64>, String);

/// Split a framed stream into [`FrameSpan`]s after the magic, walking to
/// EOF. `header_len` is per stream kind.
fn split_frames(bytes: &[u8], header_len: usize) -> Result<Vec<FrameSpan>, FrameError> {
    if bytes.len() < MAGIC_BYTES.len() || bytes[..MAGIC_BYTES.len()] != MAGIC_BYTES {
        return Err((None, "missing framing magic".to_string()));
    }
    let mut out = Vec::new();
    let mut cursor = MAGIC_BYTES.len();
    while cursor < bytes.len() {
        if cursor + header_len > bytes.len() {
            return Err((None, "truncated frame header".to_string()));
        }
        let seq = u64::from_le_bytes(
            bytes[cursor..cursor + 8]
                .try_into()
                .expect("8 bytes decode as u64"),
        );
        let len_at = cursor + header_len - FRAME_LEN_BYTES;
        let len = u32::from_le_bytes(
            bytes[len_at..len_at + FRAME_LEN_BYTES]
                .try_into()
                .expect("frame length prefix is 4 bytes"),
        ) as usize;
        let body_start = cursor + header_len;
        let body_end = body_start
            .checked_add(len)
            .ok_or((Some(seq), "frame length overflow".to_string()))?;
        if body_end > bytes.len() {
            return Err((Some(seq), "truncated frame body".to_string()));
        }
        out.push((body_start, body_end, seq));
        cursor = body_end;
    }
    Ok(out)
}

/// Decode a full `kernel.log` byte stream. On failure returns the offending
/// frame's seq (when the header decoded) and a description.
pub fn decode_kernel_records(bytes: &[u8]) -> Result<Vec<KernelRecord>, FrameError> {
    let frames = split_frames(bytes, KERNEL_HEADER_LEN)?;
    let mut out = Vec::with_capacity(frames.len());
    for (start, end, seq) in frames {
        // Header layout: [cursor..cursor+8) seq, [cursor+8..cursor+40) hash,
        // [cursor+40..cursor+44) len; body starts at cursor+44 = start.
        let hash: [u8; 32] = bytes[start - 36..start - 4]
            .try_into()
            .map_err(|_| (Some(seq), "hash slice misaligned".to_string()))?;
        out.push(KernelRecord {
            seq,
            event_hash: hash,
            payload: bytes[start..end].to_vec(),
        });
    }
    Ok(out)
}

/// Decode a full `gate.log` byte stream.
pub fn decode_gate_frames(bytes: &[u8]) -> Result<Vec<GateFrame>, FrameError> {
    let frames = split_frames(bytes, GATE_HEADER_LEN)?;
    let mut out = Vec::with_capacity(frames.len());
    for (start, end, seq) in frames {
        let kind_byte = bytes[start - FRAME_LEN_BYTES - 1];
        let Some(kind) = RecordKind::from_u8(kind_byte) else {
            return Err((Some(seq), format!("unknown record kind byte {kind_byte}")));
        };
        out.push(GateFrame {
            seq,
            kind,
            payload: bytes[start..end].to_vec(),
        });
    }
    Ok(out)
}

/// Write a complete store directory (creates `dir` if needed).
pub fn write_store(dir: &Path, files: &StoreFiles) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let paths = store_paths(dir);

    let mut kernel = Vec::new();
    kernel.extend_from_slice(&MAGIC_BYTES);
    for record in &files.kernel_records {
        kernel.extend_from_slice(&encode_kernel_frame(
            record.seq,
            &record.event_hash,
            &record.payload,
        ));
    }
    std::fs::write(&paths.kernel_log, &kernel)?;

    std::fs::write(
        &paths.kernel_checkpoints,
        borsh::to_vec(&files.checkpoints).expect("borsh Vec<Checkpoint> is infallible"),
    )?;
    std::fs::write(&paths.kernel_head, files.kernel_head)?;

    let mut gate = Vec::new();
    gate.extend_from_slice(&MAGIC_BYTES);
    for frame in &files.gate_frames {
        gate.extend_from_slice(&encode_gate_frame(frame.seq, frame.kind, &frame.payload));
    }
    std::fs::write(&paths.gate_log, &gate)?;
    std::fs::write(
        &paths.gate_policy,
        borsh::to_vec(&files.policy).expect("borsh Policy is infallible"),
    )?;
    std::fs::write(&paths.gate_head, files.gate_head)?;
    Ok(())
}

/// Read a file that must hold exactly 32 bytes.
pub fn read_bytes_exact_32(path: &Path) -> Result<[u8; 32], String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    bytes
        .try_into()
        .map_err(|b: Vec<u8>| format!("expected 32 bytes, found {}", b.len()))
}
