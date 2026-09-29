# strata-kernel — Scope Handoff

Standalone determinism kernel, built for drop-in under `crates/`. This crate
keeps its own empty `[workspace]` table so it does NOT join the root
vestige workspace (parallel agents: do not remove that table until the
`crates/strata` integration lands). Committed on `build/strata-kernel`.

Dependencies (pinned in `Cargo.lock`, commit it): `borsh 1` (derive),
`blake3 1`, `libm 0.2`. No I/O anywhere in the library. Pure functions only.
`#![forbid(unsafe_code)]`.

## Exact type surface (code against these names)

```rust
// ---- canonical (src/canonical.rs) ------------------------------------
pub const Q_ONE: i64;                    // 1 << 32
pub const FRAC_BITS: u32;                // 32
pub fn to_q32_32(x: f64) -> i64;         // (libm::rint(x * 2^32)) as i64, saturating
pub fn from_q32_32(q: i64) -> f64;       // (q as f64) / 2^32, exact
pub trait HashableState: BorshSerialize + BorshDeserialize {}
pub trait AllowedStateField: Sized {}    // u8,u32,u64,i64,[u8;N],Vec,Option,BTreeMap (+linted structs, opted-in enums)
pub fn lint_roundtrip<T: HashableState + Clone>(sample: &T);  // debug_assert backstop
lint_state_type!(Name { field: ty, ... });                    // impls both traits above, compile-time field allowlist
strata_allowed_enum!(Name);                                  // opts an enum into AllowedStateField

// ---- event (src/event.rs) ---------------------------------------------
pub trait StrataEvent: borsh::BorshSerialize {
    fn seq(&self) -> u64;                // TIME. Never a wall clock.
}
#[derive(Borsh..., Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReviewEvent {
    pub card_id: u64,
    pub rating: u8,                      // 1..=4, clamped by the fold if outside
    pub event_seq: u64,
}

// ---- state (src/state.rs) ----------------------------------------------
#[derive(Borsh..., Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub applied_seq: u64,                       // last applied event seq (not in state_root)
    pub cards: BTreeMap<u64, CardState>,        // the ONLY map kind allowed
}

// ---- fsrs (src/fsrs.rs) -------------------------------------------------
pub const ALGO_V1: u32 = 1;
pub const ALGO_V2: u32 = 2;
pub const V1_WEIGHTS_MILLI: [u32; 21];
pub const V2_WEIGHTS_MILLI: [u32; 21];
pub fn weights_for(version: u32) -> Result<&'static [u32; 21], UnknownAlgoVersion>;
#[derive(Borsh..., Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardPhase { Learning, Review, Relearning }
#[derive(Borsh..., Clone, Debug, PartialEq, Eq)]
pub struct CardState {
    pub stability_q: i64,       // days, Q32.32, clamped [0.01, 36500]
    pub difficulty_q: i64,      // [1, 10], Q32.32
    pub last_seq: u64,
    pub review_count: u32,
    pub lapse_count: u32,
    pub phase: CardPhase,
}
pub struct FsrsFold;
impl FsrsFold {
    pub fn fold(state: &mut State, event: &ReviewEvent, weights: &[u32; 21]);
    pub fn retrievability(card: &CardState, current_seq: u64, version: u32)
        -> Result<f64, UnknownAlgoVersion>;     // DERIVED-ONLY, never stored
}

// ---- kernel (src/kernel.rs) ---------------------------------------------
pub type ApplyFn<E> = dyn Fn(&mut State, &E);           // the contract type, named
pub type KernelTableEntry<E> = (u32, fn(&mut State, &E));
pub struct UnknownAlgoVersion(pub u32);                 // Display + Error (std)
pub trait VersionedFold: StrataEvent + 'static {
    fn kernel_table() -> &'static [KernelTableEntry<Self>];
}
pub fn kernel_for<E: VersionedFold>(version: u32)
    -> Result<&'static ApplyFn<E>, UnknownAlgoVersion>; // dispatch by ALGORITHM_VERSION
pub struct Kernel<E: VersionedFold> {
    pub algo_version: u32,
    /* private apply fn */
}
impl<E: VersionedFold> Kernel<E> {
    pub fn for_version(version: u32) -> Result<Self, UnknownAlgoVersion>;
    pub fn apply(&self, state: &mut State, event: &E);
    pub fn apply_all<'a, I: IntoIterator<Item = &'a E>>(&self, state: &mut State, events: I);
}

// ---- checkpoint (src/checkpoint.rs) --------------------------------------
pub const MAGIC: [u8; 8] = *b"STRATAv1";
#[derive(Borsh..., Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    pub magic: [u8; 8],
    pub algo_version: u32,
    pub log_seq: u64,           // all events with seq <= log_seq are folded in
    pub prev_checkpoint: [u8; 32],  // hash of predecessor; [0;32] = genesis
    pub state_root: [u8; 32],
}
impl Checkpoint {
    pub fn genesis(algo_version: u32) -> Self;
    pub fn seal(algo_version: u32, log_seq: u64, prev: [u8; 32], state: &State) -> Self;
    pub fn hash(&self) -> [u8; 32];
}
pub fn state_root(state: &State) -> [u8; 32];
//  = blake3(concat over card_id ASCENDING of blake3(borsh((card_id, CardState))))
//  per-card subhash => incremental apply; empty state => blake3("")
pub fn checkpoint_hash(c: &Checkpoint) -> [u8; 32];  // = blake3(borsh(c))

// ---- verify (src/verify.rs) -----------------------------------------------
pub fn verify<E: VersionedFold>(
    checkpoints: &[Checkpoint],
    events: impl Iterator<Item = (u64, [u8; 32], E)>,   // (seq, blake3(borsh(event)), event)
) -> Result<(), VerifyError>;
pub fn verify_with_head<E: VersionedFold>(
    checkpoints: &[Checkpoint],
    head_hash: Option<[u8; 32]>,        // anchor for the HEAD checkpoint's hash
    events: impl Iterator<Item = (u64, [u8; 32], E)>,
) -> Result<(), VerifyError>;
pub enum VerifyError {
    BadMagic { index: usize, log_seq: u64 },
    UnknownAlgoVersion { version: u32, index: usize, log_seq: u64 },
    CheckpointOutOfOrder { index: usize, log_seq: u64, prev_log_seq: u64 },
    ChainBreak { index: usize, log_seq: u64 },
    CheckpointHashMismatch { index: usize, log_seq: u64 },
    EventSeqMismatch { seq: u64 },
    EventHashMismatch { seq: u64 },
    OutOfOrderEvent { seq: u64, applied_seq: u64 },
    StateRootMismatch { index: usize, log_seq: u64, expected: [u8; 32], actual: [u8; 32] },
    TrailingEvents { first_extra_seq: u64 },
}   // Display + Error (std); every variant names the offending seq/log_seq

// ---- prelude ---------------------------------------------------------------
pub mod prelude;  // re-exports all of the above
```

## Quantization spec (chosen: Q32.32 with round-ties-to-even)

`to_q32_32(x) = (libm::rint(x * 2^32)) as i64` where `rint` is IEEE-754
roundTiesToEven (2.5 -> 2, 3.5 -> 4, -2.5 -> -2, -3.5 -> -4) and the cast is
Rust saturating float-to-int (`NaN -> 0`, `+inf -> i64::MAX`, `-inf ->
i64::MIN`). `from_q32_32` divides by 2^32 — exact. `libm` (musl software
port) is the ONLY transcendental provider (exp/pow/rint); IEEE basic ops are
exactly specified and rustc does no FMA contraction by default.

## Verify semantics (read before wiring the log layer)

- Checkpoints' `log_seq` strictly increase; `prev_checkpoint` must equal the
  recomputed `checkpoint_hash` of the predecessor (genesis chains from
  `[0; 32]`). Any stored-byte tamper of a non-head checkpoint is caught by
  its successor's chain check OR by its own root check.
- The HEAD checkpoint has no successor: `verify()` still checks its magic,
  version, and state_root by replay, but the log layer SHOULD persist the
  head's `checkpoint_hash` externally and call `verify_with_head(...,
  Some(anchor), ...)` — then head tamper (e.g. raising `log_seq`) fails as
  `CheckpointHashMismatch`.
- Events are `(seq, event_hash, event)`; the kernel requires
  `blake3(borsh(event)) == event_hash` and `event.seq() == seq`; seqs must
  be strictly increasing; events past the last checkpoint are
  `TrailingEvents`.
- Version dispatch is PER SEGMENT: events with `prev.log_seq < seq <=
  log_seq` fold under THIS checkpoint's `algo_version`. That is the upgrade
  path — a mixed v1-then-v2 log verifies (tested).
- At each `log_seq`, recomputed `state_root` must equal the stored root
  bit-for-bit.

## FSRS weight layout (21 u32 milli-units; version-pinned, never edit in place)

| idx | role | v1 | v2 |
|-----|------|----|----|
| 0-3 | initial stability S0(G), ratings 1..=4 | 400/600/2400/5800 | …/…/…/6200 |
| 4-6 | D0: base / exp slope / offset | 7195/535/1460 | same |
| 7-8 | difficulty delta slope / mean-reversion | 520/200 | 545/210 |
| 9-11 | recall gain log / S exponent / R sensitivity | 500/140/940 | 540/165/965 |
| 12-13 | hard penalty / easy bonus | 620/1450 | same |
| 14-17 | forget: scale / D exp / S exp / R sens | 2180/50/340/1260 | same |
| 18 | short-term log-gain | 460 | same |
| 19-20 | retrievability FACTOR / decay offset | 235/0 | 242/0 |

v1 constants are FSRS-6-SHAPED approximations (documented in `src/fsrs.rs`):
`R = (1 + FACTOR*t/S)^decay`, `decay = -(0.5 + w20)`; recall update
`S*(1 + e^w9*(11-D)*S^-w10*(e^(w11*(1-R))-1)*hard*easy)`; forget
`w14*D^-w15*((S+1)^w16 - 1)*e^(w17*(1-R))` capped at S; difficulty with
mean reversion toward D0(4); elapsed==0 takes the short-term path
`S*e^(w18*(G-3))`. Changing ANY of this = new `ALGO_*` version.

## Integration notes for the parallel agents

- log agent: store borsh bytes per event; the record you hand `verify` is
  `(event.seq(), blake3(&borsh_event_bytes), event)`. Seal checkpoints with
  `Checkpoint::seal(version, seq, prev_hash, &state)` and persist
  `checkpoint_hash` of the head next to it (anchor).
- gate agent: `kernel_for::<ReviewEvent>(cp.algo_version)?` gives the exact
  fold the writer used; `state_root` is the only committed value.
- `State.applied_seq` is bookkeeping and is NOT covered by `state_root`.
- Tests: 19 unit + 22 integration, all green; `cargo clippy --all-targets
  -- -D warnings` clean; `cargo fmt` applied.
