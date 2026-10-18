<script lang="ts">
	/**
	 * Trace — trace your own memory.
	 *
	 * 1. Answer: "What happened today?" resolved by an exact handle (a day, an
	 *    id or a tag), never by resemblance. Pick any memory and its receipt is
	 *    re-verified against the hash-chained log on the spot.
	 * 2. Why: causal_walk follows recorded edges backward from that memory;
	 *    forgotten_lesson reads how well each lesson on the path is retained.
	 * 3. Cinema: the answer and its causes become a Memory Cinema film.
	 *
	 * Everything here is a live tool call through /api/mcp (the same path an
	 * agent's call takes). Nothing is hardcoded.
	 */
	import '$lib/trace/trace-organ.css';
	import { alive } from '$lib/trace/alive';
	import { onMount, tick } from 'svelte';
	import { replaceState } from '$app/navigation';
	import MemoryCinema from '$components/MemoryCinema.svelte';
	import OrganStage from '$lib/trace/OrganStage.svelte';
	import type { StageWords } from '$lib/trace/TraceStage.svelte';
	import { api } from '$lib/stores/api';
	import { osHref } from '$lib/os-nav';
	import type { GraphEdge, GraphNode } from '$types';
	import { typeColor, type VerifyState } from '$lib/trace/trace-pass';
	import {
		headline,
		isCausalEdge,
		normalizeReceipt,
		normalizeWalk,
		parseHandle,
		TRACE_SEEN_KEY,
		walkEdges,
		type ParsedHandle,
		type ReceiptProof,
		type TraceMemory,
		type WalkResult
	} from '$lib/trace/trace-data';
	import { toTraceMemory } from '$lib/trace/trace-memory';
	import { fetchWindow, recallTag } from '$lib/trace/trace-fetch';

	type Beat = 1 | 2 | 3;

	let question = $state('What happened today?');
	let questionEl = $state<HTMLInputElement | null>(null);
	let parsed = $state<ParsedHandle | null>(null);
	let answer = $state<TraceMemory[]>([]);
	let answerTotal = $state(0);
	let range = $state<{ start: number; end: number } | null>(null);
	let loading = $state(false);
	let error = $state<string | null>(null);
	let newestDay = $state<string | null>(null);
	let beat = $state<Beat>(1);

	let picked = $state<TraceMemory | null>(null);
	let receipt = $state<ReceiptProof | null>(null);
	let receiptBusy = $state(false);
	let receiptError = $state<string | null>(null);
	let receiptMs = $state<number | null>(null);
	let verify = $state<VerifyState>('idle');

	let walk = $state<WalkResult | null>(null);
	let walkBusy = $state(false);
	let walkError = $state<string | null>(null);
	/** Live FSRS retention (percent) of each cause, from forgotten_lesson. */
	let retained = $state<Map<string, number>>(new Map());

	let editing = $state(false);
	let editDraft = $state('');
	let suppressConfirm = $state('');
	let changeNote = $state<string | null>(null);

	/** Admit a corrected successor; the old node retires and the id changes. */
	async function edit() {
		if (!picked || !editDraft.trim()) return;
		try {
			const res = await api.tool<{ id?: string; nodeId?: string; newId?: string; successorId?: string; receiptId?: string }>('memory', {
				action: 'edit',
				id: picked.id,
				content: editDraft.trim()
			});
			const next = res.successorId ?? res.newId ?? res.nodeId ?? res.id;
			changeNote = `Edited · new id ${next ?? 'not returned'} · receipt ${res.receiptId ?? 'not returned'}`;
			editing = false;
			if (next && next !== picked.id) {
				question = next;
				void ask(next);
			}
		} catch (e) {
			changeNote = e instanceof Error ? e.message : String(e);
		}
	}

	/** Hide a memory from every read. On 4.0 this cannot be undone, and it is not erasure. */
	async function suppress() {
		if (!picked || suppressConfirm.trim() !== 'suppress') return;
		try {
			const res = await api.tool<{ receiptId?: string }>('suppress', { id: picked.id, reason: 'Suppressed from the Trace organ' });
			changeNote = `Suppressed · receipt ${res.receiptId ?? 'not returned'}. Its bytes stay on the log.`;
			suppressConfirm = '';
		} catch (e) {
			changeNote = e instanceof Error ? e.message : String(e);
		}
	}

	/** The last feedback write on the picked memory, with its receipt. */
	let feedback = $state<{ action: string; receiptId: string | null; error?: string } | null>(null);
	let feedbackBusy = $state(false);

	let cinemaHost = $state<HTMLDivElement | null>(null);
	let cinemaData = $state<{ nodes: GraphNode[]; edges: GraphEdge[]; centerId: string } | null>(null);
	let cinemaKey = $state(0);

	const contentById = $derived(new Map(answer.map((m) => [m.id, m])));

	/** Where an answer erupts from: the question itself, in NDC. */
	function questionOrigin(): { x: number; y: number } | undefined {
		if (!questionEl) return undefined;
		const q = questionEl.getBoundingClientRect();
		return {
			x: ((q.left + Math.min(q.width, 420) * 0.5) / innerWidth) * 2 - 1,
			y: 1 - ((q.top + q.height * 0.5) / innerHeight) * 2
		};
	}

	async function ask(input = question) {
		const p = parseHandle(input);
		if (!p) return;
		parsed = p;
		loading = true;
		error = null;
		newestDay = null;
		const url = new URL(location.href);
		if (url.searchParams.get('q') !== p.raw) {
			url.searchParams.set('q', p.raw);
			try {
				replaceState(url, {});
			} catch {
				// The router is not up yet on the very first ask; the next ask
				// writes the handle into the URL.
			}
		}
		resetPick();
		try {
			let mems: TraceMemory[] = [];
			if (p.kind === 'day') {
				const day = await fetchWindow(p.start!, p.end!, 6);
				mems = day.memories;
				answerTotal = mems.length;
				if (mems.length === 0) {
					const stats = await api.stats().catch(() => null);
					newestDay = stats?.newestMemory ? localDate(stats.newestMemory) : null;
				}
			} else if (p.kind === 'id') {
				const m = await api.memories.get(p.raw);
				const one = toTraceMemory(m as unknown as Record<string, unknown>);
				mems = one ? [one] : [];
				answerTotal = mems.length;
			} else {
				mems = await recallTag(p.raw, 100);
				answerTotal = mems.length;
			}
			mems.sort((a, b) => Date.parse(a.createdAt) - Date.parse(b.createdAt));
			range = p.kind === 'day' && p.start && p.end ? { start: p.start.getTime(), end: p.end.getTime() } : null;
			answer = mems;
			beat = 1;
			try {
				localStorage.setItem(TRACE_SEEN_KEY, '1');
			} catch {
				// Storage blocked: the root simply keeps opening Trace first.
			}
			if (p.kind === 'id' && mems[0]) void pick(mems[0]);
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
			answer = [];
		} finally {
			loading = false;
		}
	}

	function localDate(iso: string): string {
		const d = new Date(iso);
		const p = (n: number) => String(n).padStart(2, '0');
		return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
	}

	function resetPick() {
		picked = null;
		receipt = null;
		receiptError = null;
		verify = 'idle';
		walk = null;
		walkError = null;
		retained = new Map();
		feedback = null;
		editing = false;
		editDraft = '';
		suppressConfirm = '';
		changeNote = null;
		cinemaData = null;
	}

	async function pick(m: TraceMemory) {
		resetPick();
		picked = m;
		receiptBusy = true;
		verify = 'running';
		const t0 = performance.now();
		try {
			const proof = normalizeReceipt(await api.tool('receipt', { action: 'get', receipt_id: m.id }));
			if (picked?.id !== m.id) return;
			receipt = proof;
			receiptMs = Math.round(performance.now() - t0);
			verify = proof.chainValid ? 'ok' : 'fail';
		} catch (e) {
			if (picked?.id !== m.id) return;
			receiptError = e instanceof Error ? e.message : String(e);
			verify = 'fail';
		} finally {
			if (picked?.id === m.id) receiptBusy = false;
		}
	}

	async function why() {
		if (!picked) return;
		const at = picked.id;
		beat = 2;
		walkBusy = true;
		walkError = null;
		try {
			const raw = await api.tool('causal_walk', {
				start_points: [{ kind: 'logged_write', node_id: at }],
				promote: false
			});
			if (picked?.id !== at) return;
			walk = normalizeWalk(raw);
			// The live retention of each lesson on this path (FSRS); below 50%
			// it is fading out of reach.
			const lessons = await api
				.tool<{ forgotten_lessons?: { lesson_id?: string; retention_pct?: number }[] }>('forgotten_lesson', { failure_id: at })
				.catch(() => null);
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

	/** Feedback the way an agent gives it: promote what helped, demote what was wrong. */
	async function rate(action: 'promote' | 'demote') {
		if (!picked) return;
		feedbackBusy = true;
		try {
			const res = await api.tool<{ receiptId?: string; receipt_id?: string }>('memory', {
				action,
				id: picked.id,
				reason: action === 'promote' ? 'Marked helpful from the Trace organ' : 'Marked wrong from the Trace organ'
			});
			feedback = { action, receiptId: res.receiptId ?? res.receipt_id ?? null };
		} catch (e) {
			feedback = { action, receiptId: null, error: e instanceof Error ? e.message : String(e) };
		} finally {
			feedbackBusy = false;
		}
	}

	async function film() {
		if (!picked) return;
		beat = 3;
		const nodes = new Map<string, GraphNode>();
		const add = (id: string, label: string, type: string, retention: number, createdAt: string, isCenter: boolean) => {
			if (nodes.has(id)) return;
			nodes.set(id, { id, label: headline(label, 80), type, retention, tags: [], createdAt, updatedAt: createdAt, isCenter });
		};
		add(picked.id, picked.content, picked.type, picked.retention, picked.createdAt, true);
		for (const n of walk?.nodes ?? []) {
			const m = contentById.get(n.id);
			add(n.id, n.content, m?.type ?? 'memory', m?.retention ?? 0.8, m?.createdAt ?? picked.createdAt, false);
		}
		// The rest of the answer around it, so the film tours the day too.
		for (const m of answer.slice(0, 40)) add(m.id, m.content, m.type, m.retention, m.createdAt, false);
		const edges: GraphEdge[] = walk
			? walkEdges(walk).map((e) => ({ source: e.source_id, target: e.target_id, weight: isCausalEdge(e.link_type) ? 1 : 0.35, type: e.link_type }))
			: [];
		cinemaData = { nodes: [...nodes.values()], edges, centerId: picked.id };
		cinemaKey++;
		await tick();
		cinemaHost?.querySelector<HTMLButtonElement>('.cinema-launch')?.click();
	}

	function onpick(id: string) {
		const m = contentById.get(id);
		if (m) void pick(m);
		else if (walk?.nodes.some((n) => n.id === id)) {
			question = id;
			void ask(id);
		}
	}

	function describe(id: string): StageWords | null {
		const m = contentById.get(id);
		const w = walk?.nodes.find((n) => n.id === id);
		if (w && w.depth > 0) {
			const edge = lastEdge(w);
			return { kicker: `${edge} · depth ${w.depth}${m ? ` · ${time(m.createdAt)}` : ''}`, text: w.content || m?.content || id, tone: edge };
		}
		if (m) return { kicker: `${m.type} · ${time(m.createdAt)}`, text: m.content, tone: m.type };
		return null;
	}

	const labelIds = $derived(
		picked ? [picked.id, ...(walk?.nodes ?? []).filter((n) => n.depth > 0).slice(0, 6).map((n) => n.id)] : []
	);

	const typeCounts = $derived.by(() => {
		const c = new Map<string, number>();
		for (const m of answer) c.set(m.type, (c.get(m.type) ?? 0) + 1);
		return [...c.entries()].sort((a, b) => b[1] - a[1]);
	});

	const walkByDepth = $derived.by(() => {
		const out = new Map<number, WalkResult['nodes']>();
		for (const n of walk?.nodes ?? []) {
			if (n.depth === 0) continue;
			out.set(n.depth, [...(out.get(n.depth) ?? []), n]);
		}
		return [...out.entries()].sort((a, b) => a[0] - b[0]);
	});

	function lastEdge(n: WalkResult['nodes'][number]): string {
		return n.path[n.path.length - 1]?.link_type ?? 'edge';
	}

	const rgb = (t: string) => {
		const [r, g, b] = typeColor(t);
		return `rgb(${Math.round(r * 255)} ${Math.round(g * 255)} ${Math.round(b * 255)})`;
	};
	const time = (iso: string) => {
		const d = new Date(iso);
		return Number.isFinite(d.getTime()) ? d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : '';
	};

	onMount(() => {
		// ?q= carries the handle, so a trace can be linked and replayed.
		const q = new URL(location.href).searchParams.get('q');
		if (q) question = q;
		void ask();
	});
</script>

<svelte:head>
	<title>Trace · Vestige</title>
</svelte:head>

<OrganStage
	memories={answer}
	{range}
	{walk}
	selectedId={picked?.id ?? null}
	{verify}
	origin={questionOrigin}
	{describe}
	{labelIds}
	{onpick}
	seed="vestige-trace-v1"
/>

<header class="tr-ask" class:tr-quiet={!!picked}>
	<p class="tr-eyebrow" use:alive={{ text: 'Trace', glow: true }}></p>
	<form onsubmit={(e) => { e.preventDefault(); void ask(); }}>
		<input
			class="tr-question"
			bind:value={question}
			bind:this={questionEl}
			aria-label="Ask by exact handle: today, yesterday, a date, a memory id or a tag"
			spellcheck="false"
			autocomplete="off"
		/>
	</form>
	<p class="tr-hint">
		{#each ['today', 'yesterday'] as h (h)}
			<button type="button" class="tr-chip" onclick={() => { question = h; void ask(h); }}>{h}</button>
		{/each}
		<span class="tr-hint-text">or a date, a memory id, an exact tag</span>
	</p>
	{#if loading}
		<p class="tr-line">Resolving the exact handle…</p>
	{:else if error}
		<p class="tr-line tr-err">{error}</p>
	{:else if parsed}
		<p class="tr-line">
			<strong use:alive={{ text: String(answerTotal), count: true }}></strong>
			{answerTotal === 1 ? 'memory' : 'memories'}
			{parsed.kind === 'day' ? `written ${parsed.label}` : parsed.kind === 'tag' ? `under ${parsed.label}` : ''},
			found by exact {parsed.kind === 'day' ? 'date' : parsed.kind === 'tag' ? 'tag' : 'id'}. No similarity, no vectors.
		</p>
		{#if typeCounts.length}
			<p class="tr-types">
				{#each typeCounts.slice(0, 6) as [t, n] (t)}
					<span><i style:background={rgb(t)}></i>{t} {n}</span>
				{/each}
			</p>
		{/if}
		{#if answerTotal === 0 && newestDay}
			<p class="tr-line">
				Nothing written {parsed.label} yet. The newest day with memories is
				<button type="button" class="tr-chip" onclick={() => { question = newestDay!; void ask(newestDay!); }}>{newestDay}</button>
			</p>
		{/if}
		{#if answer.length && !picked}
			<p class="tr-nudge">Each world is one kind of memory, newest at its heart. Click a world to open every memory in it; click an orb to go inside it.</p>
		{/if}
	{/if}
</header>

<nav class="tr-beats" class:under-lane={!!picked} aria-label="Trace steps">
	<span class:on={beat === 1}>1 · Answer + receipt</span>
	<span class:on={beat === 2}>2 · Why</span>
	<span class:on={beat === 3}>3 · Cinema</span>
</nav>

{#if picked}
	<aside class="tr-lane" aria-live="polite">
		<p class="tr-kicker" style:color={rgb(picked.type)}>{picked.type} · {time(picked.createdAt)}</p>
		<p class="tr-text" use:alive={{ text: headline(picked.content, 420) }}></p>
		<a class="tr-id" href={osHref('/field', { memory: picked.id })}>{picked.id}</a>

		<div class="tr-block">
			<p class="tr-proof-kicker" use:alive={{ text: 'Receipt' }}></p>
			{#if receiptBusy}
				<p class="tr-proof-line">Re-verifying the hash-chained log…</p>
			{:else if receiptError}
				<p class="tr-proof-line tr-err">{receiptError}</p>
			{:else if receipt}
				<p class="tr-id">{receipt.receiptId ?? '—'}</p>
				<p class="tr-proof-line">
					{#if receipt.chainValid}<span class="tr-ok">chain valid</span>{:else}<span class="tr-err">chain not valid</span>{/if}
					· {receipt.framesVerified?.toLocaleString() ?? '?'} frames · {receipt.segmentsVerified ?? '?'} segments
					· gate {receipt.gateAllowed ? 'allowed' : 'not allowed'}
				</p>
				<p class="tr-note">Re-verified just now in {receiptMs ?? '?'} ms, against the log itself.</p>
			{/if}
		</div>

		{#if walk}
			<div class="tr-block">
				<p class="tr-proof-kicker" use:alive={{ text: 'Why · recorded edges back' }}></p>
				{#if walk.nodes.length <= 1}
					<p class="tr-proof-line">{walk.emptyBecause ?? 'No recorded edge leads upstream from this memory.'}</p>
				{/if}
				{#each walkByDepth.slice(0, 4) as [d, list] (d)}
					<p class="tr-depth">depth {d} · {list.length}</p>
					{#each list.slice(0, 6) as n (n.id)}
						{@const pct = retained.get(n.id)}
						<button type="button" class="tr-item"
				data-trace-id={n.id} class:fading={pct !== undefined && pct < 50} onclick={() => { question = n.id; void ask(n.id); }}>
							<span class="tr-edge" class:causal={isCausalEdge(lastEdge(n))}>{lastEdge(n)}</span>
							<span>{headline(n.content, 110)}</span>
							{#if pct !== undefined}
								<span class="tr-tag">{pct < 50 ? 'fading lesson · ' : ''}{Math.round(pct)}% retained</span>
							{/if}
						</button>
					{/each}
				{/each}
			</div>
		{:else if walkError}
			<p class="tr-proof-line tr-err">{walkError}</p>
		{/if}

		<div class="tr-actions">
			<button type="button" class="tr-act" disabled={walkBusy} onclick={() => void why()}>
				{walkBusy ? 'Walking recorded edges…' : 'Why did this happen?'}
			</button>
			<button type="button" class="tr-act alt" onclick={() => void film()}>Put it on screen</button>
		</div>
		<div class="tr-actions">
			<button type="button" class="tr-act quiet" disabled={feedbackBusy} onclick={() => void rate('promote')}>That helped</button>
			<button type="button" class="tr-act quiet" disabled={feedbackBusy} onclick={() => void rate('demote')}>That was wrong</button>
		</div>
		<div class="tr-actions">
			<button type="button" class="tr-act quiet" onclick={() => { editing = !editing; editDraft = picked?.content ?? ''; }}>{editing ? 'Keep it as it is' : 'Correct it'}</button>
		</div>
		{#if editing}
			<textarea class="tr-field" rows="5" bind:value={editDraft}></textarea>
			<p class="tr-note">A correction is admitted as a successor; this memory retires and the id changes.</p>
			<div class="tr-actions"><button type="button" class="tr-act alt" disabled={!editDraft.trim()} onclick={() => void edit()}>Admit the correction</button></div>
			<input class="tr-field" bind:value={suppressConfirm} placeholder="type suppress to hide it from every read" spellcheck="false" />
			<p class="tr-note">Suppress cannot be undone on 4.0 and is not erasure: the bytes stay on the log.</p>
			<div class="tr-actions"><button type="button" class="tr-act danger" disabled={suppressConfirm.trim() !== 'suppress'} onclick={() => void suppress()}>Suppress it</button></div>
		{/if}
		{#if changeNote}<p class="tr-note">{changeNote}</p>{/if}
		{#if feedback}
			<p class="tr-note">
				{#if feedback.error}<span class="tr-err">{feedback.error}</span>
				{:else}{feedback.action === 'promote' ? 'Promoted' : 'Demoted'} through the gate · receipt {feedback.receiptId ?? 'not returned'}{/if}
			</p>
		{/if}
	</aside>
{/if}

<div class="tr-cinema-host" bind:this={cinemaHost}>
	{#if cinemaData}
		{#key cinemaKey}
			<MemoryCinema nodes={cinemaData.nodes} edges={cinemaData.edges} centerId={cinemaData.centerId} />
		{/key}
	{/if}
</div>
