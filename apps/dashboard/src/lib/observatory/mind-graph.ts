/**
 * The Field's default view: a cross-section of the whole mind.
 *
 * The graph endpoint returns the neighbourhood around ONE centre memory. On a
 * store where one kind of memory dominates the links (hundreds of commit
 * events wired to each other), the most-connected neighbourhood is that kind
 * and nothing else, so the field shows one type in one colour. Instead, every
 * recorded type gets its own share of the field: the neighbourhood around its
 * newest memory (so its recorded edges come along), topped up with its newest
 * memories. Every node and edge comes from the store; members are chosen by
 * exact type and recency, never by similarity.
 */
import { api } from '$lib/stores/api';
import type { GraphNode, GraphResponse, Memory } from '$types';

const TYPES = ['decision', 'correction', 'fact', 'pattern', 'note', 'event'] as const;
const PER_TYPE = 36;

function asNode(m: Memory): GraphNode {
	return {
		id: m.id,
		label: (m.content ?? '').slice(0, 160),
		type: m.nodeType,
		retention: m.retentionStrength,
		tags: m.tags ?? [],
		createdAt: m.createdAt,
		updatedAt: m.updatedAt,
		isCenter: false,
		lastAccessed: m.lastAccessedAt
	};
}

/**
 * The Field page and its stage both ask for the mind as they boot: callers
 * within a few seconds of each other share one load instead of each fetching
 * twelve requests' worth.
 */
let inFlight: Promise<GraphResponse> | null = null;
const SHARE_MS = 4000;
export function loadMindGraph(): Promise<GraphResponse> {
	if (inFlight) return inFlight;
	const p = buildMindGraph();
	inFlight = p;
	const release = () => setTimeout(() => inFlight === p && (inFlight = null), SHARE_MS);
	p.then(release, () => (inFlight = null));
	return p;
}

async function buildMindGraph(): Promise<GraphResponse> {
	const lists = await Promise.all(
		TYPES.map((node_type) =>
			api.memories
				.list({ node_type, limit: String(PER_TYPE), sort: 'newest' })
				.then((r) => r.memories ?? [])
				.catch(() => [] as Memory[])
		)
	);
	const centres = lists.map((l) => l[0]?.id).filter((id): id is string => !!id);
	if (centres.length < 2) return api.graph({ max_nodes: 200, depth: 3, sort: 'connected' });

	const graphs = (
		await Promise.all(
			centres.map((center_id) => api.graph({ center_id, max_nodes: PER_TYPE, depth: 2 }).catch(() => null))
		)
	).filter((g): g is GraphResponse => !!g);

	// Each type's share: its neighbourhood first (it brings recorded edges),
	// then its newest memories, up to PER_TYPE.
	const nodes = new Map<string, GraphNode>();
	TYPES.forEach((type, t) => {
		const centre = lists[t][0]?.id;
		const hood = graphs.find((g) => g.center_id === centre)?.nodes ?? [];
		let taken = 0;
		for (const n of hood) {
			if (taken >= PER_TYPE) break;
			if (!nodes.has(n.id)) {
				nodes.set(n.id, { ...n, isCenter: false });
				taken++;
			}
		}
		for (const m of lists[t]) {
			if (taken >= PER_TYPE) break;
			if (!nodes.has(m.id)) {
				nodes.set(m.id, asNode(m));
				taken++;
			}
		}
	});

	const centre = centres[0];
	const centreNode = nodes.get(centre);
	if (centreNode) nodes.set(centre, { ...centreNode, isCenter: true });
	const edgeMap = new Map<string, GraphResponse['edges'][number]>();
	for (const g of graphs) {
		for (const e of g.edges) {
			if (nodes.has(e.source) && nodes.has(e.target)) edgeMap.set(`${e.source}:${e.target}:${e.type}`, e);
		}
	}
	const edges = [...edgeMap.values()];
	const base = graphs[0] ?? (await api.graph({ max_nodes: 1, depth: 1 }));
	return {
		...base,
		nodes: [...nodes.values()],
		edges,
		center_id: centre,
		nodeCount: nodes.size,
		edgeCount: edges.length
	};
}
