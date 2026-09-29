//! Version-dispatched kernels.
//!
//! The kernel is generic over the event type `E: StrataEvent`. Concrete
//! event types provide a version table through [`VersionedFold`]; dispatch
//! then picks a `&'static` function pointer by `ALGORITHM_VERSION`, so a
//! log written under v1 replays under v1 constants forever, even after v2,
//! v3, … exist.

use core::fmt;

use crate::event::StrataEvent;
use crate::state::State;

/// The dispatch result type: a `'static` apply function over `E`.
///
/// This IS the contract signature `&'static dyn Fn(&mut State, &E)`, named.
pub type ApplyFn<E> = dyn Fn(&mut State, &E);

/// One `(version, apply_fn)` row of a [`VersionedFold`] kernel table.
pub type KernelTableEntry<E> = (u32, fn(&mut State, &E));

/// The requested algorithm version has no pinned kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownAlgoVersion(pub u32);

impl fmt::Display for UnknownAlgoVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown strata algorithm version {}", self.0)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for UnknownAlgoVersion {}

/// Event-type side of the dispatch table: maps algorithm versions to
/// `'static` fold function pointers for `Self`.
///
/// Implement this alongside [`StrataEvent`]; see `ReviewEvent` in
/// [`crate::fsrs`] for the reference implementation. `Self: 'static` is
/// required because the dispatch result is a `&'static dyn Fn(&mut State,
/// &Self)`.
pub trait VersionedFold: StrataEvent + 'static {
    /// `(version, apply_fn)` pairs, function pointers so the table can be a
    /// promoted `&'static` const.
    fn kernel_table() -> &'static [KernelTableEntry<Self>];
}

/// Resolve the pinned kernel for `version`.
///
/// The returned reference borrows from static memory (the version table),
/// never from `version`, so it is `'static` and freely stashable.
pub fn kernel_for<E: VersionedFold>(
    version: u32,
) -> Result<&'static ApplyFn<E>, UnknownAlgoVersion> {
    let table = E::kernel_table();
    for entry in table.iter() {
        if entry.0 == version {
            // &fn-ptr coerces to &dyn Fn; points into the promoted table.
            return Ok(&entry.1);
        }
    }
    Err(UnknownAlgoVersion(version))
}

/// A resolved, versioned kernel: the dispatch result wrapped for reuse.
///
/// `Kernel<E>` is the type callers hold across a replay; it applies one
/// event at a time and performs no I/O.
#[derive(Clone)]
pub struct Kernel<E: VersionedFold> {
    /// The pinned algorithm version this kernel folds under.
    pub algo_version: u32,
    apply: &'static ApplyFn<E>,
}

impl<E: VersionedFold> Kernel<E> {
    /// Resolve the kernel for `version` (fails for unknown versions).
    pub fn for_version(version: u32) -> Result<Self, UnknownAlgoVersion> {
        Ok(Kernel {
            algo_version: version,
            apply: kernel_for(version)?,
        })
    }

    /// Fold one event into `state`.
    pub fn apply(&self, state: &mut State, event: &E) {
        (self.apply)(state, event)
    }

    /// Fold many events, in iterator order.
    pub fn apply_all<'a, I>(&self, state: &mut State, events: I)
    where
        I: IntoIterator<Item = &'a E>,
        E: 'a,
    {
        for e in events {
            self.apply(state, e);
        }
    }
}

impl<E: VersionedFold> fmt::Debug for Kernel<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Kernel")
            .field("algo_version", &self.algo_version)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ReviewEvent;

    #[test]
    fn dispatch_resolves_distinct_kernels() {
        let k1 = kernel_for::<ReviewEvent>(1).unwrap();
        let k2 = kernel_for::<ReviewEvent>(2).unwrap();
        let ev = ReviewEvent {
            card_id: 1,
            rating: 4,
            event_seq: 1,
        };
        let mut s1 = State::default();
        let mut s2 = State::default();
        k1(&mut s1, &ev);
        k2(&mut s2, &ev);
        // v1/v2 differ at w3 (initial stability, rating 4): fold output differs.
        assert_ne!(s1.cards[&1].stability_q, s2.cards[&1].stability_q);
        assert!(kernel_for::<ReviewEvent>(7).is_err());
    }

    #[test]
    fn kernel_struct_applies() {
        let k = Kernel::<ReviewEvent>::for_version(1).unwrap();
        let mut st = State::default();
        k.apply(
            &mut st,
            &ReviewEvent {
                card_id: 1,
                rating: 3,
                event_seq: 1,
            },
        );
        assert_eq!(st.cards.len(), 1);
        assert_eq!(st.applied_seq, 1);
        k.apply_all(
            &mut st,
            &[ReviewEvent {
                card_id: 1,
                rating: 4,
                event_seq: 2,
            }],
        );
        assert_eq!(st.cards[&1].review_count, 2);
    }
}
