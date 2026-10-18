/**
 * Trace — the data layer.
 *
 * "What happened today?" answered the way Vestige answers everything: by an
 * exact handle, never by resemblance. A handle is one of
 *   - a day: `today`, `yesterday`, or an ISO date (YYYY-MM-DD), local time;
 *   - a memory id (`mem-…` or a UUID);
 *   - an exact tag (case-sensitive), as `recall` resolves it.
 * The answer is a set of real memories. Each one can then show its receipt
 * (re-verified against the hash-chained log at call time) and be walked
 * backward along recorded causal edges.
 *
 * Pure data and parsing: no Svelte, DOM or GPU imports.
 */

/** Set once Trace has answered for this browser; the root then opens the Palace. */
export const TRACE_SEEN_KEY = 'vestige.trace.seen.v1';

export type HandleKind = 'day' | 'id' | 'tag';

export interface ParsedHandle {
	kind: HandleKind;
	/** The handle as the user typed it, trimmed. */
	raw: string;
	/** For day: local midnight of that day, and of the next day. */
	start?: Date;
	end?: Date;
	/** A human label for the answer heading. */
	label: string;
}

const MEM_ID = /^mem-[0-9a-f]{16}$/i;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const ISO_DAY = /^(\d{4})-(\d{2})-(\d{2})$/;

function localDay(y: number, m: number, d: number): { start: Date; end: Date } {
	const start = new Date(y, m, d, 0, 0, 0, 0);
	const end = new Date(y, m, d + 1, 0, 0, 0, 0);
	return { start, end };
}

/** Parse what the user typed into an exact handle. `now` is injectable for tests. */
export function parseHandle(input: string, now: Date = new Date()): ParsedHandle | null {
	const raw = input.trim().replace(/\?+$/, '').trim();
	if (!raw) return null;
	const lower = raw.toLowerCase();
	if (lower === 'today' || lower === 'what happened today') {
		const { start, end } = localDay(now.getFullYear(), now.getMonth(), now.getDate());
		return { kind: 'day', raw, start, end, label: 'today' };
	}
	if (lower === 'yesterday' || lower === 'what happened yesterday') {
		const { start, end } = localDay(now.getFullYear(), now.getMonth(), now.getDate() - 1);
		return { kind: 'day', raw, start, end, label: 'yesterday' };
	}
	const iso = ISO_DAY.exec(raw);
	if (iso) {
		const { start, end } = localDay(Number(iso[1]), Number(iso[2]) - 1, Number(iso[3]));
		return { kind: 'day', raw, start, end, label: raw };
	}
	if (MEM_ID.test(raw) || UUID.test(raw)) return { kind: 'id', raw, label: raw };
	return { kind: 'tag', raw, label: `#${raw}` };
}

/** One memory in the answer. */
export interface TraceMemory {
	id: string;
	type: string;
	content: string;
	createdAt: string;
	retention: number;
	tags: string[];
	/** Still to come (an intention's deadline): drawn right of now, breathing. */
	future?: boolean;
	/** Orders the lanes (higher sits higher); by type when absent. */
	lane?: number;
	/** The name of its lane when it is not the memory's type (for example an agent). */
	laneLabel?: string;
}

/** A recorded edge between two memories already in the set (a tension pair, a closure). */
export interface TraceLink {
	source: string;
	target: string;
	type: string;
}

/** A recorded edge on a walk path (source → target, by its own type). */
export interface WalkEdge {
	link_type: string;
	source_id: string;
	target_id: string;
}

export interface WalkNode {
	id: string;
	content: string;
	depth: number;
	path: WalkEdge[];
}

export interface WalkResult {
	status: string;
	nodes: WalkNode[];
	causes: WalkNode[];
	emptyBecause: string | null;
	truncated?: boolean;
	note?: string;
}

/** Edge types that carry cause; `touched` is a shared-handle join and drawn faint. */
export const CAUSAL_EDGES = new Set(['derived_from', 'evidence_of', 'closes', 'closed_by', 'supersedes', 'corrects', 'contradicts']);

export function isCausalEdge(t: string): boolean {
	return CAUSAL_EDGES.has(t);
}

/** Normalize a causal_walk result defensively (fields can be absent). */
export function normalizeWalk(raw: unknown): WalkResult {
	const r = (raw ?? {}) as Record<string, unknown>;
	const nodeOf = (n: unknown): WalkNode | null => {
		const o = (n ?? {}) as Record<string, unknown>;
		if (typeof o.id !== 'string') return null;
		const path = Array.isArray(o.path)
			? (o.path as unknown[])
					.map((e) => e as Record<string, unknown>)
					.filter((e) => typeof e.source_id === 'string' && typeof e.target_id === 'string')
					.map((e) => ({
						link_type: String(e.link_type ?? 'edge'),
						source_id: e.source_id as string,
						target_id: e.target_id as string
					}))
			: [];
		return {
			id: o.id,
			content: typeof o.content === 'string' ? o.content : '',
			depth: typeof o.depth === 'number' ? o.depth : 0,
			path
		};
	};
	const list = (v: unknown) => (Array.isArray(v) ? (v.map(nodeOf).filter(Boolean) as WalkNode[]) : []);
	return {
		status: typeof r.status === 'string' ? r.status : 'unknown',
		nodes: list(r.nodes),
		causes: list(r.causes),
		emptyBecause: typeof r.emptyBecause === 'string' ? r.emptyBecause : null,
		truncated: r.truncated === true,
		note: typeof r.note === 'string' ? r.note : undefined
	};
}

/** Every distinct recorded edge a walk crossed. */
export function walkEdges(walk: WalkResult): WalkEdge[] {
	const seen = new Set<string>();
	const out: WalkEdge[] = [];
	for (const n of walk.nodes) {
		for (const e of n.path) {
			const k = `${e.source_id}|${e.link_type}|${e.target_id}`;
			if (seen.has(k)) continue;
			seen.add(k);
			out.push(e);
		}
	}
	return out;
}

/** What receipt(action='get') proves, flattened for the reading lane. */
export interface ReceiptProof {
	receiptId: string | null;
	chainValid: boolean | null;
	framesVerified: number | null;
	segmentsVerified: number | null;
	gateAllowed: boolean | null;
	payloadDigest: string | null;
	status: string | null;
	claimBoundary: string | null;
	mutations: { id: string; kind: string; note?: string }[];
}

export function normalizeReceipt(raw: unknown): ReceiptProof {
	const r = (raw ?? {}) as Record<string, unknown>;
	const att = (r.attestation ?? {}) as Record<string, unknown>;
	const ver = (att.verification ?? {}) as Record<string, unknown>;
	const rec = (r.receipt ?? {}) as Record<string, unknown>;
	const num = (v: unknown) => (typeof v === 'number' ? v : null);
	const bool = (v: unknown) => (typeof v === 'boolean' ? v : null);
	const str = (v: unknown) => (typeof v === 'string' ? v : null);
	return {
		receiptId: str(rec.receipt_id),
		chainValid: bool(ver.chainValid),
		framesVerified: num(ver.framesVerified),
		segmentsVerified: num(ver.segmentsVerified),
		gateAllowed: bool(ver.gateAllowed),
		payloadDigest: str(ver.payloadDigest),
		status: str(att.status),
		claimBoundary: str(ver.claimBoundary),
		mutations: Array.isArray(rec.mutations)
			? (rec.mutations as Record<string, unknown>[])
					.filter((m) => typeof m.id === 'string')
					.map((m) => ({ id: m.id as string, kind: String(m.kind ?? ''), note: str(m.note) ?? undefined }))
			: []
	};
}

/** Short, readable first line of a memory (its own words, trimmed). */
export function headline(content: string, max = 120): string {
	const line = content.replace(/\s+/g, ' ').trim();
	return line.length > max ? `${line.slice(0, max - 1)}…` : line;
}
