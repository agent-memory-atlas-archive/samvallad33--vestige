<script lang="ts">
	/**
	 * TraceStage — the shared instrument every organ looks through.
	 *
	 * A full-bleed WebGPU field (createTracePass): every group of the answer
	 * is one big round island, its memories big glowing orbs on it, newest at
	 * its heart. Recorded links arc between them, a causal walk rings a picked
	 * memory, and the events (answer, pick, receipt check, walk, cursor) are
	 * forces in the simulation. Hovering an orb shows that memory in a card
	 * beside it; a click on an island opens it to every memory in it, and a
	 * click on an orb flies the camera in and opens the memory's own page.
	 * Words stay on the page around it; the stage only adds the islands'
	 * names, an opened island's heading and the hover card.
	 */
	import { onDestroy, onMount, untrack } from 'svelte';
	import { afterNavigate, preloadCode, replaceState } from '$app/navigation';
	import { base } from '$app/paths';
	import { cursorHot as cursorHotStore } from './cursor';
	import { markVeilReady, takeHandoff } from './handoff';
	import { enterMemory } from '$lib/inside/enter';
	import ObservatoryCanvas from '$lib/components/ObservatoryCanvas.svelte';
	import HoverCard from './HoverCard.svelte';
	import type { ObservatoryEngine } from '$lib/observatory/engine';
	import { headline, type TraceLink, type TraceMemory, type WalkResult } from './trace-data';
	import {
		createTracePass,
		edgeColor,
		typeColor,
		type Axis,
		type Lane,
		type RowZoom,
		type Story,
		type TimeTick,
		type TracePass,
		type VerifyState
	} from './trace-pass';

	export interface StageWords {
		kicker: string;
		text: string;
		/** Colour key for the kicker: a memory type or an edge type. */
		tone?: string;
	}

	interface Props {
		memories: TraceMemory[];
		range?: { start: number; end: number } | null;
		links?: TraceLink[];
		walk?: WalkResult | null;
		selectedId?: string | null;
		verify?: VerifyState;
		now?: number | null;
		/** Where a new answer erupts from, in NDC (called when the answer changes). */
		origin?: () => { x: number; y: number } | undefined;
		/** Words for a memory on the stage (hover and labels). */
		describe?: (id: string) => StageWords | null;
		/** Memories to name on the scene itself, in priority order. */
		labelIds?: string[];
		onpick?: (id: string, kind: 'answer' | 'walk') => void;
		seed?: string;
		/** Hide the particle/GPU readout (it is shown by default). */
		quiet?: boolean;
		/** 'time' by the minute written (default); 'order' evenly spaced in that order. */
		axis?: Axis;
		/** The organ's story: how its answer arrives and keeps moving. */
		story?: Story;
		/**
		 * A click on a memory zooms into it and opens its own page (default).
		 * Off, the click is only reported through onpick. 'second': the first
		 * click picks the memory (onpick, the camera flies in and the page
		 * shows what it needs to); a click on the picked memory, or on one the
		 * walk brought up, goes inside it.
		 */
		enter?: boolean | 'second';
	}

	let {
		memories,
		range = null,
		links = [],
		walk = null,
		selectedId = null,
		verify = 'idle',
		now = null,
		origin,
		describe,
		labelIds = [],
		onpick,
		seed = 'vestige-trace-v1',
		quiet = false,
		axis = 'time',
		story = 'trace',
		enter = true
	}: Props = $props();

	// River, memory light and edge streams (trace-pass RIVER + MEM_POOL + MAX_EDGES * PER_EDGE).
	const PARTICLES = 48_000 + 36_000 + 256 * 64;
	/** The islands start this far right of the stage's left edge, clear of the dock. */
	const DOCK_DESKTOP = 88;
	const DOCK_PHONE = 8;

	let engine: ObservatoryEngine | null = null;
	let pass = $state<TracePass | null>(null);
	let stage = $state<HTMLDivElement | null>(null);
	let ticks = $state<{ ticks: TimeTick[]; floorY: number }>({ ticks: [], floorY: -0.7 });
	let fps = $state(0);
	let gpuMs = $state<number | null>(null);
	interface HoverState {
		id: string;
		/** The marker on screen (px in the stage). */
		x: number;
		y: number;
		/** The card opens to the left when the marker is near the right edge. */
		left: boolean;
		words: StageWords;
		/** A pill of memories written together: which one, of how many. */
		member: number;
		members: number;
		/** A click on the river opens the memory nearest in time: say so. */
		near: boolean;
		/** The pill's memories, top to bottom. */
		cluster: string[] | null;
	}
	let hover = $state<HoverState | null>(null);
	let lanes = $state<Lane[]>([]);
	let phone = $state(false);
	let laneKey = '';
	let preloaded = false;

	// ── Islands: a click on one opens it ─────────────────────────────────────
	// The opened island is part of the page's address (?row=), so the user and
	// any agent they work with can open exactly the same view.
	let row = $state<string | null>(null);
	let zoom = $state<RowZoom | null>(null);
	const keyOfMemory = (m: TraceMemory) => m.laneLabel ?? m.type;
	const rowMembers = $derived(
		row
			? memories.filter((m) => keyOfMemory(m) === row).sort((a, b) => Date.parse(a.createdAt) - Date.parse(b.createdAt))
			: []
	);
	const ROW_LIST_MAX = 400;
	function openRow(key: string | null) {
		row = key;
		hover = null;
		pass?.setFocusLane(key);
		const u = new URL(location.href);
		if (key) u.searchParams.set('row', key);
		else u.searchParams.delete('row');
		if (u.href !== location.href) replaceState(u, {});
	}
	afterNavigate(() => {
		const r = new URL(location.href).searchParams.get('row');
		if (r !== row) {
			row = r;
			pass?.setFocusLane(r);
		}
	});
	// The page's own reading lane steps aside while a zoomed row lists its memories.
	const rowPanel = $derived(!!zoom && !selectedId);
	$effect(() => {
		const on = rowPanel;
		document.documentElement.dataset.rowPanel = on ? '1' : '';
		return () => {
			delete document.documentElement.dataset.rowPanel;
		};
	});
	onMount(() => {
		const onKey = (e: KeyboardEvent) => {
			if (e.key === 'Escape' && row && !selectedId) openRow(null);
		};
		window.addEventListener('keydown', onKey);
		return () => window.removeEventListener('keydown', onKey);
	});

	let cursorHot = $state(false);
	// The one root-mounted cursor opens into its ring over a memory.
	$effect(() => cursorHotStore.set(cursorHot));
	$effect(() => () => cursorHotStore.set(false));

	// ── A thread from any memory named in the page's words to its light ─────
	let tether = $state<{ d: string; tone: string } | null>(null);
	let tetherId: string | null = null;
	let tetherEl: HTMLElement | null = null;
	function tetherPath(): { d: string; tone: string } | null {
		if (!pass || !tetherId || !tetherEl || !stage) return null;
		const at = pass.ndcOf(tetherId);
		if (!at) return null;
		const r = stage.getBoundingClientRect();
		const e = tetherEl.getBoundingClientRect();
		const tx = ((at.x + 1) / 2) * r.width;
		const ty = ((1 - at.y) / 2) * r.height;
		// Leave from the side of the words that faces the light.
		const fromRight = e.left + e.width / 2 < tx;
		const sx = (fromRight ? e.right + 6 : e.left - 6) - r.left;
		const sy = e.top + Math.min(e.height / 2, 14) - r.top;
		const mx = (sx + tx) / 2;
		const d = `M ${sx} ${sy} C ${mx} ${sy}, ${mx} ${ty}, ${tx} ${ty}`;
		return { d, tone: describe?.(tetherId)?.tone ?? '' };
	}
	function onDocOver(e: PointerEvent) {
		const el = (e.target as Element | null)?.closest?.('[data-trace-id]') as HTMLElement | null;
		const id = el?.dataset.traceId ?? null;
		if (id === tetherId) return;
		tetherId = id;
		tetherEl = el;
		pass?.setHover(id);
		tether = tetherPath();
	}
	onMount(() => {
		document.addEventListener('pointerover', onDocOver);
		// Every movement reaches the field, over the words too, not only
		// where the canvas is uncovered.
		const onDocMove = (e: PointerEvent) => {
			if (!stage || !pass) return;
			const r = stage.getBoundingClientRect();
			pass.setCursor({ x: ((e.clientX - r.left) / r.width) * 2 - 1, y: 1 - ((e.clientY - r.top) / r.height) * 2 });
		};
		const onDocOut = (e: PointerEvent) => {
			if (e.relatedTarget) return;
			pass?.setCursor(null);
		};
		window.addEventListener('pointermove', onDocMove, { passive: true });
		window.addEventListener('pointerout', onDocOut);
		return () => {
			window.removeEventListener('pointermove', onDocMove);
			window.removeEventListener('pointerout', onDocOut);
		};
	});
	onDestroy(() => {
		if (typeof document !== 'undefined') document.removeEventListener('pointerover', onDocOver);
	});

	function onready(e: ObservatoryEngine) {
		engine = e;
		pass?.dispose();
		const p = createTracePass(e);
		e.addPass(p);
		if (import.meta.env.DEV) (window as unknown as { __trace?: TracePass }).__trace = p;
		p.setAxis(axis);
		p.setStory(story);
		p.setFocusLane(row);
		// Arriving from a Palace click: start on the particles that were on
		// screen there, then let this organ's story reorganize them.
		const h = takeHandoff();
		if (h) {
			p.beginHandoff(h.points, h.count);
			handoffArmed = true;
		}
		p.setAnswer(memories, range, origin?.());
		p.setLinks(links);
		p.setNow(now);
		if (selectedId) p.setSelected(selectedId);
		if (walk) p.setWalk(walk);
		pass = p;
	}

	// One field, many organs: when another organ takes this stage over, its
	// story begins from the particles already on screen. This runs before the
	// answer effect below, so the new organ's first answer already continues.
	let storyShown: string | null = null;
	// The page's words take its universe's glow (trace-organ.css).
	$effect(() => {
		document.documentElement.dataset.universe = story;
		return () => {
			delete document.documentElement.dataset.universe;
		};
	});
	$effect(() => {
		const s = story;
		untrack(() => {
			const before = storyShown;
			storyShown = s;
			if (!pass) return;
			if (before !== null && before !== s) pass.continueNext();
			pass.setStory(s);
		});
	});

	// Keep the field in step with what the page holds. Each effect reads only
	// its own inputs, so a new walk never re-erupts the answer.
	$effect(() => {
		const m = memories;
		const r = range;
		untrack(() => pass?.setAnswer(m, r, origin?.()));
	});
	$effect(() => {
		const a = axis;
		untrack(() => pass?.setAxis(a));
	});
	$effect(() => {
		const l = links;
		untrack(() => pass?.setLinks(l));
	});
	$effect(() => {
		const n = now;
		untrack(() => pass?.setNow(n));
	});
	$effect(() => {
		const id = selectedId;
		untrack(() => pass?.setSelected(id));
	});
	$effect(() => {
		const w = walk;
		untrack(() => pass?.setWalk(w));
	});
	$effect(() => {
		const v = verify;
		untrack(() => pass?.setVerify(v));
	});

	let tickKey = '';
	let quietAt = 0;
	let lastAt = 0;
	let handoffArmed = false;
	function onframe(_frame: number, f: number) {
		if (handoffArmed) {
			// The handed-off picture is on screen: the dive's veil can lift.
			handoffArmed = false;
			markVeilReady();
		}
		const t = performance.now();
		// Fast enough that labels ride along while the camera flies.
		if (t - lastAt < 70 || !pass) return;
		lastAt = t;
		fps = f;
		gpuMs = engine?.gpuMs ?? null;
		const next = pass.timeTicks();
		const key = JSON.stringify(next);
		if (key !== tickKey) {
			tickKey = key;
			ticks = next;
		}
		// Words and light never share a place: the field learns where the
		// page's blocks of words are and keeps out (and quiet) there.
		if (t - quietAt > 500 && stage) {
			quietAt = t;
			const r = stage.getBoundingClientRect();
			const rects = [...document.querySelectorAll('.tr-ask, .tr-lane, .tr-beats, [data-quiet]')]
				.map((el) => el.getBoundingClientRect())
				.filter((b) => b.width > 8 && b.height > 8)
				.slice(0, 4)
				.map((b) => ({
					x0: ((b.left - r.left) / r.width) * 2 - 1,
					x1: ((b.right - r.left) / r.width) * 2 - 1,
					y0: 1 - ((b.bottom - r.top) / r.height) * 2,
					y1: 1 - ((b.top - r.top) / r.height) * 2
				}));
			pass.setQuiet(rects);
		}
		if (tetherId) tether = tetherPath();
		const z = pass.zoom();
		if (
			z?.key !== zoom?.key ||
			z?.count !== zoom?.count ||
			z?.page !== zoom?.page ||
			z?.pages !== zoom?.pages ||
			Math.abs((z?.top ?? 0) - (zoom?.top ?? 0)) > 0.002 ||
			Math.abs((z?.x ?? 0) - (zoom?.x ?? 0)) > 0.002
		)
			zoom = z;
		const nextLanes = pass.lanes();
		if (stage) {
			phone = stage.clientWidth < 768;
			pass.setInset(phone ? DOCK_PHONE : DOCK_DESKTOP);
		}
		const lk = JSON.stringify(nextLanes);
		if (lk !== laneKey) {
			laneKey = lk;
			lanes = nextLanes;
		}
		// The card rides with its marker while the camera moves.
		if (hover) placeCard(hover.id);
	}

	/** Put the card beside its marker (px in the stage). */
	function placeCard(id: string) {
		if (!hover || !pass || !stage) return;
		const at = pass.ndcOf(id);
		if (!at) return;
		const W = stage.clientWidth;
		const H = stage.clientHeight;
		const x = ((at.x + 1) / 2) * W;
		const y = ((1 - at.y) / 2) * H;
		if (Math.abs(x - hover.x) < 0.5 && Math.abs(y - hover.y) < 0.5) return;
		hover = { ...hover, x, y, left: x > W * 0.62 };
	}


	function ndc(e: PointerEvent | MouseEvent) {
		if (!stage) return null;
		const r = stage.getBoundingClientRect();
		return { x: ((e.clientX - r.left) / r.width) * 2 - 1, y: 1 - ((e.clientY - r.top) / r.height) * 2, r };
	}

	/**
	 * What the pointer is on: a marker (or one memory of a pill), else the
	 * memory on the lane under it written closest to that moment. Everything
	 * a click would open shows itself first.
	 */
	function targetAt(
		x: number,
		y: number
	): { id: string; kind: 'answer' | 'walk'; member: number; members: number; near: boolean; cluster: string[] | null } | null {
		if (!pass) return null;
		const hit = pass.pickAt(x, y);
		if (hit) {
			const cluster = hit.cluster ?? null;
			// The wheel chose a memory of this pill: keep it until the pointer leaves the pill.
			if (cluster && wheelPill === hit.id && hover && cluster.includes(hover.id)) {
				return { id: hover.id, kind: hit.kind, member: hover.member, members: cluster.length, near: false, cluster };
			}
			const id = pass.memberAt(hit, y);
			return { id, kind: hit.kind, member: cluster ? cluster.indexOf(id) : 0, members: cluster?.length ?? 1, near: false, cluster };
		}
		return null;
	}

	// A pill of memories written together: the wheel steps through them one by
	// one. On an opened island with more than one page, the wheel turns pages.
	let wheelPill: string | null = null;
	let wheelAt = 0;
	function turnPage(step: number) {
		if (!zoom || !pass) return;
		const next = Math.min(zoom.pages - 1, Math.max(0, zoom.page + step));
		if (next === zoom.page) return;
		hover = null;
		pass.setPage(next);
	}
	function onWheel(e: WheelEvent) {
		if (zoom && zoom.pages > 1 && !hover?.cluster) {
			e.preventDefault();
			const now = performance.now();
			if (now - wheelAt < 420) return;
			const step = Math.sign(e.deltaY || e.deltaX);
			if (!step) return;
			wheelAt = now;
			turnPage(step);
			return;
		}
		const cl = hover?.cluster;
		if (!hover || !cl || cl.length < 2 || !pass) return;
		e.preventDefault();
		const step = Math.sign(e.deltaY || e.deltaX);
		if (!step) return;
		const member = Math.min(cl.length - 1, Math.max(0, hover.member + step));
		if (member === hover.member) return;
		const id = cl[member];
		const words = describe?.(id);
		if (!words) return;
		wheelPill = cl[0];
		pass.setHover(id);
		hover = { ...hover, id, member, words };
	}
	$effect(() => {
		const el = stage;
		if (!el) return;
		el.addEventListener('wheel', onWheel, { passive: false });
		return () => el.removeEventListener('wheel', onWheel);
	});

	function onMove(e: PointerEvent) {
		const p = ndc(e);
		if (!p || !pass) return;
		const t = targetAt(p.x, p.y);
		pass.setHover(t?.id ?? null);
		cursorHot = !!t;
		// Between the orbs, the island itself answers: it opens.
		const rowKey = t ? null : pass.laneAt(p.x, p.y);
		pass.setHoverLane(t ? null : rowKey);
		if (rowKey) {
			cursorHot = true;
			const lane = lanes.find((l) => l.label === rowKey);
			const n = lane?.count ?? 0;
			const x = e.clientX - p.r.left;
			hover = {
				id: `row:${rowKey}`,
				x,
				y: e.clientY - p.r.top,
				left: x > p.r.width * 0.62,
				words: {
					kicker: `${laneName(rowKey)} · ${n} ${n === 1 ? 'memory' : 'memories'}`,
					text:
						lane && lane.shown < n
							? `Open it to see all ${n.toLocaleString()}, big and spaced out. The ${lane.shown} here are ${axis === 'day' || axis === 'week' ? 'spread across its hours' : 'the newest'}.`
							: 'Open it to see every memory in it, big and spaced out.',
					tone: memories.find((m) => keyOfMemory(m) === rowKey)?.type ?? rowKey
				},
				member: 0,
				members: 1,
				near: false,
				cluster: null
			};
			return;
		}
		// The picked memory is already shown in the page's reading lane
		// (unless a second click on it is how one goes inside).
		const words = t && (t.id !== selectedId || enter === 'second') ? describe?.(t.id) : null;
		if (!t || !words) {
			hover = null;
			wheelPill = null;
			return;
		}
		if (!t.cluster || t.cluster[0] !== wheelPill) wheelPill = null;
		if (enter && !preloaded) {
			// The memory page's code is fetched while the pointer is still deciding.
			preloaded = true;
			void preloadCode(`${base}/memory`).catch(() => (preloaded = false));
		}
		if (hover?.id !== t.id) {
			hover = { id: t.id, x: 0, y: 0, left: false, words, member: t.member, members: t.members, near: t.near, cluster: t.cluster };
			placeCard(t.id);
		}
	}

	function onLeave() {
		hover = null;
		wheelPill = null;
		cursorHot = false;
		pass?.setHover(tetherId);
		pass?.setHoverLane(null);
	}

	let entering = false;
	function onClick(e: MouseEvent) {
		const p = ndc(e);
		if (!p || !pass) return;
		// A marker opens its memory; a pill or the row around the markers
		// opens the row, where every memory has room of its own.
		const hit = targetAt(p.x, p.y);
		hover = null;
		if (!hit) {
			const rowKey = pass.laneAt(p.x, p.y);
			if (rowKey) openRow(rowKey);
			return;
		}
		if (hit.members > 1 && !zoom) {
			const lane = pass.placed().find((q) => q.cluster?.includes(hit.id) || q.id === hit.id)?.lane;
			if (lane) {
				openRow(lane);
				return;
			}
		}
		openMemory(hit.id, hit.kind);
	}

	/** What a click on a memory does: pick it, or fly in and go inside it. */
	function openMemory(id: string, kind: 'answer' | 'walk') {
		if (!pass) return;
		hover = null;
		if (!enter || (enter === 'second' && kind === 'answer' && id !== selectedId)) {
			onpick?.(id, kind);
			return;
		}
		if (entering) return;
		entering = true;
		// Zoom in: the camera flies to the memory, then the view dives inside
		// it and its own page opens on the same light.
		const target = id;
		const tone = describe?.(target)?.tone;
		pass.setSelected(target);
		hover = null;
		// The camera's flight in is the first part of the transition; the dive
		// into the memory takes over before it settles.
		setTimeout(() => {
			const focus = pass?.ndcOf(target) ?? null;
			const [r, g, b] = typeColor(tone ?? '');
			void enterMemory(target, {
				canvas: stage?.querySelector('canvas') ?? null,
				engine,
				focus,
				color: `rgb(${Math.round(r * 255)} ${Math.round(g * 255)} ${Math.round(b * 255)})`
			}).finally(() => (entering = false));
		}, 340);
	}

	/** What a click on this memory does, for its card. */
	function actionFor(id: string): string {
		if (id.startsWith('row:')) return 'Click to open it';
		if (hover?.id === id && hover.members > 1 && !zoom) return 'Click to open it and see each one';
		if (enter === true) return 'Click to open this memory';
		if (enter === false) return 'Click to read it here';
		const answer = memories.some((m) => m.id === id);
		return answer && id !== selectedId ? 'Click to see what shaped it' : 'Click to go inside this memory';
	}

	const LANE_NAMES: Record<string, string> = {
		decision: 'decisions',
		correction: 'corrections',
		fact: 'facts',
		event: 'events',
		pattern: 'patterns',
		note: 'notes',
		intention: 'intentions',
		stop: 'stopped',
		permit: 'permitted',
		pass: 'went through',
		shadow: 'shadow'
	};
	const laneName = (k: string) => LANE_NAMES[k] ?? k;
	const rgbOf = (c: [number, number, number]) => `rgb(${Math.round(c[0] * 255)} ${Math.round(c[1] * 255)} ${Math.round(c[2] * 255)})`;
	const sameDay = (z: RowZoom) => new Date(z.from).toDateString() === new Date(z.to).toDateString();
	/** When a zoomed row's memories were written, in words. */
	function spanText(z: RowZoom): string {
		if (!z.from) return '';
		const day = (t: number) => new Date(t).toLocaleDateString([], { month: 'short', day: 'numeric' });
		const clock = (t: number) => new Date(t).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
		return sameDay(z) ? `${day(z.from)}, ${clock(z.from)} to ${clock(z.to)}` : `${day(z.from)} to ${day(z.to)}`;
	}
	function timeOf(iso: string, z: RowZoom): string {
		const t = Date.parse(iso);
		if (!Number.isFinite(t)) return '';
		const d = new Date(t);
		return sameDay(z)
			? d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
			: `${d.toLocaleDateString([], { month: 'short', day: 'numeric' })} ${d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`;
	}

	const EDGE_TYPES = new Set(['derived_from', 'evidence_of', 'closes', 'closed_by', 'supersedes', 'corrects', 'contradicts', 'touched']);
	const rgb = (t: string | undefined) => {
		const [r, g, b] = t && EDGE_TYPES.has(t) ? edgeColor(t) : typeColor(t ?? '');
		return `rgb(${Math.round(r * 255)} ${Math.round(g * 255)} ${Math.round(b * 255)})`;
	};
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="stage" bind:this={stage} onpointermove={onMove} onpointerleave={onLeave} onclick={onClick}>
	<ObservatoryCanvas demo="recall-path" {seed} maxDpr={2} {onready} {onframe} />
	{#each ticks.ticks as tk, k (`${k}:${tk.label}`)}
		<span class="tick" class:away={!!selectedId} style:left="{(tk.x + 1) * 50}%" style:top="{(1 - tk.y) * 50}%">{tk.label}</span>
	{/each}
	{#each lanes as lane (lane.label)}
		<!-- Each island's name, big and glowing, just under it. -->
		<button
			type="button"
			class="isle-name"
			class:phone
			class:away={!!selectedId}
			style:left="{(lane.x + 1) * 50}%"
			style:top="{(1 - (lane.y - lane.r)) * 50}%"
			style:max-width="{Math.max(6, lane.w * 50 - 1)}vw"
			style:--lane={rgbOf(lane.color)}
			title="Open {laneName(lane.label)}"
			onpointerenter={() => pass?.setHoverLane(lane.label)}
			onpointerleave={() => pass?.setHoverLane(null)}
			onclick={(e) => {
				e.stopPropagation();
				openRow(lane.label);
			}}
		>
			<span class="isle-label">{laneName(lane.label)}</span>
			<span class="isle-count"
				><b>{lane.count.toLocaleString()}</b>{lane.shown < lane.count && !phone ? ` · ${lane.shown} shown` : ''}</span
			>
		</button>
	{/each}
	{#if zoom}
		<!-- An opened island's heading: the way back, its name, and its pages. -->
		<div
			class="isle-head"
			class:phone
			class:away={!!selectedId}
			style:left="{(zoom.x + 1) * 50}%"
			style:top="{(1 - zoom.top) * 50}%"
			style:--lane={rgbOf(zoom.color)}
		>
			<button
				type="button"
				class="isle-back"
				onclick={(e) => {
					e.stopPropagation();
					openRow(null);
				}}>← all</button
			>
			<span class="isle-title">{laneName(zoom.key)}</span>
			<span class="isle-meta">{zoom.count.toLocaleString()} {zoom.count === 1 ? 'memory' : 'memories'} · {spanText(zoom)}</span>
			{#if zoom.pages > 1}
				<span class="isle-pages">
					<button
						type="button"
						class="isle-page"
						disabled={zoom.page === 0}
						onclick={(e) => {
							e.stopPropagation();
							turnPage(-1);
						}}>‹ newer</button
					>
					<span class="isle-page-n">{zoom.page + 1} / {zoom.pages}</span>
					<button
						type="button"
						class="isle-page"
						disabled={zoom.page >= zoom.pages - 1}
						onclick={(e) => {
							e.stopPropagation();
							turnPage(1);
						}}>older ›</button
					>
				</span>
			{/if}
		</div>
	{/if}
	{#if hover}
		{#key hover.id}
			<HoverCard
				x={hover.x}
				y={hover.y}
				left={hover.left}
				tone={rgb(hover.words.tone)}
				kicker={hover.words.kicker}
				text={headline(hover.words.text, 160)}
				note={hover.members > 1
					? zoom
						? `${hover.member + 1} of ${hover.members} written together · scroll or move along the pill to choose`
						: `${hover.members} written together`
					: null}
				action={`${actionFor(hover.id)}${hover.near ? ' · nearest in time' : ''}`}
			/>
		{/key}
	{/if}
	{#if tether}
		<svg class="tether" aria-hidden="true">
			<path d={tether.d} style:stroke={tether.tone ? rgb(tether.tone) : '#7ff3e6'} />
			<path class="tether-run" d={tether.d} />
		</svg>
	{/if}
</div>
{#if rowPanel && zoom}
	<aside class="tr-lane tr-row" data-quiet aria-label="Every memory in this row" style:--lane={rgbOf(zoom.color)}>
		<p class="tr-proof-kicker row-kicker">{laneName(zoom.key)} · {zoom.count}</p>
		<p class="tr-note">{spanText(zoom)}. In the order they were written: hover one to find its orb, click it to {enter === true ? 'go inside' : enter === false ? 'read it here' : 'see what shaped it'}.</p>
		{#each rowMembers.slice(0, ROW_LIST_MAX) as m (m.id)}
			<button type="button" class="tr-item" data-trace-id={m.id} onclick={() => openMemory(m.id, 'answer')}>
				<span class="tr-edge row-time">{timeOf(m.createdAt, zoom)}</span>
				<span>{headline(describe?.(m.id)?.text ?? m.content, 120)}</span>
			</button>
		{/each}
		{#if rowMembers.length > ROW_LIST_MAX}
			<p class="tr-note">and {(rowMembers.length - ROW_LIST_MAX).toLocaleString()} more: turn the pages on the field to reach them.</p>
		{/if}
	</aside>
{/if}
{#if !quiet}
	<p class="telemetry">
		{PARTICLES.toLocaleString()} particles · WebGPU compute{gpuMs !== null ? ` · GPU ${gpuMs.toFixed(2)} ms a frame` : ''}{fps
			? ` · ${fps} fps`
			: ''}
	</p>
{/if}

<style>
	.stage {
		position: fixed;
		inset: 0;
		background: #05060a;
		cursor: crosshair;
	}
	.tether {
		position: absolute;
		inset: 0;
		width: 100%;
		height: 100%;
		pointer-events: none;
		overflow: visible;
		z-index: 25;
	}
	.tether path {
		fill: none;
		stroke-width: 1.2;
		opacity: 0.55;
		filter: drop-shadow(0 0 4px currentColor);
	}
	.tether .tether-run {
		stroke: #ffffff;
		stroke-width: 1.6;
		stroke-dasharray: 3 22;
		opacity: 0.9;
		animation: tether-run 0.9s linear infinite;
	}
	@keyframes tether-run {
		to {
			stroke-dashoffset: -25;
		}
	}
	/* An opened clock's hours, round its rim. */
	.tick {
		position: absolute;
		transform: translate(-50%, -50%);
		pointer-events: none;
		font-family: ui-monospace, 'SF Mono', Menlo, monospace;
		font-size: 0.72rem;
		letter-spacing: 0.14em;
		text-transform: uppercase;
		color: color-mix(in srgb, var(--uni-ink, #e4fff8) 80%, transparent);
		text-shadow:
			0 0 12px var(--uni-glow, rgba(127, 243, 230, 0.5)),
			0 0 3px rgba(2, 3, 7, 0.9);
		white-space: nowrap;
		transition: opacity 0.3s ease;
		z-index: 21;
	}
	/* Each island's name, big and glowing, centred just under it. */
	.isle-name {
		position: absolute;
		transform: translate(-50%, 0.85rem);
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 0.2rem;
		padding: 0.15rem 0.4rem;
		border: none;
		background: none;
		cursor: pointer;
		font-family: ui-monospace, 'SF Mono', Menlo, monospace;
		text-align: center;
		white-space: nowrap;
		transition:
			opacity 0.3s ease,
			transform 0.2s ease;
		z-index: 22;
	}
	.isle-name:hover,
	.isle-name:focus-visible {
		transform: translate(-50%, 0.85rem) scale(1.06);
	}
	.isle-label {
		max-width: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
		font-size: 0.98rem;
		font-weight: 600;
		letter-spacing: 0.16em;
		text-transform: uppercase;
		color: color-mix(in srgb, var(--lane) 70%, white);
		text-shadow:
			0 0 12px color-mix(in srgb, var(--lane) 75%, transparent),
			0 0 28px color-mix(in srgb, var(--uni-glow, var(--lane)) 55%, transparent);
	}
	.isle-name:hover .isle-label,
	.isle-name:focus-visible .isle-label {
		color: #fff;
	}
	.isle-count {
		font-size: 0.74rem;
		letter-spacing: 0.08em;
		color: color-mix(in srgb, var(--lane) 50%, #8aa0a6);
		text-shadow: 0 0 10px color-mix(in srgb, var(--lane) 35%, transparent);
	}
	.isle-count b {
		font-size: 0.86rem;
		font-weight: 700;
		color: color-mix(in srgb, var(--lane) 80%, white);
	}
	.isle-name.phone .isle-label {
		font-size: 0.72rem;
		letter-spacing: 0.08em;
	}
	.isle-name.phone .isle-count {
		font-size: 0.62rem;
	}
	.isle-name.away,
	.isle-head.away,
	.tick.away {
		opacity: 0;
		pointer-events: none;
	}
	/* An opened island's heading, centred over it. */
	.isle-head {
		position: absolute;
		transform: translate(-50%, 0.3rem);
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		justify-content: center;
		gap: 0.4rem 1.1rem;
		max-width: 92vw;
		font-family: ui-monospace, 'SF Mono', Menlo, monospace;
		white-space: nowrap;
		z-index: 22;
		transition: opacity 0.3s ease;
	}
	.isle-back,
	.isle-page {
		padding: 0.35rem 0.7rem;
		border: 1px solid color-mix(in srgb, var(--lane) 45%, transparent);
		border-radius: 999px;
		background: color-mix(in srgb, var(--lane) 10%, rgba(3, 5, 9, 0.6));
		cursor: pointer;
		font: inherit;
		font-size: 0.74rem;
		letter-spacing: 0.12em;
		text-transform: uppercase;
		color: color-mix(in srgb, var(--lane) 75%, white);
		box-shadow: 0 0 18px color-mix(in srgb, var(--lane) 25%, transparent);
		transition:
			background 0.16s ease,
			color 0.16s ease;
	}
	.isle-back:hover,
	.isle-back:focus-visible,
	.isle-page:hover:not(:disabled),
	.isle-page:focus-visible {
		color: #fff;
		background: color-mix(in srgb, var(--lane) 26%, rgba(3, 5, 9, 0.6));
	}
	.isle-page:disabled {
		opacity: 0.35;
		cursor: default;
	}
	.isle-title {
		font-size: 1.25rem;
		font-weight: 600;
		letter-spacing: 0.16em;
		text-transform: uppercase;
		color: color-mix(in srgb, var(--lane) 70%, white);
		text-shadow:
			0 0 14px color-mix(in srgb, var(--lane) 75%, transparent),
			0 0 34px color-mix(in srgb, var(--uni-glow, var(--lane)) 55%, transparent);
	}
	.isle-meta {
		font-size: 0.78rem;
		letter-spacing: 0.06em;
		color: color-mix(in srgb, var(--lane) 45%, #9fb3b8);
	}
	.isle-pages {
		display: inline-flex;
		align-items: baseline;
		gap: 0.6rem;
	}
	.isle-page-n {
		font-size: 0.8rem;
		color: color-mix(in srgb, var(--lane) 70%, white);
	}
	.isle-head.phone .isle-title {
		font-size: 0.92rem;
	}
	.isle-head.phone .isle-meta {
		display: none;
	}
	.row-kicker {
		color: color-mix(in srgb, var(--lane) 85%, white);
	}
	.row-time {
		min-width: 4.2rem;
		color: color-mix(in srgb, var(--lane) 70%, #9fb3b0);
	}
	.telemetry {
		position: fixed;
		z-index: 20;
		right: 1.8rem;
		bottom: 1.4rem;
		margin: 0;
		font-family: ui-monospace, 'SF Mono', Menlo, monospace;
		font-size: 0.6rem;
		letter-spacing: 0.14em;
		color: #3f5855;
		pointer-events: none;
	}
	@media (max-width: 767px) {
		.telemetry {
			display: none;
		}
	}
</style>
