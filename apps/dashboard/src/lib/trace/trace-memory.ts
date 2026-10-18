/**
 * One memory from any tool result, as the Trace field draws it.
 *
 * Tools name the same fields differently (nodeType or type, createdAt or
 * created_at, retentionStrength or retention); this reads all of them and
 * returns null for anything without an id.
 */
import type { TraceMemory } from './trace-data';

export function toTraceMemory(raw: Record<string, unknown> | null | undefined): TraceMemory | null {
	if (!raw || typeof raw.id !== 'string') return null;
	const num = (v: unknown, fallback: number) => (typeof v === 'number' && Number.isFinite(v) ? v : fallback);
	return {
		id: raw.id,
		type: String(raw.nodeType ?? raw.node_type ?? raw.type ?? 'memory'),
		content: String(raw.content ?? raw.content_preview ?? raw.label ?? ''),
		createdAt: String(raw.createdAt ?? raw.created_at ?? raw.recorded_at ?? ''),
		retention: num(raw.retentionStrength ?? raw.retention_strength ?? raw.retention, 0.8),
		tags: Array.isArray(raw.tags) ? (raw.tags as unknown[]).filter((t): t is string => typeof t === 'string') : []
	};
}
