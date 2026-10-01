//! GhostLink over the recorded graph: the bridge and divergent lenses.
//!
//! GhostLink proposes memory pairs that were never composed. Every pair it
//! admits, ranks, pairs or explains carries its proof in recorded structure
//! only: node ids, exact scope / node_type / tag identity, typed edges, woven
//! composition records, FSRS state, and log sequence numbers. Content text,
//! embeddings, keyword or shared-term overlap and tag-name overlap never feed
//! admission, ranking, pairing or explanation.
//!
//! * **Bridge lens.** A pair is admitted only when one member reaches the
//!   other within [`BRIDGE_MAX_HOPS`] undirected hops over recorded
//!   `touched` / `derived_from` / `closed_by` edges. The shortest witness path
//!   (ties broken by id) is the proof.
//! * **Divergent lens.** A pair is eligible only when NO recorded edge of any
//!   kind joins its members. `Path_min` is the shortest undirected path over
//!   every recorded edge within [`DIVERGENT_RADIUS`]; divergence is measured
//!   on typed neighbor-id sets only,
//!   `1 - |Nt(a) & Nt(b)| / sqrt(|Nt(a)| * |Nt(b)|)`, and the synthesis score
//!   is `min(Path_min, 7) * divergence`. When either typed set is empty
//!   nothing is measured: the pair is a forced juxtaposition picked by a
//!   deterministic spread sampler, never by an argmax.
//!
//! Legacy `legacy_inferred` edges (carried over from v3) can only shorten a
//! path. They can only dampen a divergent score or remove the one pair they
//! join from eligibility; they never admit a bridge pair, never add a
//! candidate and never raise a score.
//!
//! Every read here is a read: nothing is appended to the log.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet};

use crate::error::StoreError;
use crate::store::StrataStore;
use crate::types::{EdgeKind, NodeRecord};

/// `node_type` of a woven composition record.
pub const COMPOSITION_NODE_TYPE: &str = "composition";
/// Tag every GhostLink write carries.
pub const GHOSTLINK_TAG: &str = "ghostlink";
/// Tag a composition record carries (exact identity, never a prefix match).
pub const WEAVE_TAG: &str = "ghostlink-weave";
/// Source-system prefix of a composition record: `ghostlink-weave:<a>:<b>`.
pub const WEAVE_SOURCE_PREFIX: &str = "ghostlink-weave:";
/// Tag prefix naming a record's outcome type (`outcome:<type>`).
pub const OUTCOME_TAG_PREFIX: &str = "outcome:";
/// Tag prefix naming the lens a record was woven from (`lens:<lens>`).
pub const LENS_TAG_PREFIX: &str = "lens:";
/// `link_type` the v3 migration stored for every inferred v3 edge.
pub const LEGACY_INFERRED: &str = "legacy_inferred";
/// The edge kinds that admit a bridge pair and build a typed profile.
pub const ADMITTING_KINDS: [EdgeKind; 3] =
    [EdgeKind::Touched, EdgeKind::DerivedFrom, EdgeKind::ClosedBy];
/// Bridge admission radius (undirected hops over admitting edges).
pub const BRIDGE_MAX_HOPS: u32 = 3;
/// Divergent `Path_min` radius (undirected hops over every recorded edge).
pub const DIVERGENT_RADIUS: u32 = 6;
/// The distance a pair beyond [`DIVERGENT_RADIUS`] is scored at.
pub const BEYOND_RADIUS: u32 = DIVERGENT_RADIUS + 1;
/// Typed-profile members the measured lane evaluates exactly per page.
pub const MEASURED_MEMBER_CAP: usize = 2_000;

const CURSOR_PREFIX: &str = "gl1";
const NO_PARENT: u32 = u32::MAX;

/// `ghostlink-weave:<a>:<b>` with the ids in order.
pub fn weave_source(first: &str, second: &str) -> String {
    let (a, b) = if first <= second {
        (first, second)
    } else {
        (second, first)
    };
    format!("{WEAVE_SOURCE_PREFIX}{a}:{b}")
}

/// The ordered pair a weave source names, or `None` for any other text.
pub fn parse_weave_source(system: &str) -> Option<(String, String)> {
    let rest = system.strip_prefix(WEAVE_SOURCE_PREFIX)?;
    let (a, b) = rest.split_once(':')?;
    if a.is_empty() || b.is_empty() || b.contains(':') || a >= b {
        return None;
    }
    Some((a.to_string(), b.to_string()))
}

/// The pair a composition record weaves, when `record` has the exact shape:
/// node_type [`COMPOSITION_NODE_TYPE`], tag [`WEAVE_TAG`], and a weave source.
pub fn composition_pair(record: &NodeRecord) -> Option<(String, String)> {
    if record.node_type != COMPOSITION_NODE_TYPE || !record.tags.iter().any(|tag| tag == WEAVE_TAG)
    {
        return None;
    }
    record
        .source
        .as_ref()
        .and_then(|source| parse_weave_source(&source.system))
}

/// One live woven composition record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositionRecord {
    /// The record's node id.
    pub id: String,
    /// Lower member id.
    pub first_id: String,
    /// Higher member id.
    pub second_id: String,
    /// `outcome:<type>` tag value.
    pub outcome_type: Option<String>,
    /// `lens:<lens>` tag value.
    pub lens: Option<String>,
    /// Exact scope the record was written in.
    pub scope: String,
    /// Caller clock at weave time (unix ms).
    pub created_at_ms: i64,
    /// Gate-space seq of the admitting effect of the record's latest upsert.
    pub origin_seq: Option<u64>,
    /// Visible `derived_from` edges from the record to its members (0..=2).
    pub member_edges: usize,
    /// Both member edges are recorded and both members are live.
    pub complete: bool,
}

/// Which memories a lens considers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PoolFilter {
    /// Exact scope; `None` considers every scope.
    pub scope: Option<String>,
    /// Exact tags; a member must carry at least one. Empty is no filter.
    pub tags: Vec<String>,
}

/// One recorded edge on a witness path, in walk order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathStep {
    /// Node the walk leaves.
    pub from: String,
    /// Recorded `link_type`.
    pub kind: String,
    /// Node the walk reaches.
    pub to: String,
    /// The recorded edge points `to -> from` (walked against its direction).
    pub reversed: bool,
}

/// One bridge-lens candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCandidate {
    /// Lower member id.
    pub first_id: String,
    /// Higher member id.
    pub second_id: String,
    /// Undirected admitting-edge hops between them (1..=3).
    pub hops: u32,
    /// Shortest witness path from `first_id` to `second_id`.
    pub path: Vec<PathStep>,
}

/// Every bridge candidate in a pool, plus what admitted them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeReport {
    /// Log head (`last_acked_seq`) at query time.
    pub head_seq: u64,
    /// Pool members.
    pub pool_size: usize,
    /// Pool members with at least one admitting edge.
    pub pool_nodes_with_admitting_edges: usize,
    /// Visible admitting edges touching a pool member.
    pub admitting_edges_touching_pool: usize,
    /// Pairs within reach that a live composition record already wove.
    pub woven_pairs_excluded: usize,
    /// Candidates in `(first_id, second_id)` order.
    pub candidates: Vec<BridgeCandidate>,
}

/// One scored bridge pair in [`GhostSnapshot::bridge_top`]'s bounded heap.
/// Ordered so that a *worse* pair compares greater: the max-heap's top is
/// the pair to drop, and `into_sorted_vec` yields best first.
#[derive(Debug, Clone, Copy)]
struct Kept {
    score: f64,
    a: u32,
    b: u32,
    hops: u32,
}

impl PartialEq for Kept {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Kept {}

impl PartialOrd for Kept {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Kept {
    fn cmp(&self, other: &Self) -> Ordering {
        // Lower score is worse; on a tie the larger id pair is worse (ids
        // are indexed in id order).
        other
            .score
            .total_cmp(&self.score)
            .then((self.a, self.b).cmp(&(other.a, other.b)))
    }
}

/// Divergent lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// Both typed profiles exist: divergence is measured.
    Measured,
    /// A typed profile is empty: the pair is forced, not found.
    Juxtaposition,
}

impl Lane {
    /// Wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Lane::Measured => "measured",
            Lane::Juxtaposition => "juxtaposition",
        }
    }
}

/// How the shortest recorded path between a divergent pair runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathVia {
    /// A shortest path exists over typed vocabulary edges alone.
    Typed,
    /// Only a path through a `legacy_inferred` (or other non-vocabulary)
    /// edge is that short.
    LegacyInferred,
    /// No recorded path within [`DIVERGENT_RADIUS`].
    None,
}

impl PathVia {
    /// Wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            PathVia::Typed => "typed",
            PathVia::LegacyInferred => "legacy_inferred",
            PathVia::None => "none",
        }
    }
}

/// One eligible divergent pair with its proof.
#[derive(Debug, Clone, PartialEq)]
pub struct DivergentEval {
    /// Lower member id.
    pub first_id: String,
    /// Higher member id.
    pub second_id: String,
    /// Lane the pair belongs to.
    pub lane: Lane,
    /// Shortest recorded path over every edge kind; `None` beyond the radius.
    pub path_min: Option<u32>,
    /// Witness path when within the radius.
    pub path: Vec<PathStep>,
    /// Which edges the witness needs.
    pub path_via: PathVia,
    /// `[|Nt(a)|, |Nt(b)|]`.
    pub typed_neighbor_counts: [usize; 2],
    /// `|Nt(a) & Nt(b)|`.
    pub shared_typed_neighbors: usize,
    /// `1 - overlap`, measured lane only.
    pub divergence: Option<f64>,
    /// `min(Path_min, 7) * divergence`, measured lane only.
    pub score: Option<f64>,
}

/// Counts that explain a divergent page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DivergentSummary {
    /// Pool members.
    pub pool_size: usize,
    /// Pool members with a non-empty typed profile.
    pub nodes_with_typed_profile: usize,
    /// Pool members with no visible edge of any kind.
    pub isolated_nodes: usize,
    /// Distinct `legacy_inferred` links between two pool members.
    pub legacy_edges_in_pool: usize,
    /// Pool pairs whose only recorded link is `legacy_inferred`: exactly
    /// the pairs legacy edges remove from eligibility.
    pub legacy_only_pairs_in_pool: usize,
    /// Distinct typed vocabulary links between two pool members.
    pub typed_edges_in_pool: usize,
    /// Eligible pairs in the pool (no edge of any kind, never woven).
    pub eligible_pairs: u64,
    /// Eligible pairs whose members both have a typed profile.
    pub measured_eligible_pairs: u64,
    /// Eligible pairs with at least one empty typed profile.
    pub juxtaposition_eligible_pairs: u64,
    /// Typed-profile members the measured lane evaluated this page.
    pub measured_members_evaluated: usize,
    /// Typed-profile members past [`MEASURED_MEMBER_CAP`] (id order). Their
    /// pairs belong to the sampler, so every eligible pair stays reachable.
    pub measured_members_beyond_cap: usize,
    /// Sampler schedule positions scanned this page.
    pub positions_scanned: u64,
}

/// One divergent page.
#[derive(Debug, Clone, PartialEq)]
pub struct DivergentPage {
    /// Log head (`last_acked_seq`) at query time.
    pub head_seq: u64,
    /// Clock the sampler evaluated retention at ([`StrataStore::head_clock_ms`]).
    pub retention_as_of_ms: i64,
    /// Measured-lane pairs, best score first.
    pub measured: Vec<DivergentEval>,
    /// Forced juxtapositions, in sampler order.
    pub juxtaposition: Vec<DivergentEval>,
    /// Cursor for the next page; `None` when both lanes are exhausted.
    pub next_cursor: Option<String>,
    /// Counts behind the page.
    pub summary: DivergentSummary,
}

#[derive(Debug, Clone, Copy)]
struct Adj {
    to: u32,
    kind: u16,
    forward: bool,
}

struct Scratch {
    stamp: Vec<u32>,
    current: u32,
    dist: Vec<u32>,
    parent: Vec<(u32, u16, bool)>,
}

impl Scratch {
    fn new(n: usize) -> Self {
        Self {
            stamp: vec![0; n],
            current: 0,
            dist: vec![0; n],
            parent: vec![(NO_PARENT, 0, true); n],
        }
    }

    fn reset(&mut self) {
        self.current = self.current.wrapping_add(1);
        if self.current == 0 {
            self.stamp.iter_mut().for_each(|stamp| *stamp = 0);
            self.current = 1;
        }
    }

    fn seen(&self, node: u32) -> bool {
        self.stamp[node as usize] == self.current
    }

    fn visit(&mut self, node: u32, dist: u32, parent: (u32, u16, bool)) {
        let at = node as usize;
        self.stamp[at] = self.current;
        self.dist[at] = dist;
        self.parent[at] = parent;
    }

    fn distance(&self, node: u32) -> Option<u32> {
        self.seen(node).then(|| self.dist[node as usize])
    }
}

/// A position in the sampler's fixed pairing schedule.
///
/// Round `(s, p)` pairs spread-order slot `i` with slot `i + s` for every `i`
/// whose block `i / s` has parity `p`. Each round is a matching (a member
/// appears at most once), and every unordered pair of slots appears in
/// exactly one round, so walking the schedule never repeats a pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    s: usize,
    p: usize,
    i: usize,
}

impl Slot {
    const START: Slot = Slot { s: 1, p: 0, i: 0 };

    /// The first valid slot at or after `self`, or `None` at the end.
    fn normalize(mut self, n: usize) -> Option<Slot> {
        loop {
            if self.s == 0 || self.s >= n {
                return None;
            }
            let block = self.i / self.s;
            if block % 2 != self.p {
                self.i = (block + 1).checked_mul(self.s)?;
                continue;
            }
            if self.i.checked_add(self.s)? >= n {
                if self.p == 0 {
                    self.p = 1;
                    self.i = self.s;
                } else {
                    self.s += 1;
                    self.p = 0;
                    self.i = 0;
                }
                continue;
            }
            return Some(self);
        }
    }

    fn next(self, n: usize) -> Option<Slot> {
        Slot {
            i: self.i + 1,
            ..self
        }
        .normalize(n)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cursor {
    measured_offset: usize,
    slot: Option<Slot>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Ranked {
    score: f64,
    a: u32,
    b: u32,
}

impl Eq for Ranked {}

impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ranked {
    /// Greater ranks later: lower score, then higher ids.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .score
            .total_cmp(&self.score)
            .then(self.a.cmp(&other.a))
            .then(self.b.cmp(&other.b))
    }
}

/// A read-only view of the recorded graph for one GhostLink query.
///
/// Built from the store's derived maps without cloning content. Only
/// visible edges count: an edge with a retired (suppressed, undone or
/// superseded) endpoint is invisible, so a retired record can neither link
/// nor bridge two live memories.
pub struct GhostSnapshot<'s> {
    store: &'s StrataStore,
    filter: PoolFilter,
    head_seq: u64,
    head_clock_ms: i64,
    ids: Vec<&'s str>,
    index: HashMap<&'s str, u32>,
    kinds: Vec<&'s str>,
    kind_admitting: Vec<bool>,
    kind_typed: Vec<bool>,
    kind_legacy: Vec<bool>,
    adj: Vec<Vec<Adj>>,
    typed_profile: Vec<bool>,
    records: Vec<CompositionRecord>,
    record_nodes: HashSet<u32>,
    woven: HashSet<(u32, u32)>,
    weave_degree: HashMap<u32, usize>,
    outcomes: HashMap<u32, BTreeSet<String>>,
    pool: Vec<u32>,
    /// The measured lane's window: the first `measured_cap` typed pool
    /// members in id order. The lane owns exactly the pairs inside it.
    measured_window: Vec<bool>,
    measured_cap: usize,
    in_pool: Vec<bool>,
    scratch: RefCell<Scratch>,
    components: RefCell<Option<(Vec<u32>, Vec<u32>)>>,
}

fn find(parent: &mut [u32], mut node: u32) -> u32 {
    while parent[node as usize] != node {
        let up = parent[parent[node as usize] as usize];
        parent[node as usize] = up;
        node = up;
    }
    node
}

fn union(parent: &mut [u32], a: u32, b: u32) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        let (low, high) = if ra < rb { (ra, rb) } else { (rb, ra) };
        parent[high as usize] = low;
    }
}

fn intersect_count(a: &[u32], b: &[u32]) -> usize {
    let (mut i, mut j, mut count) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
            Ordering::Equal => {
                count += 1;
                i += 1;
                j += 1;
            }
        }
    }
    count
}

fn pairs(n: u64) -> u64 {
    n * n.saturating_sub(1) / 2
}

/// `(year, month)` of a unix-ms instant (proleptic Gregorian, UTC).
fn year_month(ms: i64) -> (i64, u32) {
    let days = ms.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month as u32)
}

impl<'s> GhostSnapshot<'s> {
    /// Snapshot the recorded graph and select the pool `filter` names.
    pub fn new(store: &'s StrataStore, filter: PoolFilter) -> Self {
        let nodes = store.node_map();
        let edges = store.edge_list();
        // A scoped snapshot walks only its own scope: a memory recorded in
        // another scope neither admits a pair nor appears in a proof path.
        // Artifact ids (edge endpoints with no memory record) stay walkable.
        let hidden = |id: &str| {
            nodes.get(id).is_some_and(|record| {
                !record.is_live()
                    || filter
                        .scope
                        .as_deref()
                        .is_some_and(|scope| record.scope != scope)
            })
        };

        let mut all: BTreeSet<&'s str> = nodes.keys().map(String::as_str).collect();
        let mut kind_names: BTreeSet<&'s str> = BTreeSet::new();
        for edge in edges {
            all.insert(edge.source_id.as_str());
            all.insert(edge.target_id.as_str());
            kind_names.insert(edge.link_type.as_str());
        }
        let ids: Vec<&'s str> = all.into_iter().collect();
        let index: HashMap<&'s str, u32> = ids
            .iter()
            .enumerate()
            .map(|(at, id)| (*id, at as u32))
            .collect();
        let kinds: Vec<&'s str> = kind_names.into_iter().collect();
        let kind_index: HashMap<&'s str, u16> = kinds
            .iter()
            .enumerate()
            .map(|(at, kind)| (*kind, at as u16))
            .collect();
        let kind_admitting = kinds
            .iter()
            .map(|kind| ADMITTING_KINDS.iter().any(|admit| admit.as_str() == *kind))
            .collect();
        let kind_typed = kinds
            .iter()
            .map(|kind| EdgeKind::parse(kind).is_some())
            .collect();
        let kind_legacy = kinds.iter().map(|kind| *kind == LEGACY_INFERRED).collect();

        let mut adj: Vec<Vec<Adj>> = vec![Vec::new(); ids.len()];
        for edge in edges {
            if edge.source_id == edge.target_id
                || hidden(&edge.source_id)
                || hidden(&edge.target_id)
            {
                continue;
            }
            let source = index[edge.source_id.as_str()];
            let target = index[edge.target_id.as_str()];
            let kind = kind_index[edge.link_type.as_str()];
            adj[source as usize].push(Adj {
                to: target,
                kind,
                forward: true,
            });
            adj[target as usize].push(Adj {
                to: source,
                kind,
                forward: false,
            });
        }
        for list in &mut adj {
            list.sort_unstable_by_key(|edge| (edge.to, edge.kind, !edge.forward));
            list.dedup_by_key(|edge| (edge.to, edge.kind, edge.forward));
        }

        let mut snapshot = Self {
            store,
            filter,
            head_seq: store.log().head().last_acked_seq,
            head_clock_ms: store.head_clock_ms(),
            typed_profile: vec![false; ids.len()],
            in_pool: vec![false; ids.len()],
            scratch: RefCell::new(Scratch::new(ids.len())),
            ids,
            index,
            kinds,
            kind_admitting,
            kind_typed,
            kind_legacy,
            adj,
            records: Vec::new(),
            record_nodes: HashSet::new(),
            woven: HashSet::new(),
            weave_degree: HashMap::new(),
            outcomes: HashMap::new(),
            pool: Vec::new(),
            measured_window: Vec::new(),
            measured_cap: MEASURED_MEMBER_CAP,
            components: RefCell::new(None),
        };
        for at in 0..snapshot.ids.len() {
            snapshot.typed_profile[at] = snapshot.adj[at]
                .iter()
                .any(|edge| snapshot.kind_admitting[edge.kind as usize]);
        }
        snapshot.load_records(nodes);
        snapshot.select_pool(nodes);
        snapshot.fill_measured_window();
        snapshot
    }

    fn fill_measured_window(&mut self) {
        self.measured_window = vec![false; self.ids.len()];
        for &at in self
            .pool
            .iter()
            .filter(|&&at| self.typed_profile[at as usize])
            .take(self.measured_cap)
        {
            self.measured_window[at as usize] = true;
        }
    }

    /// The same snapshot with a smaller measured window (tests exercise the
    /// cap without thousands of members).
    #[cfg(test)]
    pub(crate) fn with_measured_cap(mut self, cap: usize) -> Self {
        self.measured_cap = cap;
        self.fill_measured_window();
        self
    }

    fn load_records(&mut self, nodes: &BTreeMap<String, NodeRecord>) {
        let derived = self
            .kinds
            .iter()
            .position(|kind| *kind == EdgeKind::DerivedFrom.as_str());
        for (id, record) in nodes {
            if !record.is_live() {
                continue;
            }
            let Some((first, second)) = composition_pair(record) else {
                continue;
            };
            let at = self.index[id.as_str()];
            self.record_nodes.insert(at);
            let member_edge = |member: &str| -> bool {
                let (Some(kind), Some(&target)) = (derived, self.index.get(member)) else {
                    return false;
                };
                self.adj[at as usize]
                    .iter()
                    .any(|edge| edge.to == target && edge.forward && edge.kind as usize == kind)
            };
            let member_edges = usize::from(member_edge(&first)) + usize::from(member_edge(&second));
            let live = |member: &str| nodes.get(member).is_some_and(NodeRecord::is_live);
            let complete = member_edges == 2 && live(&first) && live(&second);
            let tag_value = |prefix: &str| {
                record
                    .tags
                    .iter()
                    .find_map(|tag| tag.strip_prefix(prefix))
                    .map(str::to_string)
            };
            let outcome_type = tag_value(OUTCOME_TAG_PREFIX);
            if complete {
                let (a, b) = (self.index[first.as_str()], self.index[second.as_str()]);
                self.woven.insert((a.min(b), a.max(b)));
                for member in [a, b] {
                    *self.weave_degree.entry(member).or_insert(0) += 1;
                    if let Some(outcome) = &outcome_type {
                        self.outcomes
                            .entry(member)
                            .or_default()
                            .insert(outcome.clone());
                    }
                }
            }
            self.records.push(CompositionRecord {
                id: id.clone(),
                first_id: first,
                second_id: second,
                outcome_type,
                lens: tag_value(LENS_TAG_PREFIX),
                scope: record.scope.clone(),
                created_at_ms: record.created_at_ms,
                origin_seq: self.store.origin_seq(id),
                member_edges,
                complete,
            });
        }
    }

    fn select_pool(&mut self, nodes: &BTreeMap<String, NodeRecord>) {
        for (id, record) in nodes {
            if !record.is_live() || composition_pair(record).is_some() {
                continue;
            }
            // Expired and future memories are not composition candidates
            // (docs/TOOL-CONTRACTS.md). Validity is read at the log's own
            // head clock, the clock retention is read at, so a page stays a
            // function of the log head.
            if record.valid_from_ms > self.head_clock_ms
                || record.valid_until_ms <= self.head_clock_ms
            {
                continue;
            }
            if self
                .filter
                .scope
                .as_deref()
                .is_some_and(|scope| record.scope != scope)
            {
                continue;
            }
            if !self.filter.tags.is_empty()
                && !record.tags.iter().any(|tag| self.filter.tags.contains(tag))
            {
                continue;
            }
            let at = self.index[id.as_str()];
            self.in_pool[at as usize] = true;
            self.pool.push(at);
        }
    }

    // ------------------------------------------------------------------
    // Accessors
    // ------------------------------------------------------------------

    /// Log head (`last_acked_seq`) this snapshot was taken at.
    pub fn head_seq(&self) -> u64 {
        self.head_seq
    }

    /// The pool the filter selected, in id order.
    pub fn pool_ids(&self) -> Vec<&'s str> {
        self.pool.iter().map(|&at| self.ids[at as usize]).collect()
    }

    /// Is `id` a pool member?
    pub fn in_pool(&self, id: &str) -> bool {
        self.index
            .get(id)
            .is_some_and(|&at| self.in_pool[at as usize])
    }

    /// Live composition records, in record-id order.
    pub fn records(&self) -> &[CompositionRecord] {
        &self.records
    }

    /// Is `id` a live composition record?
    pub fn is_record(&self, id: &str) -> bool {
        self.index
            .get(id)
            .is_some_and(|at| self.record_nodes.contains(at))
    }

    /// Count of complete live composition records `id` belongs to.
    pub fn weave_degree(&self, id: &str) -> usize {
        self.index
            .get(id)
            .and_then(|at| self.weave_degree.get(at))
            .copied()
            .unwrap_or(0)
    }

    /// Did a complete live composition record weave this pair?
    pub fn is_woven(&self, first: &str, second: &str) -> bool {
        match (self.index.get(first), self.index.get(second)) {
            (Some(&a), Some(&b)) => self.woven.contains(&(a.min(b), a.max(b))),
            _ => false,
        }
    }

    /// Outcome types of every complete live record either member belongs
    /// to, sorted and deduplicated (the SQLite prior-outcome rule).
    pub fn prior_outcomes(&self, first: &str, second: &str) -> Vec<String> {
        let mut out = BTreeSet::new();
        for id in [first, second] {
            if let Some(set) = self.index.get(id).and_then(|at| self.outcomes.get(at)) {
                out.extend(set.iter().cloned());
            }
        }
        out.into_iter().collect()
    }

    /// Is there a visible recorded edge of any kind between the two ids?
    pub fn linked(&self, first: &str, second: &str) -> bool {
        match (self.index.get(first), self.index.get(second)) {
            (Some(&a), Some(&b)) => self.linked_idx(a, b),
            _ => false,
        }
    }

    fn linked_idx(&self, a: u32, b: u32) -> bool {
        self.adj[a as usize]
            .binary_search_by_key(&b, |edge| edge.to)
            .is_ok()
    }

    fn woven_idx(&self, a: u32, b: u32) -> bool {
        self.woven.contains(&(a.min(b), a.max(b)))
    }

    fn typed_set(&self, at: u32) -> Vec<u32> {
        let mut out: Vec<u32> = self.adj[at as usize]
            .iter()
            .filter(|edge| self.kind_admitting[edge.kind as usize])
            .map(|edge| edge.to)
            .collect();
        out.dedup();
        out
    }

    /// Visible recorded neighbors of `id`: `(neighbor, link_type, forward)`,
    /// in neighbor-id order. `legacy_inferred` links are included only when
    /// asked for.
    pub fn recorded_neighbors(
        &self,
        id: &str,
        include_legacy: bool,
    ) -> Vec<(String, String, bool)> {
        let Some(&at) = self.index.get(id) else {
            return Vec::new();
        };
        self.adj[at as usize]
            .iter()
            .filter(|edge| include_legacy || !self.kind_legacy[edge.kind as usize])
            .map(|edge| {
                (
                    self.ids[edge.to as usize].to_string(),
                    self.kinds[edge.kind as usize].to_string(),
                    edge.forward,
                )
            })
            .collect()
    }

    /// The recorded strength (milli) of every edge touching `id`, keyed by
    /// `(other endpoint, kind)`; parallel edges keep the strongest. One pass
    /// over the log's edges, read as written: no score is derived.
    pub fn recorded_strengths(&self, id: &str) -> HashMap<(String, String), i64> {
        let mut out: HashMap<(String, String), i64> = HashMap::new();
        for edge in self.store.edge_list() {
            let other = if edge.source_id == id {
                &edge.target_id
            } else if edge.target_id == id {
                &edge.source_id
            } else {
                continue;
            };
            let slot = out
                .entry((other.clone(), edge.link_type.clone()))
                .or_insert(edge.strength_milli);
            *slot = (*slot).max(edge.strength_milli);
        }
        out
    }

    // ------------------------------------------------------------------
    // Walks
    // ------------------------------------------------------------------

    /// Level-synchronous BFS from `from` over edges whose kind passes
    /// `edge_ok`, up to `radius`. Frontiers are walked in id order and a
    /// node's parent is its first discoverer, so witnesses are deterministic
    /// with ties broken by id. Stops early when `target` is reached.
    fn bfs(
        &self,
        scratch: &mut Scratch,
        from: u32,
        radius: u32,
        edge_ok: &dyn Fn(u16) -> bool,
        target: Option<u32>,
    ) -> Option<u32> {
        scratch.reset();
        scratch.visit(from, 0, (NO_PARENT, 0, true));
        let mut frontier = vec![from];
        for depth in 1..=radius {
            let mut next = Vec::new();
            for &node in &frontier {
                for edge in &self.adj[node as usize] {
                    if !edge_ok(edge.kind) || scratch.seen(edge.to) {
                        continue;
                    }
                    scratch.visit(edge.to, depth, (node, edge.kind, edge.forward));
                    if Some(edge.to) == target {
                        return Some(depth);
                    }
                    next.push(edge.to);
                }
            }
            if next.is_empty() {
                break;
            }
            next.sort_unstable();
            frontier = next;
        }
        None
    }

    fn path_from(&self, scratch: &Scratch, from: u32, to: u32) -> Vec<PathStep> {
        let mut steps = Vec::new();
        let mut node = to;
        while node != from {
            let (parent, kind, forward) = scratch.parent[node as usize];
            if parent == NO_PARENT {
                break;
            }
            steps.push(PathStep {
                from: self.ids[parent as usize].to_string(),
                kind: self.kinds[kind as usize].to_string(),
                to: self.ids[node as usize].to_string(),
                reversed: !forward,
            });
            node = parent;
        }
        steps.reverse();
        steps
    }

    fn components(&self) -> std::cell::Ref<'_, Option<(Vec<u32>, Vec<u32>)>> {
        if self.components.borrow().is_none() {
            let n = self.ids.len();
            let mut all: Vec<u32> = (0..n as u32).collect();
            let mut typed: Vec<u32> = (0..n as u32).collect();
            for (at, list) in self.adj.iter().enumerate() {
                for edge in list {
                    if edge.to as usize > at {
                        union(&mut all, at as u32, edge.to);
                        if self.kind_typed[edge.kind as usize] {
                            union(&mut typed, at as u32, edge.to);
                        }
                    }
                }
            }
            for node in 0..n as u32 {
                let root = find(&mut all, node);
                all[node as usize] = root;
                let root = find(&mut typed, node);
                typed[node as usize] = root;
            }
            *self.components.borrow_mut() = Some((all, typed));
        }
        self.components.borrow()
    }

    /// Shortest distance (and witness) within `radius`; `typed_only`
    /// restricts the walk to vocabulary edges.
    fn distance(
        &self,
        a: u32,
        b: u32,
        radius: u32,
        typed_only: bool,
    ) -> Option<(u32, Vec<PathStep>)> {
        {
            let comps = self.components();
            let (all, typed) = comps.as_ref().expect("components computed");
            let comp = if typed_only { typed } else { all };
            if comp[a as usize] != comp[b as usize] {
                return None;
            }
        }
        let mut scratch = self.scratch.borrow_mut();
        let kind_typed = &self.kind_typed;
        let edge_ok = |kind: u16| !typed_only || kind_typed[kind as usize];
        let depth = self.bfs(&mut scratch, a, radius, &edge_ok, Some(b))?;
        let path = self.path_from(&scratch, a, b);
        Some((depth, path))
    }

    /// Shortest recorded path over vocabulary edges (no `legacy_inferred`),
    /// within `max_hops`, ties broken by id.
    pub fn recorded_path(&self, from: &str, to: &str, max_hops: u32) -> Option<Vec<PathStep>> {
        let (&a, &b) = (self.index.get(from)?, self.index.get(to)?);
        if a == b {
            return Some(Vec::new());
        }
        self.distance(a, b, max_hops, true).map(|(_, path)| path)
    }

    /// Nodes on some shortest vocabulary path between `from` and `to`
    /// (endpoints excluded) as `(id, hops from `from`)`, nearest first.
    pub fn recorded_bridges(&self, from: &str, to: &str, max_hops: u32) -> Vec<(String, u32)> {
        let (Some(&a), Some(&b)) = (self.index.get(from), self.index.get(to)) else {
            return Vec::new();
        };
        if a == b {
            return Vec::new();
        }
        let kind_typed = &self.kind_typed;
        let edge_ok = |kind: u16| kind_typed[kind as usize];
        let mut scratch = self.scratch.borrow_mut();
        self.bfs(&mut scratch, a, max_hops, &edge_ok, None);
        let Some(total) = scratch.distance(b) else {
            return Vec::new();
        };
        let from_a: HashMap<u32, u32> = (0..self.ids.len() as u32)
            .filter_map(|node| scratch.distance(node).map(|dist| (node, dist)))
            .collect();
        self.bfs(&mut scratch, b, max_hops, &edge_ok, None);
        let mut out: Vec<(String, u32)> = from_a
            .iter()
            .filter(|(node, _)| **node != a && **node != b)
            .filter_map(|(&node, &da)| {
                let db = scratch.distance(node)?;
                (da + db == total).then(|| (self.ids[node as usize].to_string(), da))
            })
            .collect();
        out.sort_by(|x, y| x.1.cmp(&y.1).then_with(|| x.0.cmp(&y.0)));
        out
    }

    // ------------------------------------------------------------------
    // Bridge lens
    // ------------------------------------------------------------------

    /// Every pool pair within [`BRIDGE_MAX_HOPS`] admitting-edge hops that
    /// no live record wove, in `(first_id, second_id)` order. Unbounded:
    /// propose ranks through [`Self::bridge_top`], which never holds more
    /// than the page it returns.
    pub fn bridge(&self) -> BridgeReport {
        let (report, _) = self.bridge_top(usize::MAX, |_, _, _| 0.0);
        report
    }

    /// The `k` best bridge pairs by `score(first_id, second_id, hops)`,
    /// best first, ties by `(first_id, second_id)`, plus how many pairs the
    /// walk admitted. BFS runs only from pool members that have an
    /// admitting edge, over admitting edges only. Each admitted pair is
    /// scored as the walk finds it and only the best `k` are kept, so a hub
    /// that admits millions of pairs costs `k` candidates of memory; proof
    /// paths are built for the kept pairs alone.
    pub fn bridge_top(
        &self,
        k: usize,
        mut score: impl FnMut(&str, &str, u32) -> f64,
    ) -> (BridgeReport, usize) {
        let admitting = &self.kind_admitting;
        let edge_ok = |kind: u16| admitting[kind as usize];
        let mut kept: BinaryHeap<Kept> = BinaryHeap::new();
        let mut admitted = 0usize;
        let mut woven_pairs_excluded = 0;
        let mut with_edges = 0;
        let mut touching: HashSet<(u32, u32, u16, bool)> = HashSet::new();
        {
            let mut scratch = self.scratch.borrow_mut();
            for &a in &self.pool {
                if !self.typed_profile[a as usize] {
                    continue;
                }
                with_edges += 1;
                for edge in &self.adj[a as usize] {
                    if admitting[edge.kind as usize] {
                        let key = if edge.forward {
                            (a, edge.to, edge.kind, true)
                        } else {
                            (edge.to, a, edge.kind, true)
                        };
                        touching.insert(key);
                    }
                }
                scratch.reset();
                scratch.visit(a, 0, (NO_PARENT, 0, true));
                let mut frontier = vec![a];
                for depth in 1..=BRIDGE_MAX_HOPS {
                    let mut next = Vec::new();
                    for &node in &frontier {
                        for edge in &self.adj[node as usize] {
                            if !edge_ok(edge.kind) || scratch.seen(edge.to) {
                                continue;
                            }
                            scratch.visit(edge.to, depth, (node, edge.kind, edge.forward));
                            next.push(edge.to);
                        }
                    }
                    if next.is_empty() {
                        break;
                    }
                    next.sort_unstable();
                    for &b in &next {
                        if b <= a || !self.in_pool[b as usize] {
                            continue;
                        }
                        if self.woven_idx(a, b) {
                            woven_pairs_excluded += 1;
                            continue;
                        }
                        admitted += 1;
                        if k == 0 {
                            continue;
                        }
                        kept.push(Kept {
                            score: score(self.ids[a as usize], self.ids[b as usize], depth),
                            a,
                            b,
                            hops: depth,
                        });
                        if kept.len() > k {
                            kept.pop();
                        }
                    }
                    frontier = next;
                }
            }
        }
        let kept = kept.into_sorted_vec();
        // Proof paths for the kept pairs only: one BFS per distinct source,
        // the same walk (level-synchronous, first discoverer, id order) that
        // admitted them.
        let mut targets: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for pair in &kept {
            targets.entry(pair.a).or_default().push(pair.b);
        }
        let mut paths: HashMap<(u32, u32), Vec<PathStep>> = HashMap::new();
        {
            let mut scratch = self.scratch.borrow_mut();
            for (a, bs) in targets {
                self.bfs(&mut scratch, a, BRIDGE_MAX_HOPS, &edge_ok, None);
                for b in bs {
                    paths.insert((a, b), self.path_from(&scratch, a, b));
                }
            }
        }
        let candidates = kept
            .into_iter()
            .map(|pair| BridgeCandidate {
                first_id: self.ids[pair.a as usize].to_string(),
                second_id: self.ids[pair.b as usize].to_string(),
                hops: pair.hops,
                path: paths.remove(&(pair.a, pair.b)).unwrap_or_default(),
            })
            .collect();
        (
            BridgeReport {
                head_seq: self.head_seq,
                pool_size: self.pool.len(),
                pool_nodes_with_admitting_edges: with_edges,
                admitting_edges_touching_pool: touching.len(),
                woven_pairs_excluded,
                candidates,
            },
            admitted,
        )
    }

    // ------------------------------------------------------------------
    // Divergent lens
    // ------------------------------------------------------------------

    /// Evaluate one pair under the divergent lens, or `None` when it is not
    /// eligible (not both in the pool, the same id, joined by any recorded
    /// edge, or woven).
    pub fn divergent_eval(&self, first: &str, second: &str) -> Option<DivergentEval> {
        let (&a, &b) = (self.index.get(first)?, self.index.get(second)?);
        self.divergent_eval_idx(a, b)
    }

    fn eligible_idx(&self, a: u32, b: u32) -> bool {
        a != b
            && self.in_pool[a as usize]
            && self.in_pool[b as usize]
            && !self.linked_idx(a, b)
            && !self.woven_idx(a, b)
    }

    fn divergent_eval_idx(&self, a: u32, b: u32) -> Option<DivergentEval> {
        let (a, b) = (a.min(b), a.max(b));
        if !self.eligible_idx(a, b) {
            return None;
        }
        let (ta, tb) = (self.typed_set(a), self.typed_set(b));
        let shared = intersect_count(&ta, &tb);
        let measured = !ta.is_empty() && !tb.is_empty();
        let divergence =
            measured.then(|| 1.0 - shared as f64 / ((ta.len() * tb.len()) as f64).sqrt());
        let (path_min, path, path_via) = match self.distance(a, b, DIVERGENT_RADIUS, false) {
            None => (None, Vec::new(), PathVia::None),
            Some((union_hops, union_path)) => match self.distance(a, b, DIVERGENT_RADIUS, true) {
                Some((typed_hops, typed_path)) if typed_hops == union_hops => {
                    (Some(union_hops), typed_path, PathVia::Typed)
                }
                _ => (Some(union_hops), union_path, PathVia::LegacyInferred),
            },
        };
        let score = divergence
            .map(|value| f64::from(path_min.unwrap_or(BEYOND_RADIUS).min(BEYOND_RADIUS)) * value);
        Some(DivergentEval {
            first_id: self.ids[a as usize].to_string(),
            second_id: self.ids[b as usize].to_string(),
            lane: if measured {
                Lane::Measured
            } else {
                Lane::Juxtaposition
            },
            path_min,
            path,
            path_via,
            typed_neighbor_counts: [ta.len(), tb.len()],
            shared_typed_neighbors: shared,
            divergence,
            score,
        })
    }

    fn filter_fingerprint(&self) -> String {
        let mut body = Vec::new();
        body.extend_from_slice(self.filter.scope.as_deref().unwrap_or("\u{0}*").as_bytes());
        for tag in &self.filter.tags {
            body.push(0x1f);
            body.extend_from_slice(tag.as_bytes());
        }
        blake3::hash(&body).to_hex()[..8].to_string()
    }

    fn encode_cursor(&self, cursor: Cursor) -> String {
        let (s, p, i) = match cursor.slot {
            Some(slot) => (slot.s, slot.p, slot.i),
            None => (0, 0, 0),
        };
        format!(
            "{CURSOR_PREFIX}.{}.{}.{}.{s}.{p}.{i}",
            self.head_seq,
            self.filter_fingerprint(),
            cursor.measured_offset
        )
    }

    fn decode_cursor(&self, text: &str) -> Result<Cursor, StoreError> {
        let bad = || {
            StoreError::InvalidInput(format!(
                "cursor '{text}' is not a GhostLink divergent cursor; omit it to start from the first page"
            ))
        };
        let parts: Vec<&str> = text.trim().split('.').collect();
        if parts.len() != 7 || parts[0] != CURSOR_PREFIX {
            return Err(bad());
        }
        let head: u64 = parts[1].parse().map_err(|_| bad())?;
        if head != self.head_seq {
            return Err(StoreError::InvalidInput(format!(
                "stale cursor: it was issued at log seq {head} and the log is now at {}; omit the cursor to restart from the first page",
                self.head_seq
            )));
        }
        if parts[2] != self.filter_fingerprint() {
            return Err(StoreError::InvalidInput(
                "cursor was issued for a different scope or tag filter; omit it to restart".into(),
            ));
        }
        let number = |at: usize| parts[at].parse::<usize>().map_err(|_| bad());
        let measured_offset = number(3)?;
        let (s, p, i) = (number(4)?, number(5)?, number(6)?);
        // Every cursor this snapshot issues stays inside its own pool: the
        // measured offset never passes the pool's pair count, and a slot
        // pairs positions `i` and `i + s` of an `n`-member order. Anything
        // else was not issued here, and paging on it would index past the
        // order.
        let n = self.pool.len();
        if measured_offset as u64 > pairs(n as u64) {
            return Err(bad());
        }
        let slot = if s == 0 {
            None
        } else {
            if p > 1 || i.checked_add(s).is_none_or(|end| end >= n) {
                return Err(bad());
            }
            Some(Slot { s, p, i })
        };
        Ok(Cursor {
            measured_offset,
            slot,
        })
    }

    /// Retention the sampler orders by: FSRS retrievability at the log's
    /// own head clock, so a page is a function of the log head alone.
    fn retention(&self, at: u32) -> f64 {
        self.store
            .retrievability_at(self.ids[at as usize], self.head_clock_ms)
            .ok()
            .flatten()
            .unwrap_or(0.0)
    }

    /// The sampler's member order: never-woven first, then retention
    /// descending, then id; bucketed by exact (scope, node_type, creation
    /// month) and interleaved round-robin so consecutive members come from
    /// different buckets.
    fn spread_order(&self) -> Vec<u32> {
        let nodes = self.store.node_map();
        let mut members: Vec<(bool, f64, u32)> = self
            .pool
            .iter()
            .map(|&at| {
                (
                    self.weave_degree.get(&at).copied().unwrap_or(0) > 0,
                    self.retention(at),
                    at,
                )
            })
            .collect();
        members.sort_by(|x, y| {
            x.0.cmp(&y.0)
                .then_with(|| y.1.total_cmp(&x.1))
                .then_with(|| x.2.cmp(&y.2))
        });
        let mut bucket_of: HashMap<(&str, &str, (i64, u32)), usize> = HashMap::new();
        let mut buckets: Vec<Vec<u32>> = Vec::new();
        for (_, _, at) in members {
            let record = &nodes[self.ids[at as usize]];
            let key = (
                record.scope.as_str(),
                record.node_type.as_str(),
                year_month(record.created_at_ms),
            );
            let slot = *bucket_of.entry(key).or_insert_with(|| {
                buckets.push(Vec::new());
                buckets.len() - 1
            });
            buckets[slot].push(at);
        }
        let mut order = Vec::with_capacity(self.pool.len());
        let mut round = 0;
        loop {
            let mut any = false;
            for bucket in &buckets {
                if let Some(&at) = bucket.get(round) {
                    order.push(at);
                    any = true;
                }
            }
            if !any {
                break;
            }
            round += 1;
        }
        order
    }

    fn summary(&self) -> DivergentSummary {
        let mut summary = DivergentSummary {
            pool_size: self.pool.len(),
            ..DivergentSummary::default()
        };
        // (has legacy link, has any other link) per pool pair.
        let mut links: HashMap<(u32, u32), (bool, bool)> = HashMap::new();
        // A reciprocal pair of edges of one kind is one link.
        let mut distinct: HashSet<(u32, u32, u16)> = HashSet::new();
        let mut typed_member = 0u64;
        for &a in &self.pool {
            if self.adj[a as usize].is_empty() {
                summary.isolated_nodes += 1;
            }
            if self.typed_profile[a as usize] {
                typed_member += 1;
            }
            for edge in &self.adj[a as usize] {
                if edge.to <= a || !self.in_pool[edge.to as usize] {
                    continue;
                }
                let entry = links.entry((a, edge.to)).or_insert((false, false));
                if self.kind_legacy[edge.kind as usize] {
                    entry.0 = true;
                } else {
                    entry.1 = true;
                }
                if !distinct.insert((a, edge.to, edge.kind)) {
                    continue;
                }
                if self.kind_legacy[edge.kind as usize] {
                    summary.legacy_edges_in_pool += 1;
                }
                if self.kind_typed[edge.kind as usize] {
                    summary.typed_edges_in_pool += 1;
                }
            }
        }
        summary.nodes_with_typed_profile = typed_member as usize;
        summary.legacy_only_pairs_in_pool = links
            .values()
            .filter(|(legacy, other)| *legacy && !*other)
            .count();
        let mut excluded: HashSet<(u32, u32)> = links.keys().copied().collect();
        for &(a, b) in &self.woven {
            if self.in_pool[a as usize] && self.in_pool[b as usize] {
                excluded.insert((a, b));
            }
        }
        let in_window = |at: u32| self.measured_window[at as usize];
        let window = self.pool.iter().filter(|&&at| in_window(at)).count() as u64;
        let excluded_measured = excluded
            .iter()
            .filter(|(a, b)| in_window(*a) && in_window(*b))
            .count() as u64;
        summary.measured_members_beyond_cap = (typed_member - window) as usize;
        summary.eligible_pairs = pairs(self.pool.len() as u64) - excluded.len() as u64;
        summary.measured_eligible_pairs = pairs(window) - excluded_measured;
        summary.juxtaposition_eligible_pairs =
            summary.eligible_pairs - summary.measured_eligible_pairs;
        summary
    }

    /// The best `take` measured pairs (score desc, then ids), how many
    /// eligible measured pairs were evaluated, and how many members.
    fn measured_top(&self, take: usize) -> (Vec<Ranked>, u64, usize) {
        let typed: Vec<u32> = self
            .pool
            .iter()
            .copied()
            .filter(|&at| self.measured_window[at as usize])
            .collect();
        let sets: HashMap<u32, Vec<u32>> =
            typed.iter().map(|&at| (at, self.typed_set(at))).collect();
        let mut heap: BinaryHeap<Ranked> = BinaryHeap::new();
        let mut eligible = 0u64;
        let mut scratch = self.scratch.borrow_mut();
        let edge_ok = |_: u16| true;
        for (at, &a) in typed.iter().enumerate() {
            self.bfs(&mut scratch, a, DIVERGENT_RADIUS, &edge_ok, None);
            // |Nt(a) & Nt(b)| for every b two typed hops away.
            let mut shared: HashMap<u32, usize> = HashMap::new();
            for &x in &sets[&a] {
                for edge in &self.adj[x as usize] {
                    if self.kind_admitting[edge.kind as usize] && edge.to != a {
                        *shared.entry(edge.to).or_insert(0) += 1;
                    }
                }
            }
            // A neighbor reached over two admitting edges of different kinds
            // counts once: recount against the deduplicated sets.
            for &b in &typed[at + 1..] {
                if self.linked_idx(a, b) || self.woven_idx(a, b) {
                    continue;
                }
                eligible += 1;
                let common = if shared.contains_key(&b) {
                    intersect_count(&sets[&a], &sets[&b])
                } else {
                    0
                };
                let overlap = common as f64 / ((sets[&a].len() * sets[&b].len()) as f64).sqrt();
                let hops = scratch
                    .distance(b)
                    .unwrap_or(BEYOND_RADIUS)
                    .min(BEYOND_RADIUS);
                let item = Ranked {
                    score: f64::from(hops) * (1.0 - overlap),
                    a,
                    b,
                };
                if take == 0 {
                    continue;
                }
                heap.push(item);
                if heap.len() > take {
                    heap.pop();
                }
            }
        }
        (heap.into_sorted_vec(), eligible, typed.len())
    }

    /// Walk the schedule from `start`, keeping each member at most once on
    /// the page. Beyond-radius pairs are taken first; pairs within the
    /// radius are deferred and fill the page by larger `Path_min`. Pairs the
    /// walk passes over are not revisited, so no pair repeats across pages.
    fn sample(
        &self,
        order: &[u32],
        start: Option<Slot>,
        need: usize,
    ) -> (Vec<(u32, u32)>, Option<Slot>, u64) {
        let n = order.len();
        let scan_budget = (need as u64 * 64).max(4_096);
        let eval_budget = (need * 16).max(256);
        let mut slot = start.and_then(|slot| slot.normalize(n));
        let mut used: HashSet<u32> = HashSet::new();
        let mut far: Vec<(u32, u32)> = Vec::new();
        let mut near: Vec<(u32, u64, u32, u32)> = Vec::new();
        let (mut scanned, mut evaluated) = (0u64, 0usize);
        while let Some(at) = slot {
            if far.len() >= need || scanned >= scan_budget || evaluated >= eval_budget {
                break;
            }
            let (x, y) = (order[at.i], order[at.i + at.s]);
            slot = at.next(n);
            scanned += 1;
            if used.contains(&x) || used.contains(&y) {
                continue;
            }
            let (a, b) = (x.min(y), x.max(y));
            if self.measured_window[a as usize] && self.measured_window[b as usize] {
                // Both inside the measured window: the measured lane owns it.
                continue;
            }
            if self.linked_idx(a, b) || self.woven_idx(a, b) {
                continue;
            }
            evaluated += 1;
            match self.distance(a, b, DIVERGENT_RADIUS, false) {
                None => {
                    used.insert(a);
                    used.insert(b);
                    far.push((a, b));
                }
                Some((hops, _)) => near.push((hops, scanned, a, b)),
            }
        }
        near.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(&y.1)));
        let mut chosen = far;
        for (_, _, a, b) in near {
            if chosen.len() >= need {
                break;
            }
            if used.contains(&a) || used.contains(&b) {
                continue;
            }
            used.insert(a);
            used.insert(b);
            chosen.push((a, b));
        }
        (chosen, slot, scanned)
    }

    /// One divergent page: the measured lane first (score desc, then ids),
    /// then forced juxtapositions from the spread sampler. `cursor` resumes
    /// a previous page of the same log head and filter.
    pub fn divergent_page(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<DivergentPage, StoreError> {
        let limit = limit.max(1);
        let cursor = match cursor.map(str::trim).filter(|text| !text.is_empty()) {
            Some(text) => self.decode_cursor(text)?,
            None => Cursor {
                measured_offset: 0,
                slot: Some(Slot::START),
            },
        };
        let mut summary = self.summary();
        let (ranked, measured_total, members) =
            self.measured_top(cursor.measured_offset.saturating_add(limit));
        summary.measured_members_evaluated = members;
        let measured: Vec<DivergentEval> = ranked
            .iter()
            .skip(cursor.measured_offset)
            .take(limit)
            .filter_map(|item| self.divergent_eval_idx(item.a, item.b))
            .collect();
        let measured_offset = cursor.measured_offset + measured.len();
        let measured_done = measured_offset as u64 >= measured_total;
        let room = limit - measured.len();
        let mut slot = cursor.slot;
        if measured_done && summary.juxtaposition_eligible_pairs == 0 {
            // No pair belongs to the sampler: the measured lane was the whole
            // schedule, so the cursor ends here instead of paging empty.
            slot = None;
        }
        let mut juxtaposition = Vec::new();
        if measured_done && room > 0 && slot.is_some() {
            let order = self.spread_order();
            let (chosen, next, scanned) = self.sample(&order, slot, room);
            summary.positions_scanned = scanned;
            slot = next;
            juxtaposition = chosen
                .into_iter()
                .filter_map(|(a, b)| self.divergent_eval_idx(a, b))
                .collect();
        }
        let next_cursor = (!measured_done || slot.is_some()).then(|| {
            self.encode_cursor(Cursor {
                measured_offset,
                slot,
            })
        });
        Ok(DivergentPage {
            head_seq: self.head_seq,
            retention_as_of_ms: self.head_clock_ms,
            measured,
            juxtaposition,
            next_cursor,
            summary,
        })
    }
}

impl StrataStore {
    /// A GhostLink snapshot of the recorded graph with the given pool.
    pub fn ghost_snapshot(&self, filter: PoolFilter) -> GhostSnapshot<'_> {
        GhostSnapshot::new(self, filter)
    }
}

#[cfg(test)]
mod schedule_tests {
    use super::*;

    #[test]
    fn schedule_rounds_are_matchings_and_cover_each_pair_once() {
        for n in 0..14usize {
            let mut seen = HashSet::new();
            let mut slot = Slot::START.normalize(n);
            let mut round: Option<(usize, usize)> = None;
            let mut in_round: HashSet<usize> = HashSet::new();
            while let Some(at) = slot {
                if round != Some((at.s, at.p)) {
                    round = Some((at.s, at.p));
                    in_round.clear();
                }
                let (x, y) = (at.i, at.i + at.s);
                assert!(y < n);
                assert!(in_round.insert(x), "slot {x} twice in round {round:?}");
                assert!(in_round.insert(y), "slot {y} twice in round {round:?}");
                assert!(seen.insert((x, y)), "pair {x},{y} repeated");
                slot = at.next(n);
            }
            assert_eq!(seen.len(), n * n.saturating_sub(1) / 2, "n={n}");
        }
    }

    #[test]
    fn year_month_is_civil() {
        assert_eq!(year_month(0), (1970, 1));
        assert_eq!(year_month(1_790_661_141_211), (2026, 9));
        assert_eq!(year_month(951_782_400_000), (2000, 2)); // 2000-02-29
        assert_eq!(year_month(-1), (1969, 12));
    }
}
