//! Canonical encoding rules and quantization primitives.
//!
//! # Quantization spec (Q32.32, round-ties-to-even)
//!
//! Real-valued model quantities are stored in hashed state as signed 64-bit
//! Q32.32 fixed point: 32 integer bits, 32 fraction bits
//! (`Q_ONE = 1 << 32`). Conversion is defined by exactly one formula:
//!
//! ```text
//! to_q32_32(x)   = ( libm::rint(x * 2^32) ) as i64     // Rust saturating cast
//! from_q32_32(q) = (q as f64) / 2^32                   // exact (power of two)
//! ```
//!
//! * `libm::rint` is IEEE-754 roundTiesToEven on the scaled value, so values
//!   exactly halfway between two adjacent quanta round to the EVEN quantum
//!   (2.5 -> 2, 3.5 -> 4, -2.5 -> -2, -3.5 -> -4).
//! * The final `as i64` uses Rust's saturating float-to-int cast semantics:
//!   `NaN -> 0`, `+inf / overflow -> i64::MAX`, `-inf / underflow ->
//!   i64::MIN`.
//! * `from_q32_32` divides by a power of two, which is exact in `f64` for
//!   every `i64` quantum.
//!
//! Because `libm` is a pinned software implementation, the same `f64` input
//! quantizes to the same `i64` on every platform and toolchain.
//!
//! # State type allowlist
//!
//! Hashed state structs may use ONLY: `u8`, `u32`, `u64`, `i64`,
//! `[u8; N]`, `Vec<T>`, `Option<T>`, `BTreeMap<K, V>` (the only permitted
//! map), and field-limited enums. Floats, `String`, `bool`, `usize`, and
//! every other primitive are forbidden — including transitively (a
//! `Vec<Option<f64>>` is rejected just like a bare `f64`).
//!
//! Enforcement is derive-less and two-layered:
//!
//! 1. **Compile time**: [`lint_state_type!`] expands to a type-level
//!    assertion per field against [`AllowedStateField`], a sealed-ish trait
//!    implemented only for the allowlist above (nesting via the generic
//!    `Vec`/`Option`/`BTreeMap` impls). Enums opt in with
//!    [`strata_allowed_enum!`] on the author's assertion that variants nest
//!    only allowed types (a proc-macro could check this structurally; that
//!    is out of scope by contract).
//! 2. **Runtime (tests)**: [`lint_roundtrip`] is the `debug_assert` backstop
//!    — serialize a populated sample, deserialize, re-serialize, and require
//!    identical bytes.

use std::collections::BTreeMap;

/// Marker trait: a state type whose borsh encoding is canonical (integer
/// fields only, `BTreeMap`-only maps, no floats, no strings).
///
/// Implemented via [`lint_state_type!`]. Carries no methods: it is a
/// compile-time assertion plus a documentation hook.
pub trait HashableState: borsh::BorshSerialize + borsh::BorshDeserialize {}

/// Field-level allowlist enforced by the type system.
///
/// Implemented in this module for exactly the contract-allowed shapes;
/// enums opt in via [`strata_allowed_enum!`]. Everything else fails to
/// compile when used as a `lint_state_type!` field.
pub trait AllowedStateField: Sized {}

macro_rules! impl_allowed {
    ($($t:ty),+ $(,)?) => { $( impl AllowedStateField for $t {} )+ };
}

impl_allowed!(u8, u32, u64, i64);

/// Byte arrays of any length are float-free and canonical.
impl<const N: usize> AllowedStateField for [u8; N] {}

/// Containers recurse into their parameters, so `Vec<Option<f64>>` is
/// rejected exactly like a bare `f64`.
impl<T: AllowedStateField> AllowedStateField for Vec<T> {}
impl<T: AllowedStateField> AllowedStateField for Option<T> {}

/// `BTreeMap` is the ONLY permitted map (deterministic ascending iteration).
impl<K: AllowedStateField, V: AllowedStateField> AllowedStateField for BTreeMap<K, V> {}

/// Opt an enum into the state-field allowlist.
///
/// Asserts (author's responsibility; no proc-macro introspection by
/// contract) that every variant's fields — if any — are themselves allowed
/// types. The runtime backstop is [`lint_roundtrip`].
#[macro_export]
macro_rules! strata_allowed_enum {
    ($name:ident) => {
        impl $crate::canonical::AllowedStateField for $name {}
    };
}

/// Declare that a struct satisfies the canonical state-type allowlist and
/// implement [`HashableState`] for it.
///
/// ```ignore
/// lint_state_type!(State {
///     applied_seq: u64,
///     cards: BTreeMap<u64, CardState>,
/// });
/// ```
///
/// Each field's type must implement [`AllowedStateField`] or the crate
/// fails to compile at the expansion site with a missing-trait-bound error.
#[macro_export]
macro_rules! lint_state_type {
    ($name:ident { $($field:ident : $ty:ty),+ $(,)? }) => {
        impl $crate::canonical::HashableState for $name {}
        // A linted struct is itself canonical, so it may nest as a field
        // value inside other linted structs (e.g. BTreeMap<u64, CardState>).
        impl $crate::canonical::AllowedStateField for $name {}
        const _: () = {
            #[doc(hidden)]
            const fn __strata_assert_field_allowed<
                T: $crate::canonical::AllowedStateField,
            >() {}
            $( __strata_assert_field_allowed::<$ty>(); )+
        };
    };
}

/// One Q32.32 unit (`1.0`), i.e. `2^32` fraction ticks.
pub const Q_ONE: i64 = 1i64 << 32;

/// Fraction bit count of the Q32.32 format.
pub const FRAC_BITS: u32 = 32;

/// Quantize an `f64` to Q32.32 with round-ties-to-even (see module docs for
/// the exact formula and edge-case semantics).
#[inline]
pub fn to_q32_32(x: f64) -> i64 {
    let scale = 4_294_967_296.0; // 2^32
    libm::rint(x * scale) as i64
}

/// Dequantize a Q32.32 integer to `f64`. Exact for every input.
#[inline]
pub fn from_q32_32(q: i64) -> f64 {
    let scale = 4_294_967_296.0; // 2^32
    (q as f64) / scale
}

/// Runtime canonicity backstop for [`HashableState`] types: serialize the
/// sample, deserialize it, re-serialize, and `debug_assert` byte equality.
/// Call this from a test with a non-trivial (multi-entry) sample.
///
/// This is the derive-less enforcement the contract asks for: no proc-macro,
/// no reflection — a non-canonical encoding cannot round-trip to identical
/// bytes.
pub fn lint_roundtrip<T>(sample: &T)
where
    T: HashableState + Clone,
{
    let b1 = borsh::to_vec(sample).expect("borsh serialize of HashableState is infallible");
    let back: T = borsh::from_slice(&b1).expect("borsh deserialize of HashableState is infallible");
    let b2 = borsh::to_vec(&back).expect("borsh serialize of HashableState is infallible");
    debug_assert_eq!(
        b1, b2,
        "state serialization is not canonical: {b1:?} vs {b2:?}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q32_32_exact_values() {
        assert_eq!(to_q32_32(0.0), 0);
        assert_eq!(to_q32_32(1.0), Q_ONE);
        assert_eq!(to_q32_32(-1.0), -Q_ONE);
        assert_eq!(to_q32_32(0.5), Q_ONE / 2);
        assert_eq!(to_q32_32(2.0), 2 * Q_ONE);
        assert_eq!(from_q32_32(Q_ONE), 1.0);
        assert_eq!(from_q32_32(-Q_ONE / 2), -0.5);
    }

    #[test]
    fn q32_32_ties_to_even() {
        // Values exactly halfway between adjacent quanta round to the EVEN quantum.
        assert_eq!(to_q32_32(2.5 / 4_294_967_296.0), 2);
        assert_eq!(to_q32_32(3.5 / 4_294_967_296.0), 4);
        assert_eq!(to_q32_32(-2.5 / 4_294_967_296.0), -2);
        assert_eq!(to_q32_32(-3.5 / 4_294_967_296.0), -4);
        assert_eq!(to_q32_32(0.5 / 4_294_967_296.0), 0);
        assert_eq!(to_q32_32(1.5 / 4_294_967_296.0), 2);
    }

    #[test]
    fn q32_32_saturation_and_nan() {
        assert_eq!(to_q32_32(f64::NAN), 0);
        assert_eq!(to_q32_32(f64::INFINITY), i64::MAX);
        assert_eq!(to_q32_32(f64::NEG_INFINITY), i64::MIN);
        assert_eq!(to_q32_32(f64::MAX), i64::MAX);
        assert_eq!(to_q32_32(f64::MIN), i64::MIN);
        assert_eq!(to_q32_32(-f64::MAX), i64::MIN);
    }

    #[test]
    fn q32_32_roundtrip_error_bound() {
        let x = 0.123_456_789_f64;
        let err = (from_q32_32(to_q32_32(x)) - x).abs();
        assert!(err <= 2f64.powi(-(FRAC_BITS as i32 + 1))); // half-ulp of the grid
    }

    #[test]
    fn q32_32_monotone_nondecreasing() {
        let mut prev = to_q32_32(-10.0);
        let mut x = -10.0;
        while x < 10.0 {
            let q = to_q32_32(x);
            assert!(q >= prev, "quantization must be monotone at {x}");
            prev = q;
            x += 0.001;
        }
    }
}
