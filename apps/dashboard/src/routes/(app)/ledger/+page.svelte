<script lang="ts">
	/**
	 * Ledger — every write, in order, with its proof.
	 *
	 * The river is the record over a window (memory_status timeline): each
	 * light a write, placed at the minute it was admitted. A seam marks now;
	 * the active intentions wait downstream of it at their deadlines
	 * (intention list). The lane reads the record: what changed lately
	 * (memory_status changelog), and for any write its receipt (re-verified
	 * live) and the exact log frame that holds it (memory_status provenance).
	 * Writes go through the same gate an agent's do: smart_ingest, intention
	 * set and intention update, each answered with its receipt.
	 */
	import '$lib/trace/trace-organ.css';
	import { alive } from '$lib/trace/alive';
	import { onMount } from 'svelte';
	import OrganStage from '$lib/trace/OrganStage.svelte';
	import type { StageWords } from '$lib/trace/TraceStage.svelte';
	import { api } from '$lib/stores/api';
	import { osHref } from '$lib/os-nav';
	import { typeColor, type VerifyState } from '$lib/trace/trace-pass';
	import { headline, type TraceLink, type TraceMemory } from '$lib/trace/trace-data';
	import MemoryFocus from '$lib/trace/MemoryFocus.svelte';
	import { goto } from '$app/navigation';
	import { fetchWindow } from '$lib/trace/trace-fetch';

	const WINDOWS = [7, 30, 90] as const;
	type Window = (typeof WINDOWS)[number];
	const WRITE_TYPES = ['note', 'fact', 'decision', 'event', 'correction', 'pattern'] as const;

	interface Intention {
		id: string;
		description: string;
		deadline: string | null;
		priority: string;
		status: string;
		isOverdue: boolean;
		createdAt: string;
	}
	interface ChangeEvent {
		type: string;
		opType?: string;
		reason?: string;
		timestamp: string;
		affectedIdsPreview?: string[];
		affectedCount?: number;
		status?: string;
	}

	let days = $state<Window>(7);
	let writes = $state<TraceMemory[]>([]);
	let windowTotal = $state(0);
	let windowComplete = $state(true);
	/** The river holds at most this many writes: the newest ones. */
	const RIVER_MAX = 1100;
	let intentions = $state<Intention[]>([]);
	let changes = $state<ChangeEvent[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let nowMs = $state(Date.now());

	let pickedId = $state<string | null>(null);
	let verify = $state<VerifyState>('idle');
	let focusLinks = $state<TraceLink[]>([]);

	let draft = $state('');
	let draftType = $state<(typeof WRITE_TYPES)[number]>('note');
	let draftTags = $state('');
	let writing = $state(false);
	let wrote = $state<{ id: string; receiptId: string | null } | null>(null);
	let score = $state<{ composite: number; summary: string } | null>(null);
	let writeError = $state<string | null>(null);

	let intentDraft = $state('');
	let intentDue = $state('');
	let intentNote = $state<string | null>(null);
	let confirmCancel = $state(false);

	let titleEl = $state<HTMLElement | null>(null);

	/** Intentions with a deadline inside the window, as lights on the river. */
	const intentionLights = $derived.by(() => {
		const ahead = nowMs + days * 0.5 * 86_400_000;
		const start = nowMs - days * 86_400_000;
		return intentions
			.filter((i) => i.deadline && Date.parse(i.deadline) >= start && Date.parse(i.deadline) <= ahead)
			.map<TraceMemory>((i) => ({
				id: i.id,
				type: 'intention',
				content: i.description,
				createdAt: i.deadline!,
				retention: i.priority === 'critical' ? 1 : i.priority === 'high' ? 0.85 : 0.65,
				tags: [],
				future: Date.parse(i.deadline!) > nowMs
			}));
	});
	/**
	 * The ledger book: one clock per day (one per week past seven days),
	 * newest first, every write at the hour it was made; what is still to
	 * come waits on a clock of its own, ahead of them all.
	 */
	const byWeek = $derived(days > 7);
	function rowOf(ms: number): { label: string; lane: number } {
		const d = new Date(ms);
		d.setHours(0, 0, 0, 0);
		const today = new Date(nowMs);
		today.setHours(0, 0, 0, 0);
		if (!byWeek) {
			const back = Math.round((today.getTime() - d.getTime()) / 86_400_000);
			const label = back === 0 ? 'today' : back === 1 ? 'yesterday' : d.toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric' });
			return { label, lane: -back };
		}
		const monday = (x: Date) => new Date(x.getTime() - ((x.getDay() + 6) % 7) * 86_400_000);
		const back = Math.round((monday(today).getTime() - monday(d).getTime()) / (7 * 86_400_000));
		const label = back === 0 ? 'this week' : back === 1 ? 'last week' : `week of ${monday(d).toLocaleDateString([], { month: 'short', day: 'numeric' })}`;
		return { label, lane: -back };
	}
	const river = $derived([
		...writes.map((m) => ({ ...m, ...laneFor(m) })),
		...intentionLights.map((m) => (m.future ? { ...m, laneLabel: 'ahead', lane: 1 } : { ...m, ...laneFor(m) }))
	]);
	function laneFor(m: TraceMemory): { laneLabel: string; lane: number } {
		const r = rowOf(Date.parse(m.createdAt));
		return { laneLabel: r.label, lane: r.lane };
	}
	const range = $derived({ start: nowMs - days * 86_400_000, end: nowMs + days * 0.5 * 86_400_000 });
	const byId = $derived(new Map(river.map((m) => [m.id, m])));
	const intentionById = $derived(new Map(intentions.map((i) => [i.id, i])));
	const picked = $derived(pickedId ? (byId.get(pickedId) ?? null) : null);
	const pickedIntention = $derived(pickedId ? (intentionById.get(pickedId) ?? null) : null);

	async function load() {
		loading = true;
		error = null;
		nowMs = Date.now();
		const start = new Date(nowMs - days * 86_400_000).toISOString();
		const end = new Date(nowMs + 60_000).toISOString();
		try {
			const [tl, it, cl] = await Promise.all([
				fetchWindow(new Date(start), new Date(end), 6),
				api.tool<{ intentions?: Intention[] }>('intention', { action: 'list', filter_status: 'active', limit: 200 }).catch(() => null),
				api.tool<{ events?: ChangeEvent[] }>('memory_status', { view: 'changelog', limit: 40 }).catch(() => null)
			]);
			writes = tl.memories.slice(-RIVER_MAX);
			windowTotal = tl.memories.length;
			windowComplete = tl.complete;
			intentions = it?.intentions ?? [];
			changes = cl?.events ?? [];
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
		}
	}

	function pick(id: string) {
		pickedId = id;
		confirmCancel = false;
		intentNote = null;
		focusLinks = [];
		verify = 'idle';
	}

	/** A connected memory: fly to it when it is on this river, else open it in Trace. */
	function focusPick(id: string) {
		if (byId.has(id)) pick(id);
		else void goto(osHref('/trace', { q: id }));
	}

	async function record() {
		const content = draft.trim();
		if (!content) return;
		writing = true;
		writeError = null;
		try {
			const tags = draftTags
				.split(',')
				.map((t) => t.trim())
				.filter(Boolean);
			const res = await api.tool<{ nodeId?: string; receiptId?: string }>('smart_ingest', {
				content,
				node_type: draftType,
				tags,
				source: 'Vestige dashboard · Ledger'
			});
			wrote = { id: res.nodeId ?? '', receiptId: res.receiptId ?? null };
			draft = '';
			await load();
			if (res.nodeId) void pick(res.nodeId);
		} catch (e) {
			writeError = e instanceof Error ? e.message : String(e);
		} finally {
			writing = false;
		}
	}

	/** How much a draft would matter, before it is written (maintain importance_score). */
	async function scoreDraft() {
		const content = draft.trim();
		if (!content) return;
		try {
			const res = await api.tool<{ composite?: number; summary?: string }>('maintain', { action: 'importance_score', content });
			score = { composite: res.composite ?? 0, summary: res.summary ?? '' };
		} catch (e) {
			score = { composite: 0, summary: e instanceof Error ? e.message : String(e) };
		}
	}

	async function remind() {
		const description = intentDraft.trim();
		if (!description) return;
		intentNote = null;
		try {
			const args: Record<string, unknown> = { action: 'set', description, priority: 'normal' };
			if (intentDue) args.deadline = new Date(intentDue).toISOString();
			const res = await api.tool<{ id?: string; receiptId?: string; intention?: { id?: string } }>('intention', args);
			intentNote = `Set · receipt ${res.receiptId ?? 'not returned'}`;
			intentDraft = '';
			intentDue = '';
			await load();
		} catch (e) {
			intentNote = e instanceof Error ? e.message : String(e);
		}
	}

	async function settle(status: 'complete' | 'snooze' | 'cancel') {
		if (!pickedIntention) return;
		if (status === 'cancel' && !confirmCancel) {
			confirmCancel = true;
			return;
		}
		try {
			const args: Record<string, unknown> = { action: 'update', id: pickedIntention.id, status };
			if (status === 'snooze') args.snooze_minutes = 1440;
			const res = await api.tool<{ receiptId?: string }>('intention', args);
			intentNote = `${status === 'complete' ? 'Completed' : status === 'snooze' ? 'Snoozed a day' : 'Cancelled'} · receipt ${res.receiptId ?? 'not returned'}`;
			confirmCancel = false;
			pickedId = null;
			await load();
		} catch (e) {
			intentNote = e instanceof Error ? e.message : String(e);
		}
	}

	function describe(id: string): StageWords | null {
		const m = byId.get(id);
		if (!m) return null;
		if (m.type === 'intention') {
			const i = intentionById.get(id);
			return { kicker: `intention · due ${when(m.createdAt)}${i?.isOverdue ? ' · overdue' : ''}`, text: m.content, tone: 'intention' };
		}
		return { kicker: `${m.type} · ${when(m.createdAt)}`, text: m.content, tone: m.type };
	}

	const labelIds = $derived.by(() => {
		if (pickedId) return [pickedId];
		const latest = writes.at(-1)?.id;
		const next = intentionLights.filter((m) => m.future).sort((a, b) => Date.parse(a.createdAt) - Date.parse(b.createdAt))[0]?.id;
		return [latest, next].filter((x): x is string => !!x);
	});

	const typeCounts = $derived.by(() => {
		const c = new Map<string, number>();
		for (const m of writes) c.set(m.type, (c.get(m.type) ?? 0) + 1);
		return [...c.entries()].sort((a, b) => b[1] - a[1]);
	});

	function setWindow(d: Window) {
		if (d === days) return;
		days = d;
		pickedId = null;
		void load();
	}

	const rgb = (t: string) => {
		const [r, g, b] = typeColor(t);
		return `rgb(${Math.round(r * 255)} ${Math.round(g * 255)} ${Math.round(b * 255)})`;
	};
	function when(iso: string): string {
		const d = new Date(iso);
		if (!Number.isFinite(d.getTime())) return '';
		const sameDay = new Date().toDateString() === d.toDateString();
		return sameDay
			? d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
			: d.toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
	}

	onMount(() => void load());
</script>

<svelte:head>
	<title>Ledger · Vestige</title>
</svelte:head>

<OrganStage
	memories={river}
	{range}
	now={nowMs}
	selectedId={pickedId}
	links={pickedId ? focusLinks : []}
	{verify}
	origin={() => {
		const r = titleEl?.getBoundingClientRect();
		return r ? { x: ((r.left + 160) / innerWidth) * 2 - 1, y: 1 - ((r.top + r.height / 2) / innerHeight) * 2 } : undefined;
	}}
	{describe}
	{labelIds}
	onpick={(id) => void pick(id)}
	story="ledger"
	seed="vestige-ledger-v1"
	axis={byWeek ? 'week' : 'day'}
/>

<header class="tr-ask" class:tr-quiet={!!pickedId}>
	<p class="tr-eyebrow" use:alive={{ text: 'Ledger', glow: true }}></p>
	<!-- svelte-ignore a11y_missing_content (alive renders the text and its aria-label) -->
	<h1 class="tr-title" bind:this={titleEl} use:alive={{ text: 'Every write, in order, with its proof.', glow: true }}></h1>
	<p class="tr-hint">
		{#each WINDOWS as d (d)}
			<button type="button" class="tr-chip" aria-pressed={days === d} onclick={() => setWindow(d)}>{d} days</button>
		{/each}
		<span class="tr-hint-text">a clock per {byWeek ? 'week' : 'day'} · {byWeek ? 'Monday' : 'midnight'} at the top</span>
	</p>
	{#if loading}
		<p class="tr-line">Reading the record…</p>
	{:else if error}
		<p class="tr-line tr-err">{error}</p>
	{:else}
		<p class="tr-line">
			<strong>{windowComplete ? '' : 'at least '}{windowTotal.toLocaleString()}</strong>
			writes in the last {days} days{windowTotal > writes.length ? `, the newest ${writes.length.toLocaleString()} shown` : ''}
			{#if intentionLights.length}· <strong use:alive={{ text: String(intentionLights.length), count: true }}></strong> intentions in reach{/if}
		</p>
		{#if typeCounts.length}
			<p class="tr-types">
				{#each typeCounts.slice(0, 6) as [t, n] (t)}
					<span><i style:background={rgb(t)}></i>{t} {n}</span>
				{/each}
				{#if intentionLights.length}<span><i style:background={rgb('intention')}></i>intention {intentionLights.length}</span>{/if}
			</p>
		{/if}
		<p class="tr-nudge">
			Each clock is a {byWeek ? 'week' : 'day'}, each orb a write at the {byWeek ? 'moment of its week' : 'hour of its day'}, {byWeek ? 'Monday' : 'midnight'} at the top. Click a clock to open every write in it; click a write to go inside it and re-verify its receipt.
		</p>
	{/if}
</header>

<aside class="tr-lane" class:idle={!pickedId} aria-live="polite">
	{#if pickedIntention}
		<p class="tr-kicker" style:color={rgb('intention')}>
			intention · {pickedIntention.priority} · due {pickedIntention.deadline ? when(pickedIntention.deadline) : 'whenever'}
			{#if pickedIntention.isOverdue}<span class="tr-warn"> · overdue</span>{/if}
		</p>
		<p class="tr-text" use:alive={{ text: headline(pickedIntention.description, 480) }}></p>
		<p class="tr-id">{pickedIntention.id}</p>
		<div class="tr-actions">
			<button type="button" class="tr-act alt" onclick={() => void settle('complete')}>Mark done</button>
			<button type="button" class="tr-act quiet" onclick={() => void settle('snooze')}>Snooze a day</button>
			<button type="button" class="tr-act danger" onclick={() => void settle('cancel')}>
				{confirmCancel ? 'Cancel it for good?' : 'Cancel'}
			</button>
		</div>
		{#if intentNote}<p class="tr-note">{intentNote}</p>{/if}
		<div class="tr-actions"><button type="button" class="tr-act quiet" onclick={() => (pickedId = null)}>Back to the record</button></div>
	{:else if picked}
		<MemoryFocus
			id={picked.id}
			onpick={focusPick}
			onlinks={(l) => (focusLinks = l)}
			onverify={(v) => (verify = v)}
			onclose={() => (pickedId = null)}
		/>
	{:else}
		<p class="tr-proof-kicker" use:alive={{ text: 'What changed lately' }}></p>
		{#each changes.slice(0, 12) as c (c.timestamp + (c.affectedIdsPreview?.[0] ?? '') + c.type)}
			{@const first = c.affectedIdsPreview?.[0]}
			<button
				type="button"
				class="tr-item"
				data-trace-id={first}
				disabled={!first}
				onclick={() => first && (byId.has(first) ? void pick(first) : (location.href = osHref('/trace', { q: first })))}
			>
				<span class="tr-edge causal">{c.opType ?? c.type}</span>
				<span>{first && byId.get(first) ? headline(byId.get(first)!.content, 96) : (c.reason ?? c.type)}</span>
				<span class="tr-tag">{when(c.timestamp)}{c.affectedCount ? ` · ${c.affectedCount} memor${c.affectedCount === 1 ? 'y' : 'ies'}` : ''}{first ? ` · ${first}` : ''}</span>
			</button>
		{:else}
			<p class="tr-note">{loading ? '' : 'No state changes recorded yet.'}</p>
		{/each}

		<div class="tr-block">
			<p class="tr-proof-kicker" use:alive={{ text: 'Write to the ledger' }}></p>
			<textarea class="tr-field" rows="3" bind:value={draft} placeholder="What happened, why, and the evidence"></textarea>
			<p class="tr-hint">
				{#each WRITE_TYPES as t (t)}
					<button type="button" class="tr-chip" aria-pressed={draftType === t} onclick={() => (draftType = t)}>{t}</button>
				{/each}
			</p>
			<input class="tr-field" bind:value={draftTags} placeholder="exact tags, comma separated" spellcheck="false" />
			<div class="tr-actions">
				<button type="button" class="tr-act alt" disabled={writing || !draft.trim()} onclick={() => void record()}>
					{writing ? 'Passing the gate…' : 'Record through the gate'}
				</button>
				<button type="button" class="tr-act quiet" disabled={!draft.trim()} onclick={() => void scoreDraft()}>Score it first</button>
			</div>
			{#if score}<p class="tr-note">{score.summary}</p>{/if}
			{#if writeError}<p class="tr-note tr-err">{writeError}</p>{/if}
			{#if wrote}<p class="tr-note">Recorded {wrote.id} · receipt {wrote.receiptId ?? 'not returned'}</p>{/if}
		</div>

		<div class="tr-block">
			<p class="tr-proof-kicker" use:alive={{ text: 'Remind me' }}></p>
			<input class="tr-field" bind:value={intentDraft} placeholder="What should come back, and when" />
			<input class="tr-field" type="datetime-local" bind:value={intentDue} aria-label="Deadline" />
			<div class="tr-actions">
				<button type="button" class="tr-act" disabled={!intentDraft.trim()} onclick={() => void remind()}>Set the intention</button>
			</div>
			{#if intentNote}<p class="tr-note">{intentNote}</p>{/if}
		</div>
	{/if}
</aside>
