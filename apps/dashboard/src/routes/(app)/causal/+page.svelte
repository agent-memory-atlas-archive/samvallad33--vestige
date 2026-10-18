<script lang="ts">
	/**
	 * Causal — from a failure back to its causes, along recorded edges only.
	 *
	 * The river holds the memories under an exact failure tag (recall), by the
	 * minute they were written. Pick one and causal_walk follows the recorded
	 * edges backward from it (derived_from, evidence_of, closes, touched):
	 * the causes rise into the sky by depth and flow into the failure;
	 * forgotten_lesson reads how well each lesson on that path is retained.
	 * "Prove the walk" runs the planted-cause selftest on a throwaway store:
	 * a decoy edge, a forward edge and a keyword distractor must all be
	 * ignored for it to pass.
	 */
	import '$lib/trace/trace-organ.css';
	import { alive } from '$lib/trace/alive';
	import { onMount } from 'svelte';
	import OrganStage from '$lib/trace/OrganStage.svelte';
	import type { StageWords } from '$lib/trace/TraceStage.svelte';
	import { api } from '$lib/stores/api';
	import { osHref } from '$lib/os-nav';
	import { typeColor } from '$lib/trace/trace-pass';
	import { headline, isCausalEdge, normalizeWalk, type TraceMemory, type WalkResult } from '$lib/trace/trace-data';
	import { toTraceMemory } from '$lib/trace/trace-memory';
	import { fetchMemories, recallTag } from '$lib/trace/trace-fetch';
	import MemoryFocus from '$lib/trace/MemoryFocus.svelte';
	import { goto } from '$app/navigation';
	import type { VerifyState } from '$lib/trace/trace-pass';

	const TAGS = ['failure', 'operator-stop', 'fix', 'correction'] as const;

	interface SelftestResult {
		all_passed?: boolean;
		checks?: { name: string; pass: boolean }[];
		checks_passed?: number;
		checks_total?: number;
		hop_bound?: number;
		live_store_touched?: boolean;
		temp_store_deleted?: boolean;
		note?: string;
	}

	let tag = $state<string>('failure');
	let handle = $state('failure');
	let failures = $state<TraceMemory[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let titleEl = $state<HTMLElement | null>(null);

	let picked = $state<TraceMemory | null>(null);
	let walk = $state<WalkResult | null>(null);
	let walkBusy = $state(false);
	let walkError = $state<string | null>(null);
	let retained = $state<Map<string, number>>(new Map());

	let selftest = $state<SelftestResult | null>(null);
	let selftestBusy = $state(false);
	let selftestMs = $state<number | null>(null);
	let selftestError = $state<string | null>(null);

	// ── Hunt a GitHub issue (gather) ────────────────────────────────────────
	const REPO_KEY = 'vestige.healing.repo.v1';
	let huntUrl = $state('');
	let huntRepo = $state('');
	let hunting = $state(false);
	let huntLanded = $state(0);
	let huntResult = $state<Record<string, unknown> | null>(null);
	let huntError = $state<string | null>(null);
	let huntTimer: ReturnType<typeof setInterval> | null = null;
	// The hunt needs the gather tool; a server without it shows no hunt panel.
	let canGather = $state(false);
	onMount(() => {
		api
			.tool('memory_status', { view: 'tools' })
			.then((r) => (canGather = JSON.stringify(r).includes('"gather"')))
			.catch(() => (canGather = false));
	});

	/**
	 * gather records the issue, its comments, linked PRs and the commit
	 * window as receipts, then walks them; on a real repository that takes
	 * minutes. The page does not wait on it: every receipt it records erupts
	 * onto the river as it lands (memory_status changelog, polled).
	 */
	async function hunt() {
		if (!huntUrl.trim() || !huntRepo.trim()) return;
		hunting = true;
		huntLanded = 0;
		huntResult = null;
		huntError = null;
		try {
			localStorage.setItem(REPO_KEY, huntRepo.trim());
		} catch {
			// Storage blocked: the path is asked for again next time.
		}
		const since = Date.now();
		const seen = new Set<string>();
		picked = null;
		walk = null;
		failures = [];
		handle = 'hunt';
		huntTimer = setInterval(async () => {
			const cl = await api
				.tool<{ events?: { timestamp: string; affectedIdsPreview?: string[] }[] }>('memory_status', { view: 'changelog', limit: 60 })
				.catch(() => null);
			const fresh = (cl?.events ?? [])
				.filter((e) => Date.parse(e.timestamp) >= since)
				.flatMap((e) => e.affectedIdsPreview ?? [])
				.filter((id) => !seen.has(id));
			if (!fresh.length) return;
			for (const id of fresh) seen.add(id);
			huntLanded = seen.size;
			const got = await fetchMemories(fresh).catch(() => new Map<string, TraceMemory>());
			failures = [...failures, ...got.values()].sort((a, b) => Date.parse(a.createdAt) - Date.parse(b.createdAt)).slice(-1100);
		}, 2500);
		try {
			huntResult = await api.tool<Record<string, unknown>>('gather', { url: huntUrl.trim(), repoPath: huntRepo.trim(), maxComments: 20 });
		} catch (e) {
			huntError = e instanceof Error ? e.message : String(e);
		} finally {
			if (huntTimer) clearInterval(huntTimer);
			huntTimer = null;
			hunting = false;
		}
	}
	$effect(() => () => {
		if (huntTimer) clearInterval(huntTimer);
	});

	const byId = $derived(new Map(failures.map((m) => [m.id, m])));
	let verify = $state<VerifyState>('idle');
	/** A connected memory: walk from it when it is on this river, else open it in Trace. */
	function focusPick(id: string) {
		const m = byId.get(id);
		if (m) void pick(m);
		else void goto(osHref('/trace', { q: id }));
	}
	$effect(() => {
		if (!picked) verify = 'idle';
	});

	async function load(h = handle) {
		loading = true;
		error = null;
		picked = null;
		walk = null;
		try {
			const id = /^mem-[0-9a-f]{16}$/i.test(h.trim()) || /^[0-9a-f-]{36}$/i.test(h.trim());
			if (id) {
				const m = toTraceMemory((await api.memories.get(h.trim())) as unknown as Record<string, unknown>);
				failures = m ? [m] : [];
				if (m) void pick(m);
			} else {
				failures = await recallTag(h.trim(), 100);
			}
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
			failures = [];
		} finally {
			loading = false;
		}
	}

	async function pick(m: TraceMemory) {
		picked = m;
		walk = null;
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

	async function prove() {
		selftestBusy = true;
		selftestError = null;
		const t0 = performance.now();
		try {
			selftest = await api.tool<SelftestResult>('selftest', {});
			selftestMs = Math.round(performance.now() - t0);
		} catch (e) {
			selftestError = e instanceof Error ? e.message : String(e);
		} finally {
			selftestBusy = false;
		}
	}

	function setTag(t: string) {
		tag = t;
		handle = t;
		void load(t);
	}

	function lastEdge(n: WalkResult['nodes'][number]): string {
		return n.path[n.path.length - 1]?.link_type ?? 'edge';
	}

	const walkByDepth = $derived.by(() => {
		const out = new Map<number, WalkResult['nodes']>();
		for (const n of walk?.nodes ?? []) {
			if (n.depth === 0) continue;
			out.set(n.depth, [...(out.get(n.depth) ?? []), n]);
		}
		return [...out.entries()].sort((a, b) => a[0] - b[0]);
	});
	const edgeCounts = $derived.by(() => {
		const c = new Map<string, number>();
		for (const n of walk?.nodes ?? []) if (n.depth > 0) c.set(lastEdge(n), (c.get(lastEdge(n)) ?? 0) + 1);
		return [...c.entries()].sort((a, b) => b[1] - a[1]);
	});

	function describe(id: string): StageWords | null {
		const w = walk?.nodes.find((n) => n.id === id);
		if (w && w.depth > 0) {
			const pct = retained.get(id);
			return { kicker: `${lastEdge(w)} · depth ${w.depth}${pct !== undefined ? ` · ${Math.round(pct)}% retained` : ''}`, text: w.content, tone: lastEdge(w) };
		}
		const m = byId.get(id);
		return m ? { kicker: `${m.type} · ${when(m.createdAt)}`, text: m.content, tone: m.type } : null;
	}

	const labelIds = $derived(
		picked
			? [picked.id, ...(walk?.nodes ?? []).filter((n) => n.depth > 0 && isCausalEdge(lastEdge(n))).slice(0, 5).map((n) => n.id)]
			: []
	);

	const rgb = (t: string) => {
		const [r, g, b] = typeColor(t);
		return `rgb(${Math.round(r * 255)} ${Math.round(g * 255)} ${Math.round(b * 255)})`;
	};
	function when(iso?: string): string {
		if (!iso) return '';
		const d = new Date(iso);
		return Number.isFinite(d.getTime()) ? d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '';
	}

	onMount(() => {
		try {
			huntRepo = localStorage.getItem(REPO_KEY) ?? '';
		} catch {
			huntRepo = '';
		}
		void load();
	});
</script>

<svelte:head>
	<title>Causal · Vestige</title>
</svelte:head>

<OrganStage
	memories={failures}
	{walk}
	selectedId={picked?.id ?? null}
	{verify}
	origin={() => {
		const r = titleEl?.getBoundingClientRect();
		return r ? { x: ((r.left + 160) / innerWidth) * 2 - 1, y: 1 - ((r.top + r.height / 2) / innerHeight) * 2 } : undefined;
	}}
	{describe}
	{labelIds}
	onpick={(id) => {
		const m = byId.get(id);
		if (m) void pick(m);
		else {
			handle = id;
			void load(id);
		}
	}}
	story="causal"
	seed="vestige-causal-v2"
	axis="order"
/>

<header class="tr-ask" class:tr-quiet={!!picked}>
	<p class="tr-eyebrow" use:alive={{ text: 'Causal', glow: true }}></p>
	<!-- svelte-ignore a11y_missing_content (alive renders the text and its aria-label) -->
	<h1 class="tr-title" bind:this={titleEl} use:alive={{ text: 'From a failure back to its causes.', glow: true }}></h1>
	<form class="tr-hint" onsubmit={(e) => { e.preventDefault(); void load(); }}>
		{#each TAGS as t (t)}
			<button type="button" class="tr-chip" aria-pressed={tag === t && handle === t} onclick={() => setTag(t)}>#{t}</button>
		{/each}
		<input class="tr-field" style="max-width: 16rem" bind:value={handle} placeholder="an exact tag or a memory id" aria-label="Exact tag or memory id" spellcheck="false" />
	</form>
	{#if loading}
		<p class="tr-line">Recalling by exact handle…</p>
	{:else if error}
		<p class="tr-line tr-err">{error}</p>
	{:else if handle === 'hunt'}
		<p class="tr-line">
			<strong use:alive={{ text: String(failures.length), count: true }}></strong> receipts from the hunt on the field{hunting ? ', and more landing' : ''}.
			Pick one and walk its recorded edges.
		</p>
	{:else}
		<p class="tr-line">
			<strong use:alive={{ text: String(failures.length), count: true }}></strong> {failures.length === 1 ? 'memory' : 'memories'} under {handle.startsWith('mem-') ? handle : `#${handle}`}.
			Pick one and the walk follows recorded edges only: no similarity, no keywords.
		</p>
	{/if}
</header>

<aside class="tr-lane" class:idle={!picked && !selftest} aria-live="polite">
	{#if picked}
		<MemoryFocus id={picked.id} onpick={focusPick} onverify={(v) => (verify = v)} onclose={() => (picked = null)}>
			<div class="tr-block">
				<p class="tr-proof-kicker" use:alive={{ text: 'Recorded edges back' }}></p>
				{#if walkBusy}
					<p class="tr-proof-line">Walking the recorded edges…</p>
				{:else if walkError}
					<p class="tr-proof-line tr-err">{walkError}</p>
				{:else if walk}
					{#if walk.nodes.length <= 1}
						<p class="tr-proof-line">{walk.emptyBecause ?? 'No recorded edge leads upstream from this memory.'}</p>
					{:else}
						<p class="tr-proof-line">
							{walk.nodes.length - 1} {walk.nodes.length - 1 === 1 ? 'memory' : 'memories'} upstream ·
							{#each edgeCounts as [e, n], i (e)}{i ? ', ' : ''}<span class:tr-warn={isCausalEdge(e)}>{n} {e}</span>{/each}
						</p>
					{/if}
					{#each walkByDepth.slice(0, 4) as [d, list] (d)}
						<p class="tr-depth">depth {d} · {list.length}</p>
						{#each list.filter((n) => isCausalEdge(lastEdge(n))).concat(list.filter((n) => !isCausalEdge(lastEdge(n)))).slice(0, 6) as n (n.id)}
							{@const pct = retained.get(n.id)}
							<button type="button" class="tr-item"
					data-trace-id={n.id} class:fading={pct !== undefined && pct < 50} onclick={() => { handle = n.id; void load(n.id); }}>
								<span class="tr-edge" class:causal={isCausalEdge(lastEdge(n))}>{lastEdge(n)}</span>
								<span>{headline(n.content, 110)}</span>
								{#if pct !== undefined}<span class="tr-tag">{pct < 50 ? 'fading lesson · ' : ''}{Math.round(pct)}% retained</span>{/if}
							</button>
						{/each}
					{/each}
					{#if walk.note}<p class="tr-note">{walk.note}</p>{/if}
				{/if}
			</div>
		</MemoryFocus>
	{/if}

	{#if canGather}
	<div class="tr-block">
		<p class="tr-proof-kicker" use:alive={{ text: 'Hunt a GitHub issue' }}></p>
		<p class="tr-note">Records the issue, its comments, linked pull requests and the commit window as receipts, then walks them. Each receipt lands on the field as it is written.</p>
		<input class="tr-field" bind:value={huntUrl} placeholder="https://github.com/owner/repo/issues/123" spellcheck="false" />
		<input class="tr-field" bind:value={huntRepo} placeholder="absolute path of a local checkout" spellcheck="false" />
		<div class="tr-actions">
			<button type="button" class="tr-act" disabled={hunting || !huntUrl.trim() || !huntRepo.trim()} onclick={() => void hunt()}>
				{hunting ? `Gathering… ${huntLanded} receipts landed` : 'Gather the evidence'}
			</button>
		</div>
		{#if huntError}<p class="tr-note tr-err">{huntError}</p>{/if}
		{#if huntResult}
			{#each Object.entries(huntResult).filter(([, v]) => typeof v !== 'object' || v === null).slice(0, 10) as [k, v] (k)}
				<p class="tr-note">{k}: {String(v)}</p>
			{/each}
		{/if}
	</div>
	{/if}

	<div class="tr-block">
		<p class="tr-proof-kicker" use:alive={{ text: 'Prove the walk' }}></p>
		{#if selftest}
			<p class="tr-proof-line">
				{#if selftest.all_passed}<span class="tr-ok">{selftest.checks_passed} of {selftest.checks_total} checks passed</span>{:else}<span class="tr-err">{selftest.checks_passed} of {selftest.checks_total} checks passed</span>{/if}
				· {selftestMs} ms · your store {selftest.live_store_touched ? 'was touched' : 'untouched'} · temp store {selftest.temp_store_deleted ? 'deleted' : 'kept'}
			</p>
			{#each selftest.checks ?? [] as c (c.name)}
				<p class="tr-note"><span class={c.pass ? 'tr-ok' : 'tr-err'}>{c.pass ? 'pass' : 'fail'}</span> · {c.name.replaceAll('_', ' ')}</p>
			{/each}
		{:else}
			<p class="tr-note">Plants a cause, an intermediate and a symptom with recorded edges, plus a decoy edge, a forward edge and a keyword distractor, in a throwaway store, then walks back from the symptom.</p>
		{/if}
		{#if selftestError}<p class="tr-note tr-err">{selftestError}</p>{/if}
		<div class="tr-actions">
			<button type="button" class="tr-act alt" disabled={selftestBusy} onclick={() => void prove()}>{selftestBusy ? 'Planting the chain…' : selftest ? 'Run it again' : 'Run the planted-cause selftest'}</button>
		</div>
	</div>
</aside>
