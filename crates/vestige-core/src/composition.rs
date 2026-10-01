//! Backend-neutral GhostLink composition arithmetic.
//!
//! Pure numbers over recorded facts: hop counts over typed edges, how many
//! compositions a memory already belongs to, FSRS retention, and the exact
//! outcome types earlier weaves recorded. Nothing here reads content text,
//! tags, embeddings or term overlap. The legacy SQLite store and the Strata
//! log both score bridge candidates through these functions, so the two
//! backends cannot drift apart.

/// Outcome types a composition can record, in canonical order.
pub const OUTCOME_TYPES: &[&str] = &[
    "helpful",
    "dead_end",
    "submitted",
    "accepted",
    "rejected",
    "duplicate_risk",
    "needs_poc",
    "bad_severity",
    "user_promoted",
    "user_demoted",
    "closed_by_scope",
    "closed_by_duplicate",
    "closed_by_false_assumption",
    "closed_by_user",
    "expired_lane",
];

/// Outcomes that close a lane.
pub const CLOSED_OUTCOMES: &[&str] = &[
    "dead_end",
    "rejected",
    "bad_severity",
    "user_demoted",
    "closed_by_scope",
    "closed_by_false_assumption",
    "closed_by_user",
    "expired_lane",
];

/// Score adjustment from the outcome types earlier compositions of either
/// member recorded, clamped to `[-0.8, 0.5]`. Unknown types add nothing.
pub fn outcome_score_adjustment(prior_outcomes: &[String]) -> f64 {
    let mut adjustment: f64 = 0.0;
    for outcome in prior_outcomes {
        adjustment += match outcome.as_str() {
            "accepted" => 0.35,
            "helpful" => 0.25,
            "submitted" => 0.15,
            "user_promoted" => 0.20,
            "needs_poc" => -0.05,
            "duplicate_risk" => -0.35,
            "closed_by_duplicate" => -0.40,
            "dead_end"
            | "rejected"
            | "bad_severity"
            | "closed_by_scope"
            | "closed_by_false_assumption"
            | "closed_by_user"
            | "expired_lane" => -0.45,
            "user_demoted" => -0.20,
            _ => 0.0,
        };
    }
    adjustment.clamp(-0.8, 0.5)
}

/// Which way earlier outcomes point, as one label. Classifies the exact
/// outcome identifiers only.
pub fn outcome_signal(prior_outcomes: &[String]) -> String {
    if prior_outcomes.is_empty() {
        return "clean".to_string();
    }
    let has_closed = prior_outcomes
        .iter()
        .any(|outcome| CLOSED_OUTCOMES.contains(&outcome.as_str()));
    let has_duplicate = prior_outcomes
        .iter()
        .any(|outcome| matches!(outcome.as_str(), "duplicate_risk" | "closed_by_duplicate"));
    let has_success = prior_outcomes.iter().any(|outcome| {
        matches!(
            outcome.as_str(),
            "accepted" | "helpful" | "submitted" | "user_promoted"
        )
    });
    let has_needs_poc = prior_outcomes.iter().any(|outcome| outcome == "needs_poc");

    if (has_closed || has_duplicate) && has_success {
        "mixed_prior_outcomes".to_string()
    } else if has_closed {
        "prior_closed_door".to_string()
    } else if has_duplicate {
        "prior_duplicate_risk".to_string()
    } else if has_success {
        "prior_success".to_string()
    } else if has_needs_poc {
        "prior_needs_poc".to_string()
    } else {
        "prior_outcome".to_string()
    }
}

/// Novelty from composition degrees: `mean(1 / (1 + degree))`.
pub fn composition_novelty(first_degree: usize, second_degree: usize) -> f64 {
    ((1.0 / (1.0 + first_degree as f64)) + (1.0 / (1.0 + second_degree as f64))) / 2.0
}

/// Trust from the two members' retention: the mean, clamped to `0..=1`.
pub fn composition_trust(first_retention: f64, second_retention: f64) -> f64 {
    ((first_retention + second_retention) / 2.0).clamp(0.0, 1.0)
}

/// One bridge candidate's score parts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeScore {
    /// `1 / hops`.
    pub bridge: f64,
    /// `1.5 + bridge`.
    pub anchor: f64,
    /// `anchor + bridge * 2 + novelty * 1.5 + trust + outcome_adjustment`.
    pub score: f64,
}

/// The bridge-lens score (owner ruling 2026-09-28): hop proximity over
/// recorded typed edges, composition novelty, retention trust, and the
/// prior-outcome adjustment. `hops` is at least 1.
pub fn bridge_score(hops: u32, novelty: f64, trust: f64, outcome_adjustment: f64) -> BridgeScore {
    let bridge = 1.0 / f64::from(hops.max(1));
    let anchor = 1.5 + bridge;
    BridgeScore {
        bridge,
        anchor,
        score: anchor + (bridge * 2.0) + (novelty * 1.5) + trust + outcome_adjustment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn adjustment_sums_and_clamps() {
        assert_eq!(outcome_score_adjustment(&[]), 0.0);
        assert!((outcome_score_adjustment(&owned(&["helpful"])) - 0.25).abs() < 1e-12);
        assert_eq!(
            outcome_score_adjustment(&owned(&["accepted", "helpful", "submitted"])),
            0.5
        );
        assert_eq!(
            outcome_score_adjustment(&owned(&["dead_end", "rejected"])),
            -0.8
        );
        assert_eq!(outcome_score_adjustment(&owned(&["unknown"])), 0.0);
    }

    #[test]
    fn signal_classifies_exact_outcome_types() {
        assert_eq!(outcome_signal(&[]), "clean");
        assert_eq!(outcome_signal(&owned(&["helpful"])), "prior_success");
        assert_eq!(outcome_signal(&owned(&["dead_end"])), "prior_closed_door");
        assert_eq!(
            outcome_signal(&owned(&["dead_end", "accepted"])),
            "mixed_prior_outcomes"
        );
        assert_eq!(outcome_signal(&owned(&["needs_poc"])), "prior_needs_poc");
        assert_eq!(
            outcome_signal(&owned(&["duplicate_risk"])),
            "prior_duplicate_risk"
        );
    }

    #[test]
    fn bridge_score_matches_the_ruling_formula() {
        let score = bridge_score(
            2,
            composition_novelty(0, 1),
            composition_trust(0.8, 0.6),
            0.25,
        );
        let bridge = 0.5;
        let novelty = (1.0 + 0.5) / 2.0;
        let expected = (1.5 + bridge) + bridge * 2.0 + novelty * 1.5 + 0.7 + 0.25;
        assert!((score.score - expected).abs() < 1e-12);
        assert_eq!(score.bridge, bridge);
        assert_eq!(score.anchor, 2.0);
        assert_eq!(composition_trust(1.4, 1.2), 1.0);
    }
}
