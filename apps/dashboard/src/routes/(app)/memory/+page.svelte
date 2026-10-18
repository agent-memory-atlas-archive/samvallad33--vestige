<script lang="ts">
	/**
	 * One memory, inside.
	 *
	 * Opened by a click on any memory in an organ (the view zooms into it and
	 * its particles carry over), by walking to a neighbour, or by an agent that
	 * was asked about this memory. The left of the screen is the memory itself
	 * (lib/inside: its own form, its receipt ring, a moon per neighbour along
	 * each recorded edge); the right is its words, read live (MemoryFocus).
	 * The two never overlap. Everything drawn can be clicked: a moon walks into
	 * that memory, the form or the ring re-verifies the receipt now.
	 */
	import '$lib/trace/trace-organ.css';
	import { onDestroy, onMount } from 'svelte';
	import { page } from '$app/stores';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import ObservatoryCanvas from '$lib/components/ObservatoryCanvas.svelte';
	import type { ObservatoryEngine } from '$lib/observatory/engine';
	import MemoryFocus from '$lib/trace/MemoryFocus.svelte';
	import { alive } from '$lib/trace/alive';
	import { cursorHot } from '$lib/trace/cursor';
	import { markVeilReady, takeHandoff } from '$lib/trace/handoff';
	import { edgeColor } from '$lib/trace/trace-pass';
	import { headline } from '$lib/trace/trace-data';
	import { createInsidePass, insideRegion, type InsideMoon, type InsidePass } from '$lib/inside/inside-pass';
	import { agentVoice } from '$lib/inside/agent-voice';

	const id = $derived($page.url.searchParams.get('id') ?? '');

	let pass = $state<InsidePass | null>(null);
	let stage = $state<HTMLDivElement | null>(null);
	let arrivedId = '';
	let veilArmed = false;
	let proofNonce = $state(0);
	let hover = $state<{ kind: 'moon'; index: number } | { kind: 'form' } | { kind: 'ring' } | null>(null);
	let hoverLabel = $state<{ x: number; y: number; right: boolean; kicker: string; text: string; tone: string } | null>(null);
	let moons = $state<InsideMoon[]>([]);
	// The window's shape decides the layout: words beside the memory on a wide
	// screen, under it on a phone (the visual's region follows the same rule).
	let aspect = $state(16 / 10);
	onMount(() => {
		const set = () => (aspect = innerWidth / Math.max(1, innerHeight));
		set();
		addEventListener('resize', set);
		return () => removeEventListener('resize', set);
	});

	function onready(e: ObservatoryEngine) {
		pass?.dispose();
		const p = createInsidePass(e);
		e.addPass(p);
		pass = p;
		const h = takeHandoff();
		if (h) {
			p.beginHandoff(h.points, h.count);
			veilArmed = true;
		}
		arrivedId = '';
		arriveIfNew();
	}

	function arriveIfNew() {
		if (!pass || !id || id === arrivedId) return;
		arrivedId = id;
		moons = [];
		hover = null;
		hoverLabel = null;
		pass.arrive(id);
		// The words can load before the GPU is up: apply whatever already arrived.
		if (seen.node?.id === id) applyNode(seen.node);
		if (seen.neighboursFor === id && seen.neighbours) applyNeighbours(seen.neighbours);
		if (seen.verifyFor === id) pass.setVerify(seen.verify);
	}
	$effect(() => {
		void id;
		arriveIfNew();
	});

	function onframe() {
		if (veilArmed) {
			veilArmed = false;
			markVeilReady();
		}
	}

	onDestroy(() => {
		cursorHot.set(false);
		pass?.dispose();
	});

	// ── The words drive the visual ──────────────────────────────────────────
	type NodeData = { id: string; nodeType?: string; content?: string; retentionStrength?: number; stability?: number };
	type NeighbourData = { id: string; label: string; type: string; edge: string; outgoing: boolean };
	const seen: {
		node: NodeData | null;
		neighbours: NeighbourData[] | null;
		neighboursFor: string;
		verify: 'idle' | 'running' | 'ok' | 'fail';
		verifyFor: string;
	} = { node: null, neighbours: null, neighboursFor: '', verify: 'idle', verifyFor: '' };
	function onnode(n: NodeData) {
		if (n.id !== id) return;
		seen.node = n;
		if (pass && arrivedId === id) applyNode(n);
	}
	function applyNode(n: NodeData) {
		pass?.setMemory({
			id: n.id,
			type: n.nodeType ?? '',
			retention: n.retentionStrength ?? 0.8,
			stability: n.stability,
			length: n.content?.length ?? 0
		});
	}
	function onneighbours(list: NeighbourData[]) {
		seen.neighbours = list;
		seen.neighboursFor = id;
		if (pass && arrivedId === id) applyNeighbours(list);
	}
	function applyNeighbours(list: NeighbourData[]) {
		pass?.setMoons(list.map((n) => ({ id: n.id, type: n.type, edge: n.edge, outgoing: n.outgoing, label: n.label })));
		moons = pass?.moons() ?? [];
	}
	function onverify(v: 'idle' | 'running' | 'ok' | 'fail') {
		seen.verify = v;
		seen.verifyFor = id;
		if (pass && arrivedId === id) pass.setVerify(v);
	}

	/** Walk into a neighbour: its moon is where the next memory assembles from. */
	function walkTo(nextId: string) {
		if (!pass || !nextId || nextId === id) return;
		const i = moons.findIndex((m) => m.id === nextId);
		const at = i >= 0 ? pass.moonNdc(i) : null;
		if (at) pass.arriveFrom(at);
		void goto(`${base}/memory?id=${encodeURIComponent(nextId)}`, { keepFocus: true, noScroll: true });
	}

	// ── Pointer: everything drawn answers ──────────────────────────────────
	function ndcOf(e: PointerEvent) {
		if (!stage) return null;
		const r = stage.getBoundingClientRect();
		return { x: ((e.clientX - r.left) / r.width) * 2 - 1, y: 1 - ((e.clientY - r.top) / r.height) * 2 };
	}
	function onMove(e: PointerEvent) {
		const ndc = ndcOf(e);
		if (!ndc || !pass) return;
		pass.setCursor(ndc);
		const hit = pass.pickAt(ndc.x, ndc.y);
		hover = hit?.kind === 'moon' ? { kind: 'moon', index: hit.index } : hit ? { kind: hit.kind } : null;
		pass.setHover(hit?.kind === 'moon' ? hit.index : hit?.kind === 'form' || hit?.kind === 'ring' ? 'form' : null);
		cursorHot.set(!!hit);
		hoverLabel = labelFor(hit);
	}
	function onLeave() {
		pass?.setCursor(null);
		pass?.setHover(null);
		hover = null;
		hoverLabel = null;
		cursorHot.set(false);
	}
	function onClick(e: MouseEvent) {
		const ndc = ndcOf(e as PointerEvent);
		if (!ndc || !pass) return;
		const hit = pass.pickAt(ndc.x, ndc.y);
		if (!hit) return;
		if (hit.kind === 'moon') walkTo(hit.id);
		else proofNonce++;
	}

	const rgb = ([r, g, b]: [number, number, number]) => `rgb(${Math.round(r * 255)} ${Math.round(g * 255)} ${Math.round(b * 255)})`;
	const EDGE_WORDS: Record<string, [string, string]> = {
		derived_from: ['derived from', 'gave rise to'],
		evidence_of: ['is evidence of', 'is backed by'],
		closes: ['closes', 'is closed by'],
		closed_by: ['was closed by', 'closed'],
		supersedes: ['supersedes', 'was superseded by'],
		corrects: ['corrects', 'was corrected by'],
		contradicts: ['contradicts', 'is contradicted by'],
		touched: ['shares a handle with', 'shares a handle with']
	};
	/** A label for what is under the pointer, placed beside it and inside the visual's own region. */
	function labelFor(hit: ReturnType<InsidePass['pickAt']>) {
		if (!hit || !pass || !stage) return null;
		const r = stage.getBoundingClientRect();
		const region = insideRegion(r.width / Math.max(1, r.height));
		const regionRight = ((region.right + 1) / 2) * r.width;
		const toPx = (x: number, y: number) => ({ x: ((x + 1) / 2) * r.width, y: ((1 - y) / 2) * r.height });
		if (hit.kind === 'moon') {
			const m = moons[hit.index];
			const at = pass.moonNdc(hit.index);
			if (!m || !at) return null;
			const p = toPx(at.x, at.y);
			const gap = 26;
			const right = p.x + gap + 240 < regionRight;
			const words = EDGE_WORDS[m.edge] ?? [m.edge, m.edge];
			return {
				x: right ? p.x + gap : p.x - gap,
				y: p.y,
				right,
				kicker: `${words[m.outgoing ? 0 : 1]} · ${m.type}`,
				text: headline(m.label, 90),
				tone: rgb(edgeColor(m.edge))
			};
		}
		const at = hit.kind === 'ring' ? pass.ringNdc() : pass.formNdc();
		const p = toPx(at.x, at.y);
		const ry = ((at.r / 1) * r.height) / 2;
		return {
			x: p.x,
			y: p.y + (hit.kind === 'ring' ? ry : ry * 0.62) + 26,
			right: true,
			kicker: hit.kind === 'ring' ? 'the receipt' : 'this memory',
			text: 'Click to re-verify it against the hash-chained log, now.',
			tone: '#f6c35b'
		};
	}

	const voice = $derived($agentVoice && $agentVoice.id === id ? $agentVoice : null);
	const back = () => (history.length > 1 ? history.back() : void goto(`${base}/trace`));
</script>

<svelte:head>
	<title>Memory {id.slice(-6)} · Vestige</title>
</svelte:head>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="stage" bind:this={stage} onpointermove={onMove} onpointerleave={onLeave} onclick={onClick}>
	<ObservatoryCanvas demo="recall-path" seed="vestige-inside-v1" maxDpr={2} {onready} {onframe} />
</div>

<button type="button" class="back" onclick={back}>← Back</button>

{#if hoverLabel}
	<div class="hover" class:left={!hoverLabel.right} style:left="{hoverLabel.x}px" style:top="{hoverLabel.y}px">
		<span class="hover-kicker" style:color={hoverLabel.tone}>{hoverLabel.kicker}</span>
		<span class="hover-text">{hoverLabel.text}</span>
	</div>
{/if}

<aside class="panel" class:portrait={aspect < 1}>
	{#if voice}
		<div class="voice">
			<p class="tr-proof-kicker" use:alive={{ text: `${voice.by} is explaining this memory` }}></p>
			<p class="voice-text" use:alive={{ text: voice.text }}></p>
		</div>
	{/if}
	{#if id}
		<MemoryFocus
			{id}
			bare
			{proofNonce}
			onpick={walkTo}
			{onnode}
			{onneighbours}
			{onverify}
			onhoverneighbour={(nid) => {
				const i = nid ? moons.findIndex((m) => m.id === nid) : -1;
				pass?.setHover(i >= 0 ? i : null);
			}}
		/>
	{:else}
		<p class="tr-proof-line">Open a memory by its exact id: /memory?id=mem-…</p>
	{/if}
</aside>

<style>
	.stage {
		position: fixed;
		inset: 0;
		background: #05060a;
	}
	.back {
		position: fixed;
		top: 1.2rem;
		left: max(1rem, min(6.2rem, 8vw));
		z-index: 30;
		background: none;
		border: 0;
		padding: 0.2rem 0;
		font-family: ui-monospace, 'SF Mono', Menlo, monospace;
		font-size: 0.68rem;
		letter-spacing: 0.16em;
		text-transform: uppercase;
		color: #7fa8a3;
	}
	.back:hover {
		color: #e8fffb;
	}
	/* The words own the right of the screen (the bottom on a phone); the
	   visual never draws there (lib/inside insideRegion). */
	.panel {
		position: fixed;
		z-index: 20;
		top: 1.5rem;
		right: 1.8rem;
		bottom: 1.5rem;
		width: min(30rem, 36vw);
		overflow-y: auto;
		scrollbar-width: none;
		padding-left: 1.1rem;
		border-left: 1px solid rgba(127, 243, 230, 0.14);
		color: #e8f4f2;
		text-shadow: 0 0 22px rgba(2, 3, 7, 0.95), 0 1px 2px rgba(2, 3, 7, 0.9);
	}
	.panel.portrait {
		top: 52vh;
		left: 1rem;
		right: 1rem;
		width: auto;
		border-left: 0;
		padding-left: 0;
		border-top: 1px solid rgba(127, 243, 230, 0.14);
		padding-top: 0.8rem;
	}
	.voice {
		margin-bottom: 1.4rem;
		padding-bottom: 1rem;
		border-bottom: 1px solid rgba(246, 195, 91, 0.18);
	}
	.voice-text {
		margin: 0.4rem 0 0;
		font-size: 0.9rem;
		line-height: 1.6;
		color: #f4ecd8;
	}
	.hover {
		position: fixed;
		z-index: 25;
		max-width: 15rem;
		transform: translateY(-50%);
		pointer-events: none;
		display: flex;
		flex-direction: column;
		gap: 0.2rem;
		text-shadow: 0 0 18px rgba(2, 3, 7, 0.98), 0 1px 2px rgba(2, 3, 7, 0.9);
	}
	.hover.left {
		transform: translate(-100%, -50%);
		text-align: right;
	}
	.hover-kicker {
		font-family: ui-monospace, 'SF Mono', Menlo, monospace;
		font-size: 0.58rem;
		letter-spacing: 0.18em;
		text-transform: uppercase;
	}
	.hover-text {
		font-size: 0.76rem;
		line-height: 1.45;
		color: #e4f1ef;
	}
</style>
