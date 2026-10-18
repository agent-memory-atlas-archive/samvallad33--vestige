<script lang="ts">
	/**
	 * Witness — what shaped the decision.
	 *
	 * Every decision stands on a straight lane for its topic (one of its exact
	 * tags), at the minute it was written. Pick one and its witnesses rise above
	 * it in rows, each in its edge's colour: causal_walk follows its recorded
	 * edges back (derived_from, evidence_of, closes, corrects, and one touched
	 * hop), and its direct recorded connections in both directions join them
	 * (most decisions on a live store are linked forward, by what came of
	 * them, more often than back). The reading lane lists them while
	 * MemoryFocus re-verifies the decision's own receipt. A second click on the
	 * decision, or a click on any witness, goes inside it.
	 *
	 * Found by exact handle only: the node type, exact tags and memory ids.
	 * Topics are exact tags the decisions carry, never words read out of them.
	 */
	import '$lib/trace/trace-organ.css';
	import { alive } from '$lib/trace/alive';
	import { onMount } from 'svelte';
	import { page } from '$app/stores';
	import OrganStage from '$lib/trace/OrganStage.svelte';
	import type { StageWords } from '$lib/trace/TraceStage.svelte';
	import MemoryFocus from '$lib/trace/MemoryFocus.svelte';
	import { api } from '$lib/stores/api';
	import { enterMemory } from '$lib/inside/enter';
	import { headline, isCausalEdge, normalizeWalk, type TraceMemory, type WalkResult } from '$lib/trace/trace-data';
	import { toTraceMemory } from '$lib/trace/trace-memory';
	import { recallTag } from '$lib/trace/trace-fetch';
	import type { VerifyState } from '$lib/trace/trace-pass';

	/** What an exact tag brings onto the lanes: the memories that decide, and the ones that correct them. */
	const KINDS = new Set(['decision', 'correction']);
	/** Tags too broad to name a topic: on most of the store, or the kind itself. */
	const GENERIC = new Set(['vestige', 'decision', 'correction', 'verified', 'project:vestige', 'codebase:vestige', 'codebase']);
	const MAX_TOPICS = 6;
	/** Witnesses reached only over a shared name (touched): a few are kept, the recorded lineage always is. */
	const MAX_TOUCHED = 10;
	const isId = (h: string) => /^mem-[0-9a-f]{16}$/i.test(h) || /^[0-9a-f-]{36}$/i.test(h);

	let handle = $state('');
	let decisions = $state<TraceMemory[]>([]);
	let total = $state<number | null>(null);
	let topic = $state<string | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let titleEl = $state<HTMLElement | null>(null);

	let picked = $state<TraceMemory | null>(null);
	let walk = $state<WalkResult | null>(null);
	let walkBusy = $state(false);
	let walkError = $state<string | null>(null);
	let retained = $state<Map<string, number>>(new Map());
	let verify = $state<VerifyState>('idle');
	/** The picked decision's direct recorded connections (MemoryFocus reads them from ghostlink map). */
	let neighbours = $state<{ id: string; label: string; type: string; edge: string; outgoing: boolean }[]>([]);

	async function load(h = handle) {
		loading = true;
		error = null;
		picked = null;
		walk = null;
		topic = null;
		const hh = h.trim().replace(/^#/, '');
		try {
			if (!hh) {
				const r = await api.memories.list({ node_type: 'decision', limit: '200', sort: 'newest' });
				total = r.total ?? null;
				decisions = (r.memories ?? [])
					.map((m) => toTraceMemory(m as unknown as Record<string, unknown>))
					.filter((m): m is TraceMemory => m !== null);
			} else if (isId(hh)) {
				const m = toTraceMemory((await api.memories.get(hh)) as unknown as Record<string, unknown>);
				total = null;
				decisions = m ? [m] : [];
				if (m) void pick(m);
			} else {
				total = null;
				decisions = (await recallTag(hh, 100)).filter((m) => KINDS.has(m.type));
			}
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
			decisions = [];
		} finally {
			loading = false;
		}
	}

	/** Topics: the exact tags most decisions here share, beyond the broad ones and the one asked for. */
	const topics = $derived.by(() => {
		const skip = new Set([...GENERIC, handle.trim().replace(/^#/, '')]);
		const count = new Map<string, number>();
		for (const m of decisions) for (const t of new Set(m.tags)) if (!skip.has(t)) count.set(t, (count.get(t) ?? 0) + 1);
		return [...count.entries()]
			.filter(([, n]) => n >= 2)
			.sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
			.slice(0, MAX_TOPICS);
	});

	/** The lanes: each decision on its biggest topic, the rest together at the bottom. */
	const river = $derived.by(() => {
		const rank = new Map(topics.map(([t], i) => [t, i]));
		return decisions
			.filter((m) => !topic || m.tags.includes(topic))
			.map((m) => {
				let best: string | null = null;
				for (const t of m.tags) {
					const r = rank.get(t);
					if (r !== undefined && (best === null || r < rank.get(best)!)) best = t;
				}
				return { ...m, laneLabel: best ? `#${best}` : 'other topics', lane: best ? MAX_TOPICS - rank.get(best)! : -1 };
			})
			.sort((a, b) => Date.parse(a.createdAt) - Date.parse(b.createdAt));
	});
	const byId = $derived(new Map(river.map((m) => [m.id, m])));

	async function pick(m: TraceMemory) {
		picked = m;
		walk = null;
		neighbours = [];
		walkError = null;
		retained = new Map();
		walkBusy = true;
		try {
			const [raw, lessons] = await Promise.all([
				api.tool('causal_walk', { start_points: [{ kind: 'logged_write', node_id: m.id }], promote: false }),
				api
					.tool<{ forgotten_lessons?: { lesson_id?: string; retention_pct?: number }[] }>('forgotten_lesson', { failure_id: m.id })
					.catch(() => null)
			]);
			if (picked?.id !== m.id) return;
			walk = normalizeWalk(raw);
			const next = new Map<string, number>();
			for (const l of lessons?.forgotten_lessons ?? []) {
				if (typeof l.lesson_id === 'string' && typeof l.retention_pct === 'number') next.set(l.lesson_id, l.retention_pct);
			}
			retained = next;
		} catch (e) {
			walkError = e instanceof Error ? e.message : String(e);
		} finally {
			walkBusy = false;
		}
	}

	function lastEdge(n: WalkResult['nodes'][number]): string {
		return n.path[n.path.length - 1]?.link_type ?? 'edge';
	}

	/** How a direct connection relates to the decision, in its own direction. */
	const RELATION: Record<string, [string, string]> = {
		derived_from: ['it came from this', 'came from it'],
		evidence_of: ['it is evidence for this', 'evidence for it'],
		closes: ['it closed this', 'closed it'],
		closed_by: ['closed it', 'it closed this'],
		supersedes: ['it replaced this', 'replaced it'],
		corrects: ['it corrected this', 'corrected it'],
		contradicts: ['it contradicts this', 'contradicts it'],
		touched: ['names the same thing', 'names the same thing']
	};
	const relation = (edge: string, outgoing: boolean) => (RELATION[edge] ?? [edge, edge])[outgoing ? 0 : 1];

	/**
	 * What the field raises above the decision: the walk back, then its direct
	 * recorded connections the walk did not reach, as one set of witnesses. The
	 * recorded lineage is always shown; of the memories that merely share a
	 * name with it, only the nearest few.
	 */
	const shownWalk = $derived.by<WalkResult | null>(() => {
		if (!picked || (!walk && !neighbours.length)) return null;
		const at = picked;
		const base: WalkResult = walk ?? { status: 'map', nodes: [{ id: at.id, content: at.content, depth: 0, path: [] }], causes: [], emptyBecause: null };
		const have = new Set(base.nodes.map((n) => n.id));
		const direct = neighbours
			.filter((n) => !have.has(n.id))
			.map((n) => ({
				id: n.id,
				content: n.label,
				depth: 1,
				path: [{ link_type: n.edge, source_id: n.outgoing ? at.id : n.id, target_id: n.outgoing ? n.id : at.id }]
			}));
		let touched = 0;
		const nodes = [...base.nodes, ...direct].filter((n) => {
			if (n.depth === 0 || isCausalEdge(lastEdge(n))) return true;
			return touched++ < MAX_TOUCHED;
		});
		return { ...base, nodes };
	});
	const directIds = $derived(new Set(neighbours.map((n) => n.id)));
	const outgoingOf = $derived(new Map(neighbours.map((n) => [n.id, n.outgoing])));
	const witnesses = $derived((shownWalk?.nodes ?? []).filter((n) => n.depth > 0));
	// The lane lists what the walk reached; MemoryFocus lists the direct connections under it.
	const walkIds = $derived(new Set((walk?.nodes ?? []).filter((n) => n.depth > 0).map((n) => n.id)));
	const walked = $derived(witnesses.filter((n) => walkIds.has(n.id)));
	const lineage = $derived(walked.filter((n) => isCausalEdge(lastEdge(n))));
	const sharedName = $derived(walked.filter((n) => !isCausalEdge(lastEdge(n))));
	const upstream = $derived((walk?.nodes ?? []).filter((n) => n.depth > 0).length);
	const sharedTotal = $derived.by(() => {
		const ids = new Set<string>();
		for (const n of walk?.nodes ?? []) if (n.depth > 0 && !isCausalEdge(lastEdge(n))) ids.add(n.id);
		return ids.size;
	});
	const edgeCounts = $derived.by(() => {
		const c = new Map<string, number>();
		for (const n of witnesses) c.set(lastEdge(n), (c.get(lastEdge(n)) ?? 0) + 1);
		return [...c.entries()].sort((a, b) => b[1] - a[1]);
	});
	/** A witness's words: its relation to the decision, and how far back it sits. */
	function relationOf(n: WalkResult['nodes'][number]): string {
		const out = outgoingOf.get(n.id);
		return out !== undefined && n.depth === 1 && directIds.has(n.id) ? relation(lastEdge(n), out) : `${lastEdge(n)} · depth ${n.depth}`;
	}

	function describe(id: string): StageWords | null {
		const w = shownWalk?.nodes.find((n) => n.id === id);
		if (w && w.depth > 0) {
			const pct = retained.get(id);
			return { kicker: `${relationOf(w)}${pct !== undefined ? ` · ${Math.round(pct)}% retained` : ''}`, text: w.content, tone: lastEdge(w) };
		}
		const m = byId.get(id);
		return m ? { kicker: `${m.type} · ${m.laneLabel} · ${when(m.createdAt)}`, text: m.content, tone: m.type } : null;
	}

	/** A connected memory: pick it when it is on these lanes, else go inside it. */
	function focusPick(id: string) {
		const m = byId.get(id);
		if (m) void pick(m);
		else void enterMemory(id);
	}
	$effect(() => {
		if (!picked) verify = 'idle';
	});

	function setTopic(t: string | null) {
		topic = t;
		picked = null;
		walk = null;
	}

	function when(iso?: string): string {
		if (!iso) return '';
		const d = new Date(iso);
		return Number.isFinite(d.getTime()) ? d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '';
	}

	onMount(() => {
		// ?id= a memory, or ?tag= an exact tag, from a link elsewhere.
		const q = $page.url.searchParams;
		handle = q.get('id') ?? q.get('tag') ?? '';
		void load();
	});
</script>

<svelte:head>
	<title>Witness · Vestige</title>
</svelte:head>

<OrganStage
	memories={river}
	walk={shownWalk}
	selectedId={picked?.id ?? null}
	{verify}
	origin={() => {
		const r = titleEl?.getBoundingClientRect();
		return r ? { x: ((r.left + 160) / innerWidth) * 2 - 1, y: 1 - ((r.top + r.height / 2) / innerHeight) * 2 } : undefined;
	}}
	{describe}
	onpick={(id) => {
		const m = byId.get(id);
		if (m) void pick(m);
	}}
	story="witness"
	seed="vestige-witness-v1"
	enter="second"
/>

<header class="tr-ask" class:tr-quiet={!!picked}>
	<p class="tr-eyebrow" use:alive={{ text: 'Witness', glow: true }}></p>
	<!-- svelte-ignore a11y_missing_content (alive renders the text and its aria-label) -->
	<h1 class="tr-title" bind:this={titleEl} use:alive={{ text: 'What shaped the decision?', glow: true }}></h1>
	<form class="tr-hint" onsubmit={(e) => { e.preventDefault(); void load(); }}>
		<button type="button" class="tr-chip" aria-pressed={!topic} onclick={() => setTopic(null)}>every topic</button>
		{#each topics.slice(0, 4) as [t] (t)}
			<button type="button" class="tr-chip" aria-pressed={topic === t} onclick={() => setTopic(topic === t ? null : t)}>#{t}</button>
		{/each}
		<input class="tr-field" style="max-width: 15rem" bind:value={handle} placeholder="an exact tag or a memory id" aria-label="Exact tag or memory id" spellcheck="false" />
	</form>
	{#if loading}
		<p class="tr-line">Reading the decisions by exact handle…</p>
	{:else if error}
		<p class="tr-line tr-err">{error}</p>
	{:else}
		<p class="tr-line">
			<strong use:alive={{ text: String(river.length), count: true }}></strong>
			{river.length === 1 ? 'decision' : 'decisions'}{handle.trim() ? ` under ${isId(handle.trim()) ? handle.trim() : `#${handle.trim().replace(/^#/, '')}`}` : total ? `, the newest of ${total.toLocaleString()}` : ''}{topic ? ` on #${topic}` : ''},
			each on the world of its topic. Pick one to see what shaped it; pick it again to go inside.
		</p>
	{/if}
</header>

<aside class="tr-lane" class:idle={!picked} aria-live="polite">
	{#if picked}
		<MemoryFocus
			id={picked.id}
			onpick={focusPick}
			onverify={(v) => (verify = v)}
			onneighbours={(list) => (neighbours = list)}
			onclose={() => (picked = null)}
			proof="manual"
		>
			<div class="tr-block">
				<p class="tr-proof-kicker" use:alive={{ text: 'What shaped it' }}></p>
				{#if walkBusy}
					<p class="tr-proof-line">Walking its recorded edges back…</p>
				{:else if walkError}
					<p class="tr-proof-line tr-err">{walkError}</p>
				{:else if walk}
					{#if !witnesses.length}
						<p class="tr-proof-line">No recorded edge reaches this decision yet, back or forward. Its receipt still holds what wrote it.</p>
					{:else}
						<p class="tr-proof-line">
							{witnesses.length} {witnesses.length === 1 ? 'witness' : 'witnesses'} above it ·
							{#each edgeCounts as [e, n], i (e)}{i ? ', ' : ''}<span class:tr-warn={isCausalEdge(e)}>{n} {e}</span>{/each}
						</p>
						<p class="tr-note">
							{upstream ? `${upstream} reached walking its recorded edges back.` : 'Nothing recorded leads back from it.'}
							{witnesses.length > walked.length ? 'Its direct connections, in both directions, are listed under its proof.' : ''}
						</p>
					{/if}
					{#if lineage.length}
						<p class="tr-depth">recorded lineage · {lineage.length}</p>
						{#each lineage.slice(0, 12) as n (n.id)}
							{@const pct = retained.get(n.id)}
							<button type="button" class="tr-item" data-trace-id={n.id} class:fading={pct !== undefined && pct < 50} onclick={() => void enterMemory(n.id)}>
								<span class="tr-edge causal">{relationOf(n)}</span>
								<span>{headline(n.content, 110)}</span>
								{#if pct !== undefined}<span class="tr-tag">{pct < 50 ? 'fading · ' : ''}{Math.round(pct)}% retained</span>{/if}
							</button>
						{/each}
					{/if}
					{#if sharedName.length}
						<p class="tr-depth">names the same thing · {sharedName.length}{sharedTotal > sharedName.length ? ` of ${sharedTotal}` : ''}</p>
						{#each sharedName as n (n.id)}
							<button type="button" class="tr-item" data-trace-id={n.id} onclick={() => void enterMemory(n.id)}>
								<span class="tr-edge">{relationOf(n)}</span>
								<span>{headline(n.content, 110)}</span>
							</button>
						{/each}
					{/if}
					{#if walk.note}<p class="tr-note">{walk.note}</p>{/if}
				{/if}
				<div class="tr-actions">
					<button type="button" class="tr-act" onclick={() => picked && void enterMemory(picked.id)}>Go inside this decision</button>
				</div>
			</div>
		</MemoryFocus>
	{:else if !loading}
		<div class="tr-block">
			<p class="tr-proof-kicker" use:alive={{ text: 'How to read it' }}></p>
			<p class="tr-note">Each world is a topic: an exact tag the decisions carry, its newest decisions at its heart. Diamonds are decisions and crosses are the corrections that changed them.</p>
			<p class="tr-note">Pick one and the walk follows its recorded edges back: what it was derived from, the evidence for it, what it closed and what corrected it. They ring it, the nearest causes closest, and each one has a signed receipt.</p>
			<p class="tr-note">No similarity and no keywords: only edges the log recorded.</p>
		</div>
	{/if}
</aside>
