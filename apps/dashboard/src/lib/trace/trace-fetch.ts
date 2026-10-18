/**
 * Every write in a time window, by exact date range.
 *
 * memory_status view 'timeline' answers newest first and at most 200 per
 * call (its totalMemories is the returned count, not the window's size).
 * This pages backward: each next call ends just before the oldest write the
 * last one returned, until a page comes back short or the page budget runs
 * out. `complete` says whether the whole window was read.
 */
import { api } from '$lib/stores/api';
import type { TraceMemory } from './trace-data';
import { toTraceMemory } from './trace-memory';

const PAGE = 200;

interface TimelinePage {
	timeline?: { memories?: Record<string, unknown>[] }[];
	truncated?: boolean;
}

export async function fetchWindow(
	start: Date,
	end: Date,
	maxPages = 6
): Promise<{ memories: TraceMemory[]; complete: boolean }> {
	const seen = new Map<string, TraceMemory>();
	let to = end;
	for (let page = 0; page < maxPages; page++) {
		const res = await api.tool<TimelinePage>('memory_status', {
			view: 'timeline',
			start: start.toISOString(),
			end: to.toISOString(),
			detail_level: 'full',
			limit: PAGE
		});
		const batch = (res.timeline ?? [])
			.flatMap((d) => d.memories ?? [])
			.map(toTraceMemory)
			.filter((m): m is TraceMemory => m !== null);
		for (const m of batch) seen.set(m.id, m);
		if (batch.length < PAGE && !res.truncated) return { memories: sorted(seen), complete: true };
		const oldest = Math.min(...batch.map((m) => Date.parse(m.createdAt)).filter(Number.isFinite));
		if (!Number.isFinite(oldest) || oldest <= start.getTime()) return { memories: sorted(seen), complete: true };
		// Step back to the oldest write returned. The end is inclusive, so a
		// write sharing that millisecond comes back again and is deduped
		// instead of skipped.
		to = new Date(oldest);
	}
	return { memories: sorted(seen), complete: false };
}

function sorted(map: Map<string, TraceMemory>): TraceMemory[] {
	return [...map.values()].sort((a, b) => Date.parse(a.createdAt) - Date.parse(b.createdAt));
}

/** Full memories by exact id, 20 per memory get_batch call (its limit). */
export async function fetchMemories(ids: string[]): Promise<Map<string, TraceMemory>> {
	const chunks: string[][] = [];
	for (let i = 0; i < ids.length; i += 20) chunks.push(ids.slice(i, i + 20));
	const pages = await Promise.all(
		chunks.map((chunk) => api.tool<{ results?: Record<string, unknown>[] }>('memory', { action: 'get_batch', ids: chunk }))
	);
	const out = new Map<string, TraceMemory>();
	for (const r of pages.flatMap((p) => p.results ?? [])) {
		const m = toTraceMemory(r);
		if (m) out.set(m.id, m);
	}
	return out;
}

/**
 * Every memory under an exact tag. recall answers with id, type, content and
 * tags only, so the write time and retention each memory is placed by come
 * from memory get_batch.
 */
export async function recallTag(handle: string, limit = 100): Promise<TraceMemory[]> {
	const res = await api.tool<{ nodes?: Record<string, unknown>[] }>('recall', { handle, detail_level: 'full', limit });
	const found = (res.nodes ?? []).map(toTraceMemory).filter((m): m is TraceMemory => m !== null);
	const full = await fetchMemories(found.map((m) => m.id)).catch(() => new Map<string, TraceMemory>());
	return found
		.map((m) => full.get(m.id) ?? m)
		.sort((a, b) => Date.parse(a.createdAt) - Date.parse(b.createdAt));
}
