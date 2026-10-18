<script lang="ts">
	/**
	 * Healing — where memory corrected itself, and where it still could.
	 *
	 * Four lenses over the same river, each a live tool:
	 *   tension        recorded corrects / supersedes / closed_by / contradicts
	 *                  edges (GET /api/contradictions), arching from the older
	 *                  memory to the one that healed it;
	 *   code drift     codebase get_context: code memories whose anchors still
	 *                  match the source, drifted (stale) or were never anchored,
	 *                  with codebase verify to re-check them now;
	 *   never composed ghostlink propose: pairs the record joins by a typed
	 *                  path but no one has ever composed, each with its proof,
	 *                  and ghostlink weave to record what a pair turned out to be;
	 *   duplicates     dedup scan: exact duplicates only (no similarity).
	 */
	import '$lib/trace/trace-organ.css';
	import { alive } from '$lib/trace/alive';
	import { onMount } from 'svelte';
	import OrganStage from '$lib/trace/OrganStage.svelte';
	import type { StageWords } from '$lib/trace/TraceStage.svelte';
	import { api } from '$lib/stores/api';
	import { osHref } from '$lib/os-nav';
	import { edgeColor, typeColor } from '$lib/trace/trace-pass';
	import { headline, type TraceLink, type TraceMemory } from '$lib/trace/trace-data';
	import { fetchMemories } from '$lib/trace/trace-fetch';
	import MemoryFocus from '$lib/trace/MemoryFocus.svelte';
	import { goto } from '$app/navigation';
	import type { VerifyState } from '$lib/trace/trace-pass';

	type Lens = 'tension' | 'drift' | 'ghost' | 'dupes';
	const LENSES: { key: Lens; label: string }[] = [
		{ key: 'tension', label: 'recorded tension' },
		{ key: 'drift', label: 'code drift' },
		{ key: 'ghost', label: 'never composed' },
		{ key: 'dupes', label: 'duplicates' }
	];
	const REPO_KEY = 'vestige.healing.repo.v1';

	interface TensionPair {
		relation: string;
		source_id: string;
		target_id: string;
		recorded_at?: string;
		memory_a_id: string;
		memory_a_preview: string;
		memory_a_type: string;
		memory_a_created: string;
		memory_b_id: string;
		memory_b_preview: string;
		memory_b_type: string;
		memory_b_created: string;
	}
	interface CodeMemory {
		id: string;
		content: string;
		createdAt: string;
		anchorStatus: string;
		/** evidence.state: needs_recheck, unchanged_evidence, partial, unanchored. */
		state: string;
		anchorNote?: string;
		staleReason?: string;
		retentionStrength?: number;
		kind: 'decision' | 'pattern';
	}
	interface GhostPair {
		firstId: string;
		secondId: string;
		firstPreview: string;
		secondPreview: string;
		firstNodeType: string;
		secondNodeType: string;
		compositionQuestion: string;
		reason: string;
		score: number;
		proof?: { path?: { from: string; to: string; kind: string }[]; hops?: number; neverWoven?: boolean };
	}

	let lens = $state<Lens>('tension');
	let loading = $state(true);
	let error = $state<string | null>(null);
	let titleEl = $state<HTMLElement | null>(null);
	let pickedId = $state<string | null>(null);

	let pairs = $state<TensionPair[]>([]);
	let relationFilter = $state<string | null>(null);

	let repoPath = $state('');
	let codebaseName = $state('vestige');
	let code = $state<CodeMemory[]>([]);
	let verification = $state<{ checked?: number; fresh?: number; stale?: number; unverifiable?: number; warning?: string | null } | null>(null);
	let verifyBusy = $state(false);
	let verifyNote = $state<string | null>(null);

	let ghosts = $state<GhostPair[]>([]);
	let ghostMems = $state<Map<string, TraceMemory>>(new Map());
	let ghostLens = $state<'bridge' | 'divergent'>('bridge');
	let weaveNote = $state<string | null>(null);

	let dupes = $state<{ clusters: unknown[]; totalScanned?: number; note?: string } | null>(null);

	// ── The river for each lens ─────────────────────────────────────────────
	const shownPairs = $derived(relationFilter ? pairs.filter((p) => p.relation === relationFilter) : pairs);

	const tensionMems = $derived.by(() => {
		const out = new Map<string, TraceMemory>();
		for (const p of shownPairs) {
			out.set(p.memory_a_id, { id: p.memory_a_id, type: p.memory_a_type, content: p.memory_a_preview, createdAt: p.memory_a_created, retention: 0.85, tags: [] });
			out.set(p.memory_b_id, { id: p.memory_b_id, type: p.memory_b_type, content: p.memory_b_preview, createdAt: p.memory_b_created, retention: 0.85, tags: [] });
		}
		return [...out.values()];
	});
	/** Drifted, missing or moved anchors: the code changed under the memory. */
	const isStale = (c: CodeMemory) => c.state === 'needs_recheck' || ['drifted', 'missing', 'moved'].includes(c.anchorStatus);
	const isFresh = (c: CodeMemory) => !isStale(c) && (c.state === 'unchanged_evidence' || c.state === 'partial' || c.anchorStatus === 'verified');
	const codeMems = $derived(
		code.map<TraceMemory>((c) => ({
			id: c.id,
			type: isStale(c) ? 'correction' : isFresh(c) ? 'event' : 'note',
			content: c.content.replace(/^#\s*/, ''),
			createdAt: c.createdAt,
			retention: c.retentionStrength ?? 0.8,
			tags: [],
			lane: isStale(c) ? 0.45 : isFresh(c) ? -0.05 : -0.55,
			laneLabel: isStale(c) ? 'code drifted' : isFresh(c) ? 'still holds' : 'not checked'
		}))
	);
	const ghostRiver = $derived([...ghostMems.values()]);

	const river = $derived(lens === 'drift' ? codeMems : lens === 'ghost' ? ghostRiver : tensionMems);
	const links = $derived.by<TraceLink[]>(() => {
		if (lens === 'tension') {
			// Oldest-to-newest arches: what was wrong, into what healed it.
			return shownPairs.map((p) => ({ source: p.source_id, target: p.target_id, type: p.relation }));
		}
		if (lens === 'ghost') return ghosts.map((g) => ({ source: g.firstId, target: g.secondId, type: 'evidence_of' }));
		return [];
	});
	const byId = $derived(new Map(river.map((m) => [m.id, m])));
	let verify = $state<VerifyState>('idle');
	/** A connected memory: fly to it when it is on this river, else open it in Trace. */
	function focusPick(id: string) {
		if (byId.has(id)) pickedId = id;
		else void goto(osHref('/trace', { q: id }));
	}
	$effect(() => {
		if (!pickedId) verify = 'idle';
	});

	// ── Loading ──────────────────────────────────────────────────────────────
	async function load() {
		loading = true;
		error = null;
		pickedId = null;
		try {
			if (lens === 'tension') {
				const res = await api.contradictions({ limit: 500 });
				pairs = ((res as unknown as { contradictions?: TensionPair[] }).contradictions ?? []).filter((p) => p.source_id && p.target_id);
			} else if (lens === 'drift') {
				await loadCode();
			} else if (lens === 'ghost') {
				await loadGhosts();
			} else {
				const res = await api.tool<{ duplicateClusters?: { clusters?: unknown[]; totalScanned?: number; note?: string } }>('dedup', { action: 'scan', limit: 50 });
				dupes = { clusters: res.duplicateClusters?.clusters ?? [], totalScanned: res.duplicateClusters?.totalScanned, note: res.duplicateClusters?.note };
				if (!pairs.length) {
					const t = await api.contradictions({ limit: 500 }).catch(() => null);
					pairs = ((t as unknown as { contradictions?: TensionPair[] } | null)?.contradictions ?? []).filter((p) => p.source_id && p.target_id);
				}
			}
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	async function loadCode() {
		if (!repoPath.trim()) {
			code = [];
			verification = null;
			return;
		}
		const res = await api.tool<{
			decisions?: { items?: Record<string, unknown>[] };
			patterns?: { items?: Record<string, unknown>[] };
			verification?: typeof verification;
		}>('codebase', { action: 'get_context', codebase: codebaseName.trim() || 'vestige', repoPath: repoPath.trim(), limit: 100 });
		const read = (items: Record<string, unknown>[] | undefined, kind: 'decision' | 'pattern') =>
			(items ?? []).map<CodeMemory>((r) => ({
				id: String(r.id),
				content: String(r.content ?? ''),
				createdAt: String(r.createdAt ?? ''),
				anchorStatus: String(r.anchorStatus ?? 'unanchored'),
				state: String((r.evidence as Record<string, unknown> | undefined)?.state ?? ''),
				anchorNote: typeof r.anchorNote === 'string' ? r.anchorNote : undefined,
				staleReason: typeof r.staleReason === 'string' ? r.staleReason : undefined,
				retentionStrength: typeof r.retentionStrength === 'number' ? r.retentionStrength : undefined,
				kind
			}));
		code = [...read(res.decisions?.items, 'decision'), ...read(res.patterns?.items, 'pattern')];
		verification = res.verification ?? null;
	}

	async function reverify() {
		verifyBusy = true;
		verifyNote = null;
		try {
			const res = await api.tool<Record<string, unknown>>('codebase', {
				action: 'verify',
				codebase: codebaseName.trim() || 'vestige',
				repoPath: repoPath.trim(),
				limit: 100
			});
			const v = (res.verification ?? res.summary ?? res) as Record<string, unknown>;
			verifyNote = `Re-checked ${v.checked ?? '?'} anchors: ${v.fresh ?? '?'} fresh, ${v.stale ?? '?'} stale, ${v.unverifiable ?? '?'} unverifiable.`;
			await loadCode();
		} catch (e) {
			verifyNote = e instanceof Error ? e.message : String(e);
		} finally {
			verifyBusy = false;
		}
	}

	async function loadGhosts() {
		const res = await api.tool<{ candidates?: GhostPair[] }>('ghostlink', { mode: 'propose', lens: ghostLens, limit: 24 });
		ghosts = res.candidates ?? [];
		ghostMems = await fetchMemories([...new Set(ghosts.flatMap((g) => [g.firstId, g.secondId]))]);
	}

	async function weave(g: GhostPair, outcome: 'helpful' | 'dead_end' | 'needs_poc') {
		weaveNote = null;
		try {
			const res = await api.tool<{ receiptId?: string; compositionId?: string }>('ghostlink', {
				mode: 'weave',
				lens: ghostLens,
				first_id: g.firstId,
				second_id: g.secondId,
				outcome_type: outcome
			});
			weaveNote = `Woven as ${outcome.replace('_', ' ')} · receipt ${res.receiptId ?? 'not returned'}`;
		} catch (e) {
			weaveNote = e instanceof Error ? e.message : String(e);
		}
	}

	function setLens(next: Lens) {
		if (next === lens) return;
		lens = next;
		void load();
	}

	// ── Words ────────────────────────────────────────────────────────────────
	const pairsOfPicked = $derived(pickedId ? shownPairs.filter((p) => p.source_id === pickedId || p.target_id === pickedId) : []);
	const ghostsOfPicked = $derived(pickedId ? ghosts.filter((g) => g.firstId === pickedId || g.secondId === pickedId) : []);
	const codeOfPicked = $derived(pickedId ? (code.find((c) => c.id === pickedId) ?? null) : null);

	const relationCounts = $derived.by(() => {
		const c = new Map<string, number>();
		for (const p of pairs) c.set(p.relation, (c.get(p.relation) ?? 0) + 1);
		return [...c.entries()].sort((a, b) => b[1] - a[1]);
	});

	const RELATION_WORDS: Record<string, [string, string]> = {
		closed_by: ['was closed by', 'closed'],
		corrects: ['corrects', 'was corrected by'],
		supersedes: ['supersedes', 'was superseded by'],
		contradicts: ['contradicts', 'is contradicted by']
	};
	/** How the picked memory relates to the other side of a pair, in words. */
	function relationFrom(p: TensionPair, id: string): string {
		const [fwd, back] = RELATION_WORDS[p.relation] ?? [p.relation, `${p.relation} (reverse)`];
		return p.source_id === id ? fwd : back;
	}
	function otherSide(p: TensionPair, id: string) {
		return p.memory_a_id === id
			? { id: p.memory_b_id, text: p.memory_b_preview, type: p.memory_b_type, at: p.memory_b_created }
			: { id: p.memory_a_id, text: p.memory_a_preview, type: p.memory_a_type, at: p.memory_a_created };
	}

	function describe(id: string): StageWords | null {
		const m = byId.get(id);
		if (!m) return null;
		if (lens === 'drift') {
			const c = code.find((x) => x.id === id);
			return { kicker: `${c?.kind ?? 'code'} · ${c?.anchorStatus ?? ''} · ${when(m.createdAt)}`, text: m.content, tone: m.type };
		}
		const rel = lens === 'tension' ? shownPairs.find((p) => p.source_id === id || p.target_id === id) : null;
		return { kicker: `${m.type}${rel ? ` · ${relationFrom(rel, id)}` : ''} · ${when(m.createdAt)}`, text: m.content, tone: rel?.relation ?? m.type };
	}

	const labelIds = $derived.by(() => {
		if (pickedId) {
			const others =
				lens === 'tension'
					? pairsOfPicked.map((p) => otherSide(p, pickedId!).id)
					: lens === 'ghost'
						? ghostsOfPicked.map((g) => (g.firstId === pickedId ? g.secondId : g.firstId))
						: [];
			return [pickedId, ...others.slice(0, 5)];
		}
		return [];
	});

	const rgb = (t: string) => {
		const [r, g, b] = ['closed_by', 'corrects', 'supersedes', 'contradicts', 'evidence_of'].includes(t) ? edgeColor(t) : typeColor(t);
		return `rgb(${Math.round(r * 255)} ${Math.round(g * 255)} ${Math.round(b * 255)})`;
	};
	function when(iso?: string): string {
		if (!iso) return '';
		const d = new Date(iso);
		return Number.isFinite(d.getTime()) ? d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '';
	}

	onMount(() => {
		try {
			repoPath = localStorage.getItem(REPO_KEY) ?? '';
		} catch {
			repoPath = '';
		}
		void load();
	});
	$effect(() => {
		const r = repoPath;
		try {
			if (r) localStorage.setItem(REPO_KEY, r);
		} catch {
			// Storage blocked: the path is asked for again next visit.
		}
	});
</script>

<svelte:head>
	<title>Healing · Vestige</title>
</svelte:head>

<OrganStage
	memories={river}
	{links}
	selectedId={pickedId}
	{verify}
	origin={() => {
		const r = titleEl?.getBoundingClientRect();
		return r ? { x: ((r.left + 160) / innerWidth) * 2 - 1, y: 1 - ((r.top + r.height / 2) / innerHeight) * 2 } : undefined;
	}}
	{describe}
	{labelIds}
	onpick={(id) => (pickedId = id)}
	story="healing"
	seed="vestige-healing-v2"
	axis="order"
/>

<header class="tr-ask" class:tr-quiet={!!pickedId}>
	<p class="tr-eyebrow" use:alive={{ text: 'Healing', glow: true }}></p>
	<h1 class="tr-title" bind:this={titleEl}>
		{lens === 'drift' ? 'Where the code moved under its memories.' : lens === 'ghost' ? 'What the record joins but no one has composed.' : 'Where memory corrected itself.'}
	</h1>
	<p class="tr-hint">
		{#each LENSES as l (l.key)}
			<button type="button" class="tr-chip" aria-pressed={lens === l.key} onclick={() => setLens(l.key)}>{l.label}</button>
		{/each}
	</p>
	{#if loading}
		<p class="tr-line">Reading the recorded edges…</p>
	{:else if error}
		<p class="tr-line tr-err">{error}</p>
	{:else if lens === 'tension'}
		<p class="tr-line">
			<strong use:alive={{ text: String(pairs.length), count: true }}></strong> recorded {pairs.length === 1 ? 'edge' : 'edges'} where one memory corrected, replaced or closed another.
			Each arc runs from the older memory into the one that healed it.
		</p>
		<p class="tr-types">
			{#each relationCounts as [rel, n] (rel)}
				<button type="button" class="tr-chip" aria-pressed={relationFilter === null ? undefined : relationFilter === rel} onclick={() => (relationFilter = relationFilter === rel ? null : rel)}>
					<i style:background={rgb(rel)}></i>{rel.replace('_', ' ')} {n}
				</button>
			{/each}
		</p>
	{:else if lens === 'drift'}
		<form class="tr-hint" onsubmit={(e) => { e.preventDefault(); void load(); }}>
			<input class="tr-field" style="max-width: 9rem" bind:value={codebaseName} aria-label="Codebase name" spellcheck="false" />
			<input class="tr-field" style="max-width: 24rem" bind:value={repoPath} placeholder="absolute path of the checkout" aria-label="Repository path" spellcheck="false" />
			<button type="submit" class="tr-chip">check against the source</button>
		</form>
		{#if verification}
			<p class="tr-line">
				<strong use:alive={{ text: String(verification.checked ?? 0), count: true }}></strong> code memories checked against the live checkout:
				<span class="tr-ok">{verification.fresh ?? 0} still match</span>,
				<span class="tr-err">{verification.stale ?? 0} drifted</span>,
				{verification.unverifiable ?? 0} carry no anchor to check.
			</p>
		{:else}
			<p class="tr-line">Give the checkout's path and every code memory is checked against the source as it is now.</p>
		{/if}
	{:else if lens === 'ghost'}
		<p class="tr-line">
			<strong use:alive={{ text: String(ghosts.length), count: true }}></strong> pairs a typed path joins that were never composed, each with its proof. Each arc is a question nobody has asked yet.
		</p>
		<p class="tr-hint">
			<button type="button" class="tr-chip" aria-pressed={ghostLens === 'bridge'} onclick={() => { ghostLens = 'bridge'; void load(); }}>bridges</button>
			<button type="button" class="tr-chip" aria-pressed={ghostLens === 'divergent'} onclick={() => { ghostLens = 'divergent'; void load(); }}>divergent</button>
		</p>
	{:else}
		<p class="tr-line">
			{#if dupes && dupes.clusters.length === 0}
				<strong>0</strong> exact duplicates among {dupes.totalScanned?.toLocaleString() ?? 'all'} memories.
			{:else if dupes}
				<strong use:alive={{ text: String(dupes.clusters.length), count: true }}></strong> clusters of exact duplicates among {dupes.totalScanned?.toLocaleString() ?? 'all'} memories.
			{/if}
		</p>
		{#if dupes?.note}<p class="tr-nudge">{dupes.note}</p>{/if}
	{/if}
</header>

<aside class="tr-lane" class:idle={!pickedId} aria-live="polite">
	{#if pickedId && lens === 'tension'}
		<MemoryFocus id={pickedId} onpick={focusPick} onverify={(v) => (verify = v)} onclose={() => (pickedId = null)}>
			{#each pairsOfPicked as p (p.source_id + p.relation + p.target_id)}
				{@const o = otherSide(p, pickedId)}
				<div class="tr-block">
					<p class="tr-proof-kicker" style:color={rgb(p.relation)}>{relationFrom(p, pickedId)}</p>
					<button type="button" class="tr-item" data-trace-id={o.id} onclick={() => (pickedId = o.id)}>
						<span class="tr-edge">{o.type} · {when(o.at)}</span>
						<span>{headline(o.text, 220)}</span>
					</button>
					<p class="tr-note">recorded {when(p.recorded_at)} · {p.source_id} → {p.target_id}</p>
				</div>
			{/each}
		</MemoryFocus>
	{:else if pickedId && lens === 'drift' && codeOfPicked}
		<MemoryFocus id={pickedId} onpick={focusPick} onverify={(v) => (verify = v)} onclose={() => (pickedId = null)}>
			<div class="tr-block">
				<p class="tr-proof-kicker" class:tr-err={isStale(codeOfPicked)} class:tr-ok={isFresh(codeOfPicked)}>
					{codeOfPicked.kind} · anchor {codeOfPicked.anchorStatus}
				</p>
				{#if codeOfPicked.staleReason}<p class="tr-note tr-warn">{codeOfPicked.staleReason}</p>{/if}
				{#if codeOfPicked.anchorNote}<p class="tr-note">{codeOfPicked.anchorNote}</p>{/if}
			</div>
		</MemoryFocus>
	{:else if pickedId && lens === 'ghost'}
		<MemoryFocus id={pickedId} onpick={focusPick} onverify={(v) => (verify = v)} onclose={() => (pickedId = null)}>
			{#each ghostsOfPicked as g (g.firstId + g.secondId)}
				<div class="tr-block">
					<p class="tr-proof-kicker">never composed · {g.proof?.hops ?? '?'} hop{g.proof?.hops === 1 ? '' : 's'} · score {g.score}</p>
					<p class="tr-text" use:alive={{ text: g.compositionQuestion }}></p>
					<button type="button" class="tr-item" data-trace-id={g.firstId} onclick={() => (pickedId = g.firstId)}>
						<span class="tr-edge">{g.firstNodeType}</span><span>{headline(g.firstPreview, 160)}</span>
					</button>
					<button type="button" class="tr-item" data-trace-id={g.secondId} onclick={() => (pickedId = g.secondId)}>
						<span class="tr-edge">{g.secondNodeType}</span><span>{headline(g.secondPreview, 160)}</span>
					</button>
					{#if g.proof?.path?.length}
						<p class="tr-note">proof: {g.proof.path.map((s) => `${s.from} —${s.kind}→ ${s.to}`).join(' · ')}</p>
					{/if}
					<div class="tr-actions">
						<button type="button" class="tr-act alt" onclick={() => void weave(g, 'helpful')}>It held up</button>
						<button type="button" class="tr-act quiet" onclick={() => void weave(g, 'needs_poc')}>Needs proof</button>
						<button type="button" class="tr-act quiet" onclick={() => void weave(g, 'dead_end')}>Dead end</button>
					</div>
				</div>
			{/each}
			{#if weaveNote}<p class="tr-note">{weaveNote}</p>{/if}
		</MemoryFocus>
	{:else if lens === 'drift'}
		<p class="tr-proof-kicker" use:alive={{ text: 'Code memories' }}></p>
		{#if verification?.warning}<p class="tr-note tr-warn">{verification.warning}</p>{/if}
		{#each code.filter(isStale).slice(0, 12) as c (c.id)}
			<button type="button" class="tr-item"
				data-trace-id={c.id} onclick={() => (pickedId = c.id)}>
				<span class="tr-edge causal">{c.anchorStatus} {c.kind}</span><span>{headline(c.content.replace(/^#\s*/, ''), 110)}</span>
			</button>
		{:else}
			<p class="tr-note">{repoPath ? 'No drifted anchors in this checkout.' : 'Code memories appear here once a checkout path is given.'}</p>
		{/each}
		{#if repoPath}
			<div class="tr-actions">
				<button type="button" class="tr-act" disabled={verifyBusy} onclick={() => void reverify()}>{verifyBusy ? 'Checking anchors…' : 'Re-verify every anchor'}</button>
			</div>
			{#if verifyNote}<p class="tr-note">{verifyNote}</p>{/if}
		{/if}
	{:else if lens === 'ghost'}
		<p class="tr-proof-kicker" use:alive={{ text: 'Questions nobody has asked' }}></p>
		{#each ghosts.slice(0, 10) as g (g.firstId + g.secondId)}
			<button type="button" class="tr-item"
				data-trace-id={g.firstId} onclick={() => (pickedId = g.firstId)}>
				<span class="tr-edge causal">{g.firstNodeType} × {g.secondNodeType}</span><span>{headline(g.compositionQuestion, 140)}</span>
			</button>
		{:else}
			<p class="tr-note">{loading ? '' : 'Every joined pair has been composed.'}</p>
		{/each}
	{:else}
		<p class="tr-proof-kicker" use:alive={{ text: 'Healed lately' }}></p>
		{#each [...shownPairs].sort((a, b) => Date.parse(b.recorded_at ?? '') - Date.parse(a.recorded_at ?? '')).slice(0, 10) as p (p.source_id + p.relation + p.target_id)}
			<button type="button" class="tr-item"
				data-trace-id={p.target_id} onclick={() => (pickedId = p.target_id)}>
				<span class="tr-edge" style:color={rgb(p.relation)}>{p.relation.replace('_', ' ')} · {when(p.recorded_at)}</span>
				<span>{headline(p.memory_b_preview, 110)}</span>
			</button>
		{:else}
			<p class="tr-note">{loading ? '' : 'No recorded tension yet.'}</p>
		{/each}
		<p class="tr-note">Click a light to see what it corrected, or what corrected it.</p>
	{/if}
</aside>
