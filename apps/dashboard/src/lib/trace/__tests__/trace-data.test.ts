import { describe, expect, it } from 'vitest';
import { headline, isCausalEdge, normalizeReceipt, normalizeWalk, parseHandle, walkEdges } from '../trace-data';
import { toTraceMemory } from '../trace-memory';

describe('parseHandle', () => {
	const now = new Date(2026, 9, 17, 15, 30);

	it('reads "today" and the question form as the local day', () => {
		for (const q of ['today', 'What happened today?', '  what happened today ']) {
			const p = parseHandle(q, now)!;
			expect(p.kind).toBe('day');
			expect(p.start!.getTime()).toBe(new Date(2026, 9, 17).getTime());
			expect(p.end!.getTime()).toBe(new Date(2026, 9, 18).getTime());
		}
	});

	it('reads yesterday and an ISO date as local days', () => {
		expect(parseHandle('yesterday', now)!.start!.getTime()).toBe(new Date(2026, 9, 16).getTime());
		const iso = parseHandle('2026-10-15', now)!;
		expect(iso.kind).toBe('day');
		expect(iso.start!.getTime()).toBe(new Date(2026, 9, 15).getTime());
	});

	it('reads memory ids and UUIDs as exact ids, anything else as an exact tag', () => {
		expect(parseHandle('mem-00000000000a2543', now)!.kind).toBe('id');
		expect(parseHandle('002528c8-bb1a-4839-b3ea-ce2219e3bea9', now)!.kind).toBe('id');
		const tag = parseHandle('dashboard-v4', now)!;
		expect(tag.kind).toBe('tag');
		expect(tag.raw).toBe('dashboard-v4');
	});

	it('keeps a tag exactly as typed (tags are case-sensitive)', () => {
		expect(parseHandle('Vestige', now)!.raw).toBe('Vestige');
	});

	it('returns null for an empty handle', () => {
		expect(parseHandle('   ', now)).toBeNull();
		expect(parseHandle('?', now)).toBeNull();
	});
});

describe('normalizeWalk', () => {
	const raw = {
		status: 'completed',
		nodes: [
			{ id: 'a', content: 'start', depth: 0, path: [] },
			{ id: 'b', content: 'cause', depth: 1, path: [{ link_type: 'derived_from', source_id: 'a', target_id: 'b' }] },
			{ id: 'c', content: 'joined', depth: 1, path: [{ link_type: 'touched', source_id: 'a', target_id: 'c' }, { source_id: 'x' }] },
			{ content: 'no id' }
		],
		emptyBecause: null
	};

	it('keeps nodes with ids and edges with both ends', () => {
		const w = normalizeWalk(raw);
		expect(w.nodes.map((n) => n.id)).toEqual(['a', 'b', 'c']);
		expect(w.nodes[2].path).toHaveLength(1);
		expect(walkEdges(w).map((e) => e.link_type)).toEqual(['derived_from', 'touched']);
	});

	it('treats recorded causal types as causal and touched as a join', () => {
		expect(isCausalEdge('derived_from')).toBe(true);
		expect(isCausalEdge('closed_by')).toBe(true);
		expect(isCausalEdge('touched')).toBe(false);
	});

	it('survives a missing or malformed result', () => {
		expect(normalizeWalk(null).nodes).toEqual([]);
		expect(normalizeWalk({ nodes: 'nope' }).nodes).toEqual([]);
	});
});

describe('normalizeReceipt', () => {
	it('reads the live chain verification', () => {
		const r = normalizeReceipt({
			receipt: { receipt_id: 'eff-1', mutations: [{ id: 'm1', kind: 'upsert' }] },
			attestation: { status: 'strata_effect', verification: { chainValid: true, framesVerified: 669418, segmentsVerified: 45, gateAllowed: true } }
		});
		expect(r.receiptId).toBe('eff-1');
		expect(r.chainValid).toBe(true);
		expect(r.framesVerified).toBe(669418);
		expect(r.mutations).toHaveLength(1);
	});

	it('reports unknowns as null, never as a pass', () => {
		const r = normalizeReceipt({});
		expect(r.chainValid).toBeNull();
		expect(r.framesVerified).toBeNull();
	});
});

describe('toTraceMemory', () => {
	it('reads the field names every tool uses', () => {
		expect(toTraceMemory({ id: 'a', nodeType: 'fact', content: 'x', createdAt: 't', retentionStrength: 0.4, tags: ['k'] })).toEqual({
			id: 'a',
			type: 'fact',
			content: 'x',
			createdAt: 't',
			retention: 0.4,
			tags: ['k']
		});
		const recalled = toTraceMemory({ id: 'b', type: 'event', content: 'y' })!;
		expect(recalled.type).toBe('event');
		expect(recalled.retention).toBe(0.8);
		expect(toTraceMemory({ content: 'no id' })).toBeNull();
	});
});

describe('headline', () => {
	it('collapses whitespace and trims to length with an ellipsis', () => {
		expect(headline('a\n\n b   c')).toBe('a b c');
		expect(headline('abcdefghij', 5)).toBe('abcd…');
	});
});
