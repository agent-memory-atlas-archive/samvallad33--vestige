# SCOPE-HANDOFF — `crates/strata` (append-only, fail-stop log)

Standalone crate; the empty `[workspace]` table in `crates/strata/Cargo.toml`
keeps it out of the repository root workspace (root `Cargo.toml` is
untouched). Build and test on its own:

```
cd crates/strata && cargo test          # 10 tests, all passing
cd crates/strata && cargo clippy --all-targets -- -D warnings   # clean
```

Deps: `borsh 1.8` (with the `derive` feature), `blake3 1.8`,
`ed25519-dalek 3.0`, plus `libc 0.2` under `cfg(unix)` (needed for
`fcntl(F_FULLFSYNC)` and `kill(pid, 0)`; unreachable from std).

## Public API (build against these names)

```rust
pub struct StrataLog { /* Clone; Arc-backed */ }

impl StrataLog {
    pub fn open(dir: impl AsRef<Path>) -> Result<StrataLog, StrataError>;
    pub fn append(&self, kind: u8, payload: &[u8]) -> Result<SeqAck, StrataError>;
    pub fn append_batch(&self, frames: Vec<(u8, Vec<u8>)>) -> Result<Vec<SeqAck>, StrataError>;
    pub fn read_frames(&self, from_seq: u64) -> Result<Vec<FrameRecord>, StrataError>;
    pub fn head(&self) -> HeadInfo;
    pub fn verify_tail(&self) -> Result<TailReport, StrataError>;
    pub fn seal(&self) -> Result<SealInfo, StrataError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeqAck {
    pub seq: u64,                 // first frame in the log is seq 1
    pub frame_hash: [u8; 32],     // blake3 of the frame's full wire encoding
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameRecord {
    pub seq: u64,
    pub kind: u8,
    pub payload: Vec<u8>,
    pub payload_blake3: [u8; 32],
    pub prev_frame_hash: [u8; 32],
    pub frame_hash: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadInfo {
    pub last_acked_seq: u64,      // acked watermark (0 = nothing acked)
    pub next_seq: u64,
    pub segment_no: u32,
    pub segment_id: [u8; 16],
    pub prev_segment_hash: [u8; 32],
    pub frames_in_segment: u64,
    pub frames_total: u64,
    pub last_frame_hash: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SealInfo {
    pub sealed_segment_no: u32,
    pub segment_id: [u8; 16],
    pub frame_count: u64,
    pub merkle_root: [u8; 32],
    pub segment_hash: [u8; 32],   // = next segment's prev_segment_hash
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrailerCheck { pub frame_count: u64, pub merkle_root: [u8; 32], pub signature_valid: bool }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TailReport {
    pub segment_no: u32,
    pub frames_verified: u64,
    pub first_seq: u64,
    pub last_seq: u64,
    pub last_frame_hash: [u8; 32],
    pub trailer: Option<TrailerCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HaltDetail { pub last_acked_seq: u64, pub segment: u32, pub offset: u64, pub reason: String }

#[derive(Debug)]
pub enum StrataError {
    Halt(HaltDetail),        // damage at/below the watermark; never truncated
    Locked { pid: u64 },     // single-writer lock held by a live pid
    Corrupt(String),         // unusable metadata (names, key, head.state)
    Io(std::io::Error),
}

// Wire structs (borsh, all fixed-width fields, no maps):
pub struct SegmentHeader { pub magic: [u8;8] /*STRTSEG1*/, pub version: u16, pub segment_id: [u8;16], pub prev_segment_hash: [u8;32] }
pub struct SegmentTrailer { pub frame_count: u64, pub merkle_root: [u8;32], pub signature: [u8;64] }
pub struct Frame { pub kind: u8, pub payload: Vec<u8>, pub payload_blake3: [u8;32], pub prev_frame_hash: [u8;32] }
pub const MAX_BATCH_FRAMES: usize = 64;      // group-commit batch cap
pub const GROUP_COMMIT_WINDOW_MS: u64 = 2;   // coalescing window
```

## Wire formats and hash definitions

Segment file `NNNNNNNN-<uuid-hex>.seg` (numbering dense from 0):

- Header (58 B): `magic[8] || version u16 || segment_id[16] || prev_segment_hash[32]`
- Frames: `len u32 || kind u8 || payload[len] || payload_blake3[32] || prev_frame_hash[32]`
  (exactly one length prefix; `Frame`'s borsh impl is manual for this reason)
- Trailer (104 B, written by `seal()`): `frame_count u64 || merkle_root[32] || signature[64]`

Hashes (all blake3):
- `payload_blake3 = blake3(kind || payload)` — the kind byte is bound in.
- `frame_hash = blake3(frame wire bytes)`; chain: first frame's `prev_frame_hash`
  = `header_hash`, later frames = previous `frame_hash`.
- Merkle (RFC6962-shaped over per-frame `payload_blake3`):
  `leaf = H(0x00 || payload_blake3)`, `node = H(0x01 || l || r)`, split at the
  largest power of two strictly below the node count; empty tree = `H(empty)`.
- Trailer signature: ed25519 over `segment_id || prev_segment_hash || merkle_root`.
  Key: per-directory `strata.key` (32-byte seed, 0600, from /dev/urandom).
- `segment_hash = blake3(sealed segment file, header..=trailer)`; the next
  segment's `prev_segment_hash` must equal it (checked at open).

Directory layout: `NNNNNNNN-<uuid>.seg`, `head.state` (borsh
`{last_acked_seq: u64}`, replaced atomically via temp+fsync+rename+dir-fsync),
`strata.lock` (O_EXCL, owner pid, stale takeover via `kill(pid,0)` = ESRCH),
`strata.key`.

## Semantics

- Commit pipeline per batch: WRITING -> SYNCING (Linux `fdatasync`, macOS
  `fcntl(F_FULLFSYNC)`; plain `fsync` only drains the kernel cache on APFS) ->
  VERIFYING (pread each frame, recompute blake3, byte-compare) -> DURABLE
  (atomic watermark) -> ACK. Any mismatch/io-error is a deliberate `panic!`
  with report `{last_acked_seq, op, errno}` — no retry, no fallback. Build
  integrations with `panic = "abort"`; the library panics (rather than
  aborts) so tests can `catch_unwind`.
- Group commit (v1, synchronous): first appender becomes batch leader and
  coalesces <=2ms / <=64 frames into one sync; later appenders queue behind
  the mutex + condvar. No flusher thread yet; a lone append pays the 2ms
  window. `append_batch` is the batched entry point; >64 frames chunk.
- seq is global, 1-based, monotonic. `last_acked_seq` counts only frames whose
  appends returned an ACK.
- Recovery on open: torn/short/zero tail above the watermark is truncated at
  the first bad frame; a fully-written valid frame above the watermark is
  retained (durable bytes, lost ack) and the watermark is NOT advanced for it.
  Damage at/below the watermark, a bad sealed-segment trailer/signature, a
  segment-chain mismatch, or missing acked frames => `Err(StrataError::Halt)`;
  history is never truncated. A sealed segment whose follow-on segment was
  never created is auto-rolled on open.
- `read_frames(from_seq)` returns durable frames (seq >= from_seq) in order.
- Single writer via `strata.lock`; probe/unlink is best-effort (TOCTOU
  documented); unparseable lock files are treated as held.

## Deviations from the mission spec (all documented in-code too)

1. `append`/`append_batch`/`seal` return `Result<_, StrataError>` instead of
   bare values; durability violations still panic (fail-stop), `Err` covers
   lock/metadata/recovery refusals.
2. `libc` added as a unix-only dependency: `F_FULLFSYNC` and `kill(pid, 0)`
   have no std path.
3. `payload_blake3` defined as `blake3(kind || payload)` (binds the kind
   byte); merkle leaves are `H(0x00 || payload_blake3)`.
4. Fail-stop is `panic!` (not a literal process abort) so tests can exercise
   it with `catch_unwind`.
5. Extra pub surface beyond the required five: `append_batch` (deterministic
   group-commit), `seal` (writes the signed trailer; the active segment has no
   trailer until sealed), `SealInfo`, `TailReport`, `TrailerCheck`, `Frame`.
6. Tests serialize through a process-wide mutex because the failpoint hooks
   (`SYNC_COUNT`, `FAIL_ON_SYNC_N`, `#[cfg(test)]`-gated in `src/sync.rs`)
   are statics shared by parallel libtest threads.

## Integration wiring (later, not done here)

Remove the empty `[workspace]` table in `crates/strata/Cargo.toml` and add
`"crates/strata"` to the root members. Kernel/gate crates then depend on
`strata` and use `SeqAck`/`FrameRecord`/`HeadInfo`/`StrataError` by name.
