//! Event surface: the trait every strata log record must satisfy.

use borsh::{BorshDeserialize, BorshSerialize};

/// A borsh-serializable log event with a total order.
///
/// `seq()` is TIME for this kernel: a strictly increasing `u64` sequence
/// number. The kernel never reads a wall clock; elapsed time between two
/// reviews of a card is `later_seq - earlier_seq`.
pub trait StrataEvent: borsh::BorshSerialize {
    /// Monotonic sequence number of this event (the event's timestamp).
    fn seq(&self) -> u64;
}

/// A spaced-repetition review. Time is `event_seq`, never a wall clock.
///
/// Canonical borsh layout (fixed order): `card_id: u64`, `rating: u8`,
/// `event_seq: u64`. `rating` uses the FSRS convention `1..=4`
/// (again / hard / good / easy); the fold clamps out-of-range values to the
/// nearest bound rather than rejecting, so no well-formed log byte sequence
/// can make the fold fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct ReviewEvent {
    /// Card being reviewed.
    pub card_id: u64,
    /// Answer rating, FSRS convention `1..=4`.
    pub rating: u8,
    /// Log sequence number (time).
    pub event_seq: u64,
}

impl StrataEvent for ReviewEvent {
    fn seq(&self) -> u64 {
        self.event_seq
    }
}
