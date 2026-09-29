# strata-gate — Scope Handoff (byte-exact)

The gate runtime. The model proposes, the deterministic checker decides,
change+verdict land as one signed record; a write with no approving verdict
before it in the log is REJECTED at admission. Pure functions over event
streams — no clocks, no floats, no env, no RNG.

Standalone crate by design: own `Cargo.toml` with an empty `[workspace]`
table, NOT a member of the vestige root workspace. Sibling crates `strata`
and `strata-kernel` (and the root `Cargo.toml`) were not touched. When Sam's
integration lands, re-parent deliberately.

## Public API (crate `strata-gate`)

```rust
// Integration shims (local; the strata kernel wires its own equivalents).
pub struct SeqAck { pub seq: u64, pub frame_hash: [u8; 32] }
pub trait EventLog {
    fn events_before(&self, bound: u64) -> Vec<GateEvent>;      // seq < bound
    fn append(&mut self, kind: RecordKind, payload: Vec<u8>) -> SeqAck;
    fn tip(&self) -> u64 { ... }                                // provided
}
pub struct MemLog;                       // in-memory impl + wiring example

// Records (all borsh; see layouts below)
pub enum RecordKind { Propose, Gate, Effect, Gap, LessonAlarm, Canary, Alert } // codes 1..=7
pub struct Propose; pub struct GateRecord; pub struct EffectRecord;
pub struct LessonAlarmRecord; pub struct CanaryRecord; pub struct AlertRecord;
pub struct GapRecord { pub duty: DutyKind, pub detail: GapDetail }
pub enum Verdict { Allow, Deny, Hold }   // wire 0/1/2

// Inputs
pub const FORGET_FLOOR_MILLI: i64 = 2_592_000_000;   // 30 days
pub struct GateInputs; pub struct BlastRadius { pub closure_size: u32, pub tiers: u16 }
pub fn compute_inputs(log: &dyn EventLog, propose_seq: u64) -> Result<GateInputs, GateError>;

// Policy VM
pub struct Policy { pub rules: Vec<Rule> }           // first match wins; default Deny
pub struct Rule { match_kind, match_params_hash_prefix, max_blast_radius,
                  forbid_forgotten_lessons, require_human, verdict }
pub const WILDCARD_PREFIX: [u8; 8] = [0; 8];
pub const ANY_KIND: u8 = 255;
pub fn policy_hash(&Policy) -> [u8; 32];
pub fn evaluate(&Policy, &GateInputs) -> Verdict;                    // wildcard subject
pub fn evaluate_detailed(&Policy, &Propose, &GateInputs) -> (Verdict, Option<Veto>);
pub fn gate_verdict(&Policy, &Propose, &GateInputs) -> Verdict;      // + canary clamp

// Admission (the core invariant)
pub fn admit(log: &dyn EventLog, effect: &EffectRecord, pinned: &Policy)
    -> Result<SeqAck /* admission ticket */, Rejected>;
pub enum Rejected { NoProposal, NoGate, GateDenied, GateStale([u8;32]),
                    InputsDrift, ForbiddenByAlarm }                    // codes 1..=6

// Runtime — the ONLY append path for effects is commit_effect
pub struct GateRuntime<L: EventLog>;
impl GateRuntime<L> {
    new(log, pinned_policy) / pin_policy / pinned
    commit_propose(Propose) -> SeqAck          // auto-appends ALERT on canary trip
    commit_gate(propose_seq) -> Result<SeqAck, GateError>
    commit_effect(EffectRecord) -> Result<SeqAck, Rejected>   // admit() first, always
    commit_lesson_alarm / commit_canary / commit_gap
    admit(&EffectRecord) / latest_gate(propose_seq)
    rederive_verdicts() -> Result<Vec<(u64, Verdict)>, RederiveError>
    sweep() -> Vec<GapRecord>
}

// Replay + structural sweep
pub fn rederive_verdicts(log, pinned) -> Result<Vec<(u64, Verdict)>, RederiveError>;
pub fn sweep(log) -> Vec<GapRecord>;
```

## Kind codes (frame `kind` byte)

| code | kind          | payload struct        | exact borsh size            |
|-----:|---------------|-----------------------|-----------------------------|
| 1    | PROPOSE       | `Propose`             | 69 + 8n (n = context len)   |
| 2    | GATE          | `GateRecord`          | 119 + 10k (k = forgotten)   |
| 3    | EFFECT        | `EffectRecord`        | 80                          |
| 4    | GAP           | `GapRecord`           | 2 + 16 / 16 / 24 per variant|
| 5    | LESSON_ALARM  | `LessonAlarmRecord`   | 24                          |
| 6    | CANARY        | `CanaryRecord`        | 8                           |
| 7    | ALERT         | `AlertRecord`         | 16                          |

0 is reserved ("unknown"). All integers little-endian; `Vec` = `u32` length
prefix then elements; `bool` = 1 byte (0/1); enums = `u8` variant ordinal.

## Field layouts (offsets from payload start)

- **PROPOSE** `action_hash [u8;32] | action_kind u8 | params_hash [u8;32] | context Vec<u64>`
  `action_kind`: WRITE=0, RETIRE=1, GRANT=2, EFFECT=3.
- **GATE** `propose_seq u64 | verdict u8 (Allow=0, Deny=1, Hold=2) | policy_hash [u8;32] | inputs GateInputs`
  - `GateInputs` (78 + 10k): `live_facts_digest [u8;32] | retired_facts_digest [u8;32] |
    closure_size u32 | tiers u16 | forgotten_lessons Vec<(u64 lesson_id, i64 retention_milli)> |
    canary_hits u32`.
- **EFFECT** `propose_seq u64 | gate_seq u64 | action_hash [u8;32] | payload_digest [u8;32]`.
- **LESSON_ALARM** `propose_seq u64 | lesson_id u64 | retention_milli i64`.
- **CANARY** `canary_id u64`. **ALERT** `canary_id u64 | reader_seq u64`.
- **GAP** `duty u8 | detail-ordinal u8 | variant fields`
  - `duty`: OrphanEffect=0, ReadNoReceipt=1, DutySeqGap=2.
  - `GapDetail::OrphanEffect { effect_seq u64, propose_seq u64, reason u8 }` (reason = `Rejected` code 1..=6);
    `ReadNoReceipt { reader_seq u64, dangling_id u64 }`;
    `DutySeqGap { source u64, expected u64, found u64 }` (source 0 = the single append path).
- **Rule** (16): `match_kind u8 (or 255=ANY_KIND) | match_params_hash_prefix [u8;8] (or [0;8]=wildcard) |
  max_blast_radius u32 | forbid_forgotten_lessons bool | require_human bool | verdict u8`.
- **Policy**: `u32 rule-count | rules...` (16 bytes each).

## Hash recipes

- `live/retired_facts_digest` = `blake3(borsh(Vec<u64>))` over the sorted id set — exact, no similarity.
- `policy_hash` = `blake3(borsh(Policy))`.
- `MemLog` frame hash = `blake3(kind_u8 || seq_le_8 || payload)`.
- `admit()` ticket: `seq` = next free seq, `frame_hash` = `blake3(borsh(EffectRecord))`.

## Derived-state model (all pure over the prefix `seq < propose_seq`)

- Fact id = the seq of the WRITE effect that created it. RETIRE effects move
  the ids in their proposal's `context` from live to retired.
- Blast radius: closure over the prefix proposals' context hyperedges (each
  prior PROPOSE context is one hyperedge; intersecting edges merge). `tiers`
  = fixpoint expansion rounds. Empty context = (0, 0).
- `forgotten_lessons`: LESSON_ALARMs in the prefix with
  `retention_milli < FORGET_FLOOR_MILLI` (latest alarm per lesson wins).
- `canary_hits`: count of ALERT records in the prefix.

## Admission state machine (`admit`, enforced in `GateRuntime::commit_effect`)

1. PROPOSE at `effect.propose_seq` with matching `action_hash` — else `NoProposal`.
2. Latest GATE for the proposal with `propose_seq < gate_seq < effect_seq`, and
   `effect.gate_seq` must cite exactly that gate — else `NoGate`.
3. Cited gate verdict == Allow — else `GateDenied` (Deny and Hold both reject).
4. `gate.policy_hash == blake3(borsh(pinned))` — else `GateStale(stored_hash)`.
5. `gate.inputs == compute_inputs(log, propose_seq)` recomputed now — else `InputsDrift`.
6. Re-evaluate under the pinned policy (with the canary clamp): non-Allow —
   else `ForbiddenByAlarm` when the winning rule forbids forgotten lessons and
   (recomputed inputs carry one, OR a `LESSON_ALARM{propose_seq == effect.propose_seq,
   retention < floor}` exists inside the window `(propose_seq, effect_seq)`),
   `GateDenied` otherwise.
7. Pass ⇒ admission ticket; the runtime then appends the EFFECT (single path).

Canaries: `commit_canary` plants the id; `commit_propose` scans the planted
ids against the proposal's `context` and appends `ALERT {canary_id,
reader_seq = propose_seq}` per trip; later gates clamp Allow→Hold via
`inputs.canary_hits` (implemented once in `gate_verdict`, used identically by
`commit_gate` and `rederive_verdicts`, so re-derivation is bit-for-bit).

## Integration decisions (deviations the kernel must know)

1. `admit(log, effect, pinned)` and `rederive_verdicts(log, pinned)` take the
   pinned policy explicitly: policy content is not embedded in the log (GATE
   stores `policy_hash` only). Runtime-bound forms hold the pin and match the
   mission signatures with `log = self.log`.
2. `evaluate(policy, inputs)` is the spec signature and treats subject-side
   matchers as wildcards; the runtime and re-derivation use the subject-aware
   `evaluate_detailed` / `gate_verdict`.
3. `require_human`: no human signal is representable in `GateInputs`, so
   Allow-rules with `require_human` Hold (conservative); reserved for
   GRANT-linked evaluation.
4. `sweep(log)` never re-judges policy staleness — a policy rotation
   retro-stales gates at admission time only; re-flagging landed effects
   would be anachronistic.
5. borsh 1.8: the `derive` feature is no longer default and is enabled
   explicitly in `Cargo.toml`.

## Test status

`cargo test` — 14 passed, 0 failed. `cargo clippy --all-targets -- -D
warnings` — clean. Coverage: happy path; no-gate / gate-deny / stale-policy /
inputs-drift / forbidden-alarm (prefix-veto and window-alarm paths) /
blast-radius>50 rejections; canary read ⇒ ALERT + auto-Hold; rederive
bit-for-bit over a 1000-event const-seeded fixture; GAP sweep (orphan effect,
dangling read, duty-seq hole); wire-layout size assertions matching this
document; policy VM unit checks; `admit` purity.
