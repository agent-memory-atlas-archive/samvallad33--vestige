<script lang="ts">
	/**
	 * System — the store, as it is right now.
	 *
	 * The words are memory_status (health, retention, coverage); the river
	 * shows whichever memories the last action touched:
	 *   durable      project preview: what a CLAUDE.md projection would carry
	 *                (project write, with a path and a confirm);
	 *   dream        maintain dream: replays recorded edges and reviews
	 *                memories on the FSRS curve, strengthening some;
	 *   brief        session_start: what an agent is handed when it opens;
	 *   collect      maintain gc, dry run: what would be collected;
	 *   tools        memory_status tools: every tool this dashboard calls.
	 */
	import '$lib/trace/trace-organ.css';
	import { alive } from '$lib/trace/alive';
	import { onMount } from 'svelte';
	import OrganStage from '$lib/trace/OrganStage.svelte';
	import type { StageWords } from '$lib/trace/TraceStage.svelte';
	import { api } from '$lib/stores/api';
	import { osHref } from '$lib/os-nav';
	import { typeColor } from '$lib/trace/trace-pass';
	import { headline, type TraceMemory } from '$lib/trace/trace-data';
	import { fetchMemories } from '$lib/trace/trace-fetch';
	import MemoryFocus from '$lib/trace/MemoryFocus.svelte';
	import { goto } from '$app/navigation';
	import type { VerifyState } from '$lib/trace/trace-pass';
	import type { TraceLink } from '$lib/trace/trace-data';

	type Lens = 'durable' | 'dream' | 'brief' | 'collect' | 'tools';
	const LENSES: { key: Lens; label: string }[] = [
		{ key: 'durable', label: 'what lasts' },
		{ key: 'dream', label: 'dream now' },
		{ key: 'brief', label: 'session brief' },
		{ key: 'collect', label: 'collect (dry run)' },
		{ key: 'tools', label: 'every tool' }
	];
	/** Where each tool is live in the dashboard. */
	const TOOL_HOME: Record<string, string> = {
		recall: '/trace',
		memory_status: '/ledger',
		receipt: '/trace',
		causal_walk: '/causal',
		forgotten_lesson: '/causal',
		selftest: '/causal',
		memory: '/trace',
		suppress: '/trace',
		smart_ingest: '/ledger',
		intention: '/ledger',
		vestige_stops: '/guard',
		dedup: '/healing',
		ghostlink: '/healing',
		codebase: '/healing',
		maintain: '/system',
		project: '/system',
		session_start: '/system',
		gather: '/causal'
	};

	interface Health {
		totalMemories?: number;
		averageRetention?: number;
		status?: string;
		dueForReview?: number;
		warnings?: string[];
		newestMemory?: string;
		oldestMemory?: string;
	}

	let lens = $state<Lens>('durable');
	let loading = $state(true);
	let busy = $state(false);
	let error = $state<string | null>(null);
	let titleEl = $state<HTMLElement | null>(null);
	let pickedId = $state<string | null>(null);

	let health = $state<Health | null>(null);
	let retention = $state<{ bucket: string; count: number }[]>([]);
	let below50 = $state<number | null>(null);
	let edges = $state<[string, number][]>([]);

	let river = $state<TraceMemory[]>([]);
	let durableRegion = $state<string | null>(null);
	let projectPath = $state('');
	let projectConfirm = $state(false);
	let projectNote = $state<string | null>(null);
	let dream = $state<{ durationMs?: number; memoriesReplayed?: number; strengthened?: number; edgesConsidered?: number; discovery?: string; reviews: { id: string; rating?: number; retention?: number; stability?: number }[] } | null>(null);
	let brief = $state<string | null>(null);
	let collect = $state<{ candidateCount?: number; processed?: number } | null>(null);
	let tools = $state<{ name: string; description: string }[]>([]);

	const byId = $derived(new Map(river.map((m) => [m.id, m])));
	let focusLinks = $state<TraceLink[]>([]);
	let verify = $state<VerifyState>('idle');
	/** A connected memory: fly to it when it is on this river, else open it in Trace. */
	function focusPick(id: string) {
		if (byId.has(id)) pickedId = id;
		else void goto(osHref('/trace', { q: id }));
	}
	$effect(() => {
		if (!pickedId) {
			focusLinks = [];
			verify = 'idle';
		}
	});
	const picked = $derived(pickedId ? (byId.get(pickedId) ?? null) : null);
	const reviewById = $derived(new Map((dream?.reviews ?? []).map((r) => [r.id, r])));

	async function loadVitals() {
		const [h, r, c] = await Promise.all([
			api.tool<Health>('memory_status', { view: 'health' }),
			api.tool<{ distribution?: { bucket: string; count: number }[]; memoriesBelow50pct?: number }>('memory_status', { view: 'retention' }),
			api.tool<{ edgeCountsByType?: Record<string, number> }>('memory_status', { view: 'coverage' })
		]);
		health = h;
		retention = r.distribution ?? [];
		below50 = r.memoriesBelow50pct ?? null;
		edges = Object.entries(c.edgeCountsByType ?? {}).sort((a, b) => b[1] - a[1]);
	}

	async function run(next: Lens) {
		lens = next;
		pickedId = null;
		busy = true;
		error = null;
		try {
			if (next === 'durable') {
				const res = await api.tool<{ items?: { id: string }[]; region?: string }>('project', { action: 'preview', max_items: 60 });
				durableRegion = res.region ?? null;
				const full = await fetchMemories((res.items ?? []).map((i) => i.id));
				river = [...full.values()];
			} else if (next === 'dream') {
				const res = await api.tool<{
					durationMs?: number;
					memoriesReplayed?: number;
					edgesConsidered?: number;
					stats?: { memories_strengthened?: number };
					discovery?: { detail?: string };
					reviews?: { id: string; rating?: number; retention?: number; stability?: number }[];
				}>('maintain', { action: 'dream' });
				dream = {
					durationMs: res.durationMs,
					memoriesReplayed: res.memoriesReplayed,
					strengthened: res.stats?.memories_strengthened,
					edgesConsidered: res.edgesConsidered,
					discovery: res.discovery?.detail,
					reviews: res.reviews ?? []
				};
				const full = await fetchMemories(dream.reviews.map((r) => r.id));
				// The dream's verdict is the row: how each memory came back when it
				// was replayed on its FSRS curve (4 easy, 3 good, 2 hard, 1 again).
				const RATED: Record<number, string> = { 4: 'came back easily', 3: 'came back', 2: 'came back with effort', 1: 'slipped' };
				const rating = new Map(dream.reviews.map((r) => [r.id, r.rating]));
				river = [...full.values()].map((m) => {
					const r = rating.get(m.id);
					return r && RATED[r] ? { ...m, laneLabel: RATED[r], lane: r } : m;
				});
			} else if (next === 'brief') {
				const res = await api.tool<{ context?: string; codeContext?: { items?: { id: string }[] } }>('session_start', {
					include_intentions: true,
					include_status: true
				});
				brief = res.context ?? null;
				const ids = [...(brief ?? '').matchAll(/\[(mem-[0-9a-f]{16}|[0-9a-f]{8}-[0-9a-f-]{27})\]/g)].map((m) => m[1]);
				const full = ids.length ? await fetchMemories(ids) : new Map();
				river = [...full.values()];
			} else if (next === 'collect') {
				collect = await api.tool<{ candidateCount?: number; processed?: number }>('maintain', { action: 'gc', dry_run: true });
			} else {
				const res = await api.tool<{ tools?: { name: string; description: string }[] }>('memory_status', { view: 'tools' });
				tools = (res.tools ?? []).map((t) => ({ name: t.name, description: t.description }));
			}
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			busy = false;
		}
	}

	async function writeProjection() {
		if (!projectPath.trim()) return;
		if (!projectConfirm) {
			projectConfirm = true;
			return;
		}
		try {
			const res = await api.tool<{ path?: string; receiptId?: string; itemCount?: number }>('project', {
				action: 'write',
				path: projectPath.trim(),
				confirm: true,
				max_items: 60
			});
			projectNote = `Wrote ${res.itemCount ?? 'the'} memories into ${res.path ?? projectPath} · only the fenced region changed`;
		} catch (e) {
			projectNote = e instanceof Error ? e.message : String(e);
		} finally {
			projectConfirm = false;
		}
	}

	function describe(id: string): StageWords | null {
		const m = byId.get(id);
		if (!m) return null;
		const r = reviewById.get(id);
		return {
			kicker: `${m.type}${r ? ` · reviewed, rating ${r.rating}, stability ${Math.round(r.stability ?? 0)} d` : ''} · ${Math.round(m.retention * 100)}% retained`,
			text: m.content,
			tone: m.type
		};
	}

	const rgb = (t: string) => {
		const [r, g, b] = typeColor(t);
		return `rgb(${Math.round(r * 255)} ${Math.round(g * 255)} ${Math.round(b * 255)})`;
	};
	function when(iso?: string): string {
		if (!iso) return '';
		const d = new Date(iso);
		return Number.isFinite(d.getTime()) ? d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '';
	}

	onMount(async () => {
		try {
			await loadVitals();
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		}
		loading = false;
		await run('durable');
	});
</script>

<svelte:head>
	<title>System · Vestige</title>
</svelte:head>

<OrganStage
	memories={river}
	selectedId={pickedId}
	links={focusLinks}
	{verify}
	origin={() => {
		const r = titleEl?.getBoundingClientRect();
		return r ? { x: ((r.left + 160) / innerWidth) * 2 - 1, y: 1 - ((r.top + r.height / 2) / innerHeight) * 2 } : undefined;
	}}
	{describe}
	labelIds={pickedId ? [pickedId] : []}
	onpick={(id) => (pickedId = id)}
	story="system"
	seed="vestige-system-v1"
	axis="order"
/>

<header class="tr-ask" class:tr-quiet={!!pickedId}>
	<p class="tr-eyebrow" use:alive={{ text: 'System', glow: true }}></p>
	<!-- svelte-ignore a11y_missing_content (alive renders the text and its aria-label) -->
	<h1 class="tr-title" bind:this={titleEl} use:alive={{ text: 'The store, as it is right now.', glow: true }}></h1>
	<p class="tr-hint">
		{#each LENSES as l (l.key)}
			<button type="button" class="tr-chip" aria-pressed={lens === l.key} disabled={busy} onclick={() => void run(l.key)}>{l.label}</button>
		{/each}
	</p>
	{#if loading}
		<p class="tr-line">Reading the store's health…</p>
	{:else if health}
		<p class="tr-line">
			<strong use:alive={{ text: String(health.totalMemories?.toLocaleString()), count: true }}></strong> memories, {health.status ?? 'status unknown'},
			average retention {Math.round((health.averageRetention ?? 0) * 100)}%{below50 ? `, ${below50.toLocaleString()} below half` : ''}.
		</p>
		<p class="tr-types">
			{#each retention as b (b.bucket)}<span>{b.bucket} {b.count.toLocaleString()}</span>{/each}
		</p>
		<p class="tr-types">
			{#each edges.slice(0, 7) as [k, n] (k)}<span>{k.replace('_', ' ')} {n.toLocaleString()}</span>{/each}
		</p>
		{#if health.warnings?.length}<p class="tr-nudge tr-warn">{health.warnings[0]}</p>{/if}
	{/if}
	{#if error}<p class="tr-line tr-err">{error}</p>{/if}
</header>

<aside class="tr-lane" class:idle={!pickedId} aria-live="polite">
	{#if pickedId && picked}
		<MemoryFocus
			id={pickedId}
			onpick={focusPick}
			onlinks={(l) => (focusLinks = l)}
			onverify={(v) => (verify = v)}
			onclose={() => (pickedId = null)}
		>
			{#if reviewById.get(pickedId)}
				{@const r = reviewById.get(pickedId)!}
				<p class="tr-note">The dream just reviewed it: rating {r.rating}, stability {Math.round(r.stability ?? 0)} days.</p>
			{/if}
		</MemoryFocus>
	{:else if busy}
		<p class="tr-proof-line">Running…</p>
	{:else if lens === 'durable'}
		<p class="tr-proof-kicker" use:alive={{ text: 'What a CLAUDE.md projection would carry' }}></p>
		<p class="tr-note">{river.length} durable memories. Each line in the file names its memory id; only the fenced region is ever replaced.</p>
		{#if durableRegion}<pre class="tr-note" style="white-space: pre-wrap; max-height: 16rem; overflow: auto">{durableRegion.slice(0, 2400)}</pre>{/if}
		<input class="tr-field" bind:value={projectPath} placeholder="absolute path of a CLAUDE.md or MEMORY.md" spellcheck="false" />
		<div class="tr-actions">
			<button type="button" class="tr-act" disabled={!projectPath.trim()} onclick={() => void writeProjection()}>
				{projectConfirm ? 'Replace the fenced region in that file?' : 'Write the projection'}
			</button>
		</div>
		{#if projectNote}<p class="tr-note">{projectNote}</p>{/if}
	{:else if lens === 'dream' && dream}
		<p class="tr-proof-kicker" use:alive={{ text: 'The dream, just now' }}></p>
		<p class="tr-proof-line">
			{dream.memoriesReplayed ?? 0} memories replayed over {dream.edgesConsidered?.toLocaleString() ?? '?'} recorded edges,
			{dream.strengthened ?? 0} strengthened, in {((dream.durationMs ?? 0) / 1000).toFixed(1)} s.
		</p>
		{#if dream.discovery}<p class="tr-note">{dream.discovery}</p>{/if}
		{#each dream.reviews.slice(0, 10) as r (r.id)}
			<button type="button" class="tr-item"
				data-trace-id={r.id} onclick={() => (pickedId = r.id)}>
				<span class="tr-edge causal">rating {r.rating} · {Math.round(r.stability ?? 0)} d</span>
				<span>{headline(byId.get(r.id)?.content ?? r.id, 100)}</span>
			</button>
		{/each}
	{:else if lens === 'brief' && brief}
		<p class="tr-proof-kicker" use:alive={{ text: 'What an agent is handed when it opens' }}></p>
		<pre class="tr-note" style="white-space: pre-wrap">{brief}</pre>
	{:else if lens === 'collect' && collect}
		<p class="tr-proof-kicker" use:alive={{ text: 'Garbage collection, dry run' }}></p>
		<p class="tr-proof-line">
			{collect.candidateCount ?? 0} memories would be collected{collect.processed ? ` of ${collect.processed} examined` : ''}.
			Deletion is refused on Strata in 4.0; nothing was removed.
		</p>
	{:else if lens === 'tools'}
		<p class="tr-proof-kicker" use:alive={{ text: 'Every tool, and where it is live here' }}></p>
		{#each tools as t (t.name)}
			<a class="tr-item" href={osHref(TOOL_HOME[t.name] ?? '/system')}>
				<span class="tr-edge causal">{t.name} · {TOOL_HOME[t.name]?.slice(1) ?? 'system'}</span>
				<span>{headline(t.description, 120)}</span>
			</a>
		{/each}
	{/if}
</aside>
