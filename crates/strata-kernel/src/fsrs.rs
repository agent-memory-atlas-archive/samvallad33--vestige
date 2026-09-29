//! FSRS-6-shaped fold: quantized spaced-repetition state, version-pinned
//! weights, derived-only retrievability.
//!
//! # What this is (and is not)
//!
//! This is the FSRS-6 *formula shape* with v1-approximate constants. Shapes
//! implemented:
//!
//! * power-law retrievability `R(t, S) = (1 + FACTOR * t / S)^decay` with
//!   trainable factor and decay (FSRS-6 generalizes FSRS-4.5's fixed
//!   `decay = -0.5`, `FACTOR = 19/81`);
//! * initial stability per rating;
//! * initial difficulty `D0(G) = w4 - exp(w5*(G-1)) + w6`, clamped to
//!   `[1, 10]`;
//! * difficulty update with linear delta and FSRS-5-style mean reversion
//!   toward `D0(4)`;
//! * recall stability with difficulty decay:
//!   `S' = S * (1 + e^w9 * (11 - D) * S^(-w10) * (e^(w11*(1-R)) - 1) * hard * easy)`;
//! * forget stability
//!   `S' = w14 * D^(-w15) * ((S+1)^w16 - 1) * e^(w17*(1-R))`, never above `S`;
//! * a short-term (zero elapsed-seq) update of the FSRS-5 same-day shape.
//!
//! The constants are approximations pinned to an algorithm version. They
//! are deliberately load-bearing: **changing ANY weight or formula here
//! means minting a new `ALGO_*` version, never editing in place** — old
//! logs must replay under the constants they were written with, forever.
//!
//! # Storage rule
//!
//! Stability and difficulty live in [`CardState`] ONLY as quantized Q32.32
//! `i64` ([`crate::canonical::to_q32_32`]). Inside a fold the values may be
//! dequantized to `f64`, evolved with [`libm`] math, and re-quantized —
//! deterministically, since every input to every operation is a pure
//! function of (event, quantized state, version-pinned weights).
//! Retrievability is DERIVED-ONLY ([`FsrsFold::retrievability`]); it is
//! never stored.
//!
//! # Weight layout (21 slots, u32 milli-units: value = milli / 1000)
//!
//! | idx | role                                              |
//! |-----|---------------------------------------------------|
//! | 0-3 | initial stability `S0(G)` for ratings 1..=4 (days)|
//! | 4-6 | initial difficulty: base, exp slope, offset       |
//! | 7   | difficulty delta slope                            |
//! | 8   | difficulty mean-reversion weight                  |
//! | 9   | recall gain log-scale                            |
//! | 10  | recall stability exponent (`S^-w10`)             |
//! | 11  | recall retrievability sensitivity                |
//! | 12  | hard-penalty multiplier (rating 2)               |
//! | 13  | easy-bonus multiplier (rating 4)                 |
//! | 14  | forget scale                                      |
//! | 15  | forget difficulty exponent (`D^-w15`)            |
//! | 16  | forget stability exponent (`(S+1)^w16`)          |
//! | 17  | forget retrievability sensitivity                 |
//! | 18  | short-term log-gain                              |
//! | 19  | retrievability `FACTOR` (milli, ~0.235 = 19/81)   |
//! | 20  | retrievability decay offset (`decay = -(0.5+w20)`)|

use crate::canonical::{from_q32_32, to_q32_32};
use crate::event::ReviewEvent;
use crate::kernel::{UnknownAlgoVersion, VersionedFold};
use crate::lint_state_type;
use crate::state::State;
use crate::strata_allowed_enum;
use borsh::{BorshDeserialize, BorshSerialize};

/// Algorithm version 1 (first pinned constant set).
pub const ALGO_V1: u32 = 1;
/// Algorithm version 2 (same shapes, different pinned constants).
pub const ALGO_V2: u32 = 2;

/// v1 weights, milli-units. FSRS-6-shaped approximation.
pub const V1_WEIGHTS_MILLI: [u32; 21] = [
    400, 600, 2400, 5800, // 0..3: S0(G=1..=4)
    7195, 535, 1460, // 4..6: D0 base / exp slope / offset
    520, 200, // 7..8: difficulty delta slope / mean-reversion
    500, 140, 940, // 9..11: recall gain / S exponent / R sensitivity
    620, 1450, // 12..13: hard penalty / easy bonus
    2180, 50, 340, 1260, // 14..17: forget scale / D exp / S exp / R sensitivity
    460,  // 18: short-term log-gain
    235, 0, // 19..20: retrievability FACTOR / decay offset
];

/// v2 weights, milli-units. Identical shapes; differs from v1 at indices
/// 3, 7, 8, 9, 10, 11, 19 — enough to change every state root while both
/// versions stay replayable forever.
pub const V2_WEIGHTS_MILLI: [u32; 21] = [
    400, 600, 2400, 6200, //
    7195, 535, 1460, //
    545, 210, //
    540, 165, 965, //
    620, 1450, //
    2180, 50, 340, 1260, //
    460,  //
    242, 0, //
];

/// Look up the pinned weight table for an algorithm version.
pub fn weights_for(version: u32) -> Result<&'static [u32; 21], UnknownAlgoVersion> {
    match version {
        ALGO_V1 => Ok(&V1_WEIGHTS_MILLI),
        ALGO_V2 => Ok(&V2_WEIGHTS_MILLI),
        other => Err(UnknownAlgoVersion(other)),
    }
}

/// Upper bound for stability (days). Matches FSRS ecosystem conventions.
const S_MAX: f64 = 36500.0;
/// Lower bound for stability (days).
const S_MIN: f64 = 0.01;
/// Difficulty domain bounds (FSRS convention: 1 = easiest, 10 = hardest).
const D_MIN: f64 = 1.0;
const D_MAX: f64 = 10.0;

/// Learning phase of a card. Fieldless enum: borsh-encodes as a `u8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum CardPhase {
    /// Has at least one review recorded (initial phase after first review at
    /// rating <= 2).
    Learning,
    /// In the normal review cycle.
    Review,
    /// Recovering from a lapse.
    Relearning,
}

strata_allowed_enum!(CardPhase);

/// Quantized per-card FSRS state. Every real-valued quantity is Q32.32
/// `i64`; there are no floats here by construction (enforced by
/// [`crate::canonical::lint_state_type!`]).
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct CardState {
    /// Stability in days, Q32.32.
    pub stability_q: i64,
    /// Difficulty in `[1, 10]`, Q32.32.
    pub difficulty_q: i64,
    /// Seq of the last review of this card.
    pub last_seq: u64,
    /// Total reviews applied to this card.
    pub review_count: u32,
    /// Reviews answered "again" (rating 1).
    pub lapse_count: u32,
    /// Learning phase.
    pub phase: CardPhase,
}

lint_state_type!(CardState {
    stability_q: i64,
    difficulty_q: i64,
    last_seq: u64,
    review_count: u32,
    lapse_count: u32,
    phase: CardPhase,
});

/// The FSRS fold itself: pure, version-pinned, quantize-in / quantize-out.
pub struct FsrsFold;

impl FsrsFold {
    /// Apply one review to `state` under weight table `w`.
    ///
    /// Deterministic by construction: every operation is a pure function of
    /// `(state, event, w)` with [`libm`] transcendental math and Q32.32
    /// quantization at the boundary. Ratings are clamped into `1..=4`.
    pub fn fold(state: &mut State, event: &ReviewEvent, w: &[u32; 21]) {
        let g: f64 = match event.rating {
            0 => 1.0,
            1..=4 => f64::from(event.rating),
            _ => 4.0,
        };
        state.applied_seq = state.applied_seq.max(event.event_seq);

        match state.cards.get_mut(&event.card_id) {
            Some(card) => Self::update(card, g, event.event_seq, w),
            None => {
                state
                    .cards
                    .insert(event.card_id, Self::initial(g, event.event_seq, w));
            }
        }
    }

    /// Derived-only retrievability: probability the card is recallable at
    /// `current_seq`, under the constants of `version`. Never stored.
    ///
    /// `R(t, S) = (1 + FACTOR * t / S)^decay` with `t = current_seq -
    /// last_seq` (saturating; 0 for a not-yet-reviewed position), clamped to
    /// `[0, 1]`. Monotonically non-increasing in `current_seq`.
    pub fn retrievability(
        card: &CardState,
        current_seq: u64,
        version: u32,
    ) -> Result<f64, UnknownAlgoVersion> {
        let w = weights_for(version)?;
        let elapsed = current_seq.saturating_sub(card.last_seq);
        Ok(Self::r(Self::dequantized_stability(card), elapsed, w))
    }

    /// Whole days from `reviewed_at_ms` to `as_of_ms` (0 if `as_of` is earlier).
    ///
    /// The fold still uses sequence numbers. This only turns a caller-supplied
    /// review clock into the `t` that [`Self::retrievability`] already takes.
    /// Not a new algorithm version.
    pub fn elapsed_review_days(reviewed_at_ms: i64, as_of_ms: i64) -> u64 {
        let delta = as_of_ms.saturating_sub(reviewed_at_ms);
        if delta <= 0 {
            0
        } else {
            delta as u64 / 86_400_000
        }
    }

    /// Derived retrievability at `as_of_ms`.
    ///
    /// `Some(reviewed_at_ms)` measures `t` in whole days since that review.
    /// `None` (pre-field frames, or a source with no last-review time) keeps
    /// `t = fallback_seq - last_seq`.
    pub fn retrievability_at_review(
        card: &CardState,
        reviewed_at_ms: Option<i64>,
        as_of_ms: i64,
        fallback_seq: u64,
        version: u32,
    ) -> Result<f64, UnknownAlgoVersion> {
        let current = match reviewed_at_ms {
            Some(at) => card
                .last_seq
                .saturating_add(Self::elapsed_review_days(at, as_of_ms)),
            None => fallback_seq,
        };
        Self::retrievability(card, current, version)
    }

    fn dequantized_stability(card: &CardState) -> f64 {
        from_q32_32(card.stability_q).max(S_MIN)
    }

    /// Power-law forgetting curve. `base >= 1` always, `decay < 0` always,
    /// so the result lies in `[0, 1]`.
    fn r(s: f64, elapsed: u64, w: &[u32; 21]) -> f64 {
        let factor = milli(w, 19);
        let decay = -(0.5 + milli(w, 20));
        let base = 1.0 + factor * (elapsed as f64) / s;
        libm::pow(base, decay).clamp(0.0, 1.0)
    }

    fn initial(g: f64, seq: u64, w: &[u32; 21]) -> CardState {
        let s0 = milli(w, g as usize - 1).max(S_MIN);
        let d0 = Self::d0(g, w).clamp(D_MIN, D_MAX);
        CardState {
            stability_q: to_q32_32(s0),
            difficulty_q: to_q32_32(d0),
            last_seq: seq,
            review_count: 1,
            lapse_count: u32::from(g <= 1.0),
            phase: if g <= 2.0 {
                CardPhase::Learning
            } else {
                CardPhase::Review
            },
        }
    }

    /// Initial difficulty `D0(G) = w4 - exp(w5*(G-1)) + w6`.
    fn d0(g: f64, w: &[u32; 21]) -> f64 {
        milli(w, 4) - libm::exp(milli(w, 5) * (g - 1.0)) + milli(w, 6)
    }

    fn update(card: &mut CardState, g: f64, seq: u64, w: &[u32; 21]) {
        let s = Self::dequantized_stability(card);
        let d = from_q32_32(card.difficulty_q).clamp(D_MIN, D_MAX);
        let elapsed = seq.saturating_sub(card.last_seq);

        let s_next = if elapsed == 0 {
            // Short-term (same-seq) update, FSRS-5 same-day shape:
            // S' = S * exp(w18 * (G - 3)).
            s * libm::exp(milli(w, 18) * (g - 3.0))
        } else {
            let r = Self::r(s, elapsed, w);
            if g <= 1.0 {
                // Forget: w14 * D^-w15 * ((S+1)^w16 - 1) * e^(w17*(1-R)),
                // never allowed to increase stability.
                let sf = milli(w, 14)
                    * libm::pow(d, -milli(w, 15))
                    * (libm::pow(s + 1.0, milli(w, 16)) - 1.0)
                    * libm::exp(milli(w, 17) * (1.0 - r));
                sf.clamp(S_MIN, s)
            } else {
                // Recall with difficulty decay:
                // S' = S * (1 + e^w9 * (11-D) * S^-w10 * (e^(w11*(1-R))-1) * hard * easy)
                let hard = if g == 2.0 { milli(w, 12) } else { 1.0 };
                let easy = if g == 4.0 { milli(w, 13) } else { 1.0 };
                let inc = libm::exp(milli(w, 9))
                    * (11.0 - d)
                    * libm::pow(s, -milli(w, 10))
                    * (libm::exp(milli(w, 11) * (1.0 - r)) - 1.0)
                    * hard
                    * easy;
                s * (1.0 + inc)
            }
        };

        // Difficulty: linear delta + FSRS-5 mean reversion toward D0(4).
        let d_lin = d + milli(w, 7) * (3.0 - g);
        let d_next = {
            let revert = milli(w, 8).clamp(0.0, 1.0);
            let target = Self::d0(4.0, w).clamp(D_MIN, D_MAX);
            revert * target + (1.0 - revert) * d_lin
        };

        card.stability_q = to_q32_32(s_next.clamp(S_MIN, S_MAX));
        card.difficulty_q = to_q32_32(d_next.clamp(D_MIN, D_MAX));
        card.last_seq = seq;
        card.review_count = card.review_count.saturating_add(1);
        if g <= 1.0 {
            card.lapse_count = card.lapse_count.saturating_add(1);
            card.phase = CardPhase::Relearning;
        } else if g >= 3.0 {
            card.phase = CardPhase::Review;
        }
    }
}

/// Decode milli-unit weight `w[i]` to `f64`.
#[inline]
fn milli(w: &[u32; 21], i: usize) -> f64 {
    f64::from(w[i]) / 1000.0
}

/// v1 kernel entry (function pointer target for dispatch).
pub fn fold_v1(state: &mut State, event: &ReviewEvent) {
    FsrsFold::fold(state, event, &V1_WEIGHTS_MILLI);
}

/// v2 kernel entry (function pointer target for dispatch).
pub fn fold_v2(state: &mut State, event: &ReviewEvent) {
    FsrsFold::fold(state, event, &V2_WEIGHTS_MILLI);
}

impl VersionedFold for ReviewEvent {
    fn kernel_table() -> &'static [(u32, fn(&mut State, &Self))] {
        &[
            (ALGO_V1, fold_v1 as fn(&mut State, &ReviewEvent)),
            (ALGO_V2, fold_v2 as fn(&mut State, &ReviewEvent)),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrievability_is_monotone_decreasing_in_seq() {
        let card = CardState {
            stability_q: to_q32_32(10.0),
            difficulty_q: to_q32_32(5.0),
            last_seq: 100,
            review_count: 1,
            lapse_count: 0,
            phase: CardPhase::Review,
        };
        let mut prev = FsrsFold::retrievability(&card, 100, ALGO_V1).unwrap();
        assert_eq!(prev, 1.0); // zero elapsed -> certain recall
        for seq in 101..1100u64 {
            let r = FsrsFold::retrievability(&card, seq, ALGO_V1).unwrap();
            assert!(
                r <= prev,
                "R must be non-increasing: {r} > {prev} at seq {seq}"
            );
            assert!((0.0..=1.0).contains(&r));
            prev = r;
        }
        assert!(prev < 0.99); // and actually decays over 1000 seq
    }

    #[test]
    fn retrievability_across_both_versions_decays() {
        for v in [ALGO_V1, ALGO_V2] {
            let card = CardState {
                stability_q: to_q32_32(7.5),
                difficulty_q: to_q32_32(6.0),
                last_seq: 10,
                review_count: 3,
                lapse_count: 1,
                phase: CardPhase::Review,
            };
            let near = FsrsFold::retrievability(&card, 12, v).unwrap();
            let far = FsrsFold::retrievability(&card, 500, v).unwrap();
            assert!(near > far, "version {v}: {near} !> {far}");
        }
    }

    #[test]
    fn unknown_version_is_rejected() {
        assert!(weights_for(u32::MAX).is_err());
        let sample = CardState {
            stability_q: to_q32_32(5.0),
            difficulty_q: to_q32_32(5.0),
            last_seq: 1,
            review_count: 1,
            lapse_count: 0,
            phase: CardPhase::Review,
        };
        assert!(FsrsFold::retrievability(&sample, 2, 99).is_err());
    }

    #[test]
    fn first_review_matches_d0_s0_shapes() {
        let mut st = State::default();
        let w = &V1_WEIGHTS_MILLI;
        FsrsFold::fold(
            &mut st,
            &ReviewEvent {
                card_id: 1,
                rating: 1,
                event_seq: 1,
            },
            w,
        );
        let c = &st.cards[&1];
        assert_eq!(c.stability_q, to_q32_32(0.4)); // w0
        let expected_d0 = (7.195 - libm::exp(0.535 * 0.0) + 1.46).clamp(D_MIN, D_MAX);
        assert_eq!(c.difficulty_q, to_q32_32(expected_d0));
        assert_eq!(c.phase, CardPhase::Learning);
        assert_eq!(c.lapse_count, 1);
    }

    #[test]
    fn forgetting_never_increases_stability() {
        let mut st = State::default();
        let w = &V1_WEIGHTS_MILLI;
        // Long history of good reviews to build stability.
        for seq in 1..=40u64 {
            FsrsFold::fold(
                &mut st,
                &ReviewEvent {
                    card_id: 7,
                    rating: 4,
                    event_seq: seq,
                },
                w,
            );
        }
        let before = st.cards[&7].stability_q;
        FsrsFold::fold(
            &mut st,
            &ReviewEvent {
                card_id: 7,
                rating: 1,
                event_seq: 90,
            },
            w,
        );
        let after = st.cards[&7].stability_q;
        assert!(
            after < before,
            "lapse must reduce stability ({after} >= {before})"
        );
        assert_eq!(st.cards[&7].phase, CardPhase::Relearning);
    }

    #[test]
    fn recall_grows_stability_and_review_phase_resumes() {
        let mut st = State::default();
        let w = &V1_WEIGHTS_MILLI;
        FsrsFold::fold(
            &mut st,
            &ReviewEvent {
                card_id: 3,
                rating: 1,
                event_seq: 1,
            },
            w,
        );
        let s_after_lapse = st.cards[&3].stability_q;
        FsrsFold::fold(
            &mut st,
            &ReviewEvent {
                card_id: 3,
                rating: 3,
                event_seq: 30,
            },
            w,
        );
        let c = &st.cards[&3];
        assert!(c.stability_q > s_after_lapse);
        assert_eq!(c.phase, CardPhase::Review);
        assert_eq!(c.review_count, 2);
    }
}
