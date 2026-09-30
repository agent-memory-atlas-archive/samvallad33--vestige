//! The policy VM. `Policy` is an ordered rule list; first matching rule wins;
//! the default is Deny. Deterministic: no clocks, no floats, no env.

use borsh::{BorshDeserialize, BorshSerialize};

use crate::inputs::GateInputs;
use crate::log::hash32;
use crate::record::{Propose, Verdict};

/// `Rule.match_params_hash_prefix` value meaning "match any params hash".
/// A real params hash starting with eight zero bytes would collide; accepted
/// and documented (blake3 output makes this unreachable in practice).
pub const WILDCARD_PREFIX: [u8; 8] = [0u8; 8];

/// `Rule.match_kind` value meaning "match any action kind" (0..=3 are the
/// real kinds, 255 is unambiguous).
pub const ANY_KIND: u8 = 255;

/// One policy rule. Subject-side matchers (`match_kind`,
/// `match_params_hash_prefix`) are checked against the PROPOSE; input-side
/// constraints (`max_blast_radius`) against the `GateInputs`.
/// `forbid_forgotten_lessons` and `require_human` are verdict modifiers, not
/// matchers — see [`evaluate_detailed`].
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Rule {
    /// PROPOSE.action_kind to match, or [`ANY_KIND`].
    pub match_kind: u8,
    /// First 8 bytes of PROPOSE.params_hash, or [`WILDCARD_PREFIX`].
    pub match_params_hash_prefix: [u8; 8],
    /// Rule matches only if `inputs.blast_radius.closure_size` <= this.
    pub max_blast_radius: u32,
    /// If true and any forgotten lesson is present, effective verdict is Deny.
    pub forbid_forgotten_lessons: bool,
    /// If true, an otherwise-Allow verdict is held pending human approval.
    /// No human signal is representable in `GateInputs`, so in this runtime
    /// such rules always Hold — conservative by construction.
    pub require_human: bool,
    pub verdict: Verdict,
}

/// An ordered rule list. `policy_hash = blake3(borsh(Policy))`.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Default)]
pub struct Policy {
    pub rules: Vec<Rule>,
}

/// blake3(borsh(policy)).
pub fn policy_hash(policy: &Policy) -> [u8; 32] {
    let bytes = borsh::to_vec(policy).expect("borsh Policy is infallible");
    hash32(&bytes)
}

/// Why a matching rule's verdict was modified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Veto {
    /// Rule forbids forgotten lessons and `inputs.forgotten_lessons` is
    /// non-empty (a LESSON_ALARM below the forget floor matched).
    ForbiddenLessons,
    /// Rule requires human approval; none is representable here.
    HumanRequired,
}

impl Policy {
    pub fn new(rules: Vec<Rule>) -> Self {
        Self { rules }
    }

    /// blake3(borsh(self)).
    pub fn policy_hash(&self) -> [u8; 32] {
        policy_hash(self)
    }
}

fn effective(rule: &Rule, inputs: &GateInputs) -> (Verdict, Option<Veto>) {
    if rule.forbid_forgotten_lessons && !inputs.forgotten_lessons.is_empty() {
        return (Verdict::Deny, Some(Veto::ForbiddenLessons));
    }
    if rule.require_human && rule.verdict == Verdict::Allow {
        return (Verdict::Hold, Some(Veto::HumanRequired));
    }
    (rule.verdict, None)
}

fn matches(
    rule: &Rule,
    kind: Option<u8>,
    params_hash: Option<&[u8; 32]>,
    inputs: &GateInputs,
) -> bool {
    if let Some(k) = kind
        && rule.match_kind != ANY_KIND
        && rule.match_kind != k
    {
        return false;
    }
    if let Some(ph) = params_hash
        && rule.match_params_hash_prefix != WILDCARD_PREFIX
        && rule.match_params_hash_prefix[..] != ph[..8]
    {
        return false;
    }
    inputs.blast_radius.closure_size <= rule.max_blast_radius
}

/// Evaluate with a wildcard subject: only input-side constraints apply
/// (subject-side matchers are treated as matching). First match wins; default
/// Deny. This is the spec-level VM entry point; the runtime and re-derivation
/// use the subject-aware [`evaluate_detailed`] / [`evaluate_for`].
pub fn evaluate(policy: &Policy, inputs: &GateInputs) -> Verdict {
    for rule in &policy.rules {
        if matches(rule, None, None, inputs) {
            return effective(rule, inputs).0;
        }
    }
    Verdict::Deny
}

/// The first rule matching the subject and inputs (ignoring verdict
/// modifiers), or `None` when nothing matches (default Deny). Exposed so
/// admission can ask whether the winning rule forbids forgotten lessons.
pub fn first_matching_rule<'a>(
    policy: &'a Policy,
    propose: &Propose,
    inputs: &GateInputs,
) -> Option<&'a Rule> {
    policy.rules.iter().find(|r| {
        matches(
            r,
            Some(propose.action_kind),
            Some(&propose.params_hash),
            inputs,
        )
    })
}

/// Subject-aware evaluation without the canary clamp. Returns the verdict and
/// the veto cause when a modifier fired.
pub fn evaluate_detailed(
    policy: &Policy,
    propose: &Propose,
    inputs: &GateInputs,
) -> (Verdict, Option<Veto>) {
    for rule in &policy.rules {
        if matches(
            rule,
            Some(propose.action_kind),
            Some(&propose.params_hash),
            inputs,
        ) {
            return effective(rule, inputs);
        }
    }
    (Verdict::Deny, None)
}

/// Subject-aware evaluation (convenience over [`evaluate_detailed`]).
pub fn evaluate_for(policy: &Policy, propose: &Propose, inputs: &GateInputs) -> Verdict {
    evaluate_detailed(policy, propose, inputs).0
}

/// The single verdict function used by BOTH `GateRuntime::commit_gate` and
/// [`crate::rederive_verdicts`], so stored and re-derived verdicts are
/// bit-for-bit comparable: evaluate for the subject, then apply the canary
/// clamp — any Allow over a prefix containing canary trips (`canary_hits` > 0)
/// is held.
pub fn gate_verdict(policy: &Policy, propose: &Propose, inputs: &GateInputs) -> Verdict {
    let v = evaluate_for(policy, propose, inputs);
    if v == Verdict::Allow && inputs.canary_hits > 0 {
        Verdict::Hold
    } else {
        v
    }
}
