/**
 * Trace — the WebGPU pass.
 *
 * Lays real memories out on the CPU (every group of the answer as one big
 * round island of orbs, newest at its heart, and the causal walk in rings
 * round a picked memory by depth), uploads small buffers, then runs a
 * compute pass that simulates about 150k particles and one instanced draw
 * of velocity-stretched sprites over a fullscreen backdrop. Events (answer, pick, receipt, walk, cursor) are
 * forces in the simulation, not canned animations. Ages come from the
 * engine's monotonic frame count, so pausing the engine pauses the scene.
 */
import type { FramePass, ObservatoryEngine } from '$lib/observatory/engine';
import { TRACE_BACKDROP_WGSL, TRACE_COMPUTE_WGSL, TRACE_MARKERS_WGSL, TRACE_PARTICLES_WGSL } from './trace-wgsl';
import { isCausalEdge, type TraceLink, type TraceMemory, type WalkResult } from './trace-data';
import { HANDOFF_STRIDE, MAX_HANDOFF } from './handoff';

const MAX_MEMS = 1200;
const MAX_EDGES = 256;
const RIVER = 48_000;
/** One pool of galaxy particles, shared out evenly: few memories, big galaxies. */
const MEM_POOL = 36_000;
const PER_EDGE = 64;
const TOTAL = RIVER + MEM_POOL + MAX_EDGES * PER_EDGE;
const MAX_ANSWER = 1100;
/** Particles per memory at which a galaxy has its base size and brightness. */
const BASE_PER_MEM = 160;
/**
 * Galaxy slots kept for causes in the sky. The pool is split once per
 * answer (answer + this reserve), so a walk landing never re-deals the
 * river's particles.
 */
const SKY_RESERVE = 64;
const BINS = 256;
const FLAG_ANSWER = 1;
const FLAG_WALK = 2;
const FLAG_PICKED = 4;
const FLAG_CAUSE = 8;
const FLAG_FUTURE = 16;
const EDGE_STRIDE = 12;
/** The river sits low on a wide screen (the causal sky above it) and mid-screen on a portrait one. */
function riverFor(aspect: number): { y: number; halfH: number } {
	// The memories' band: below the organ's title on a wide screen, the
	// middle of the field on a phone. The river itself fills the whole screen.
	return aspect < 1 ? { y: -0.02, halfH: 0.4 } : { y: -0.24, halfH: 0.58 };
}

const ADDITIVE: GPUBlendState = {
	color: { srcFactor: 'one', dstFactor: 'one', operation: 'add' },
	alpha: { srcFactor: 'one', dstFactor: 'one', operation: 'add' }
};

/** Colour by memory type: the categories Vestige records. */
export function typeColor(type: string): [number, number, number] {
	switch (type) {
		case 'decision':
			return [1.0, 0.74, 0.3];
		case 'correction':
			return [1.0, 0.42, 0.33];
		case 'fact':
			return [0.35, 0.88, 1.0];
		case 'event':
			return [0.3, 1.0, 0.68];
		case 'pattern':
			return [0.72, 0.62, 1.0];
		case 'note':
			return [0.78, 0.84, 0.9];
		case 'stop':
			return [1.0, 0.26, 0.32];
		case 'permit':
			return [0.32, 1.0, 0.62];
		case 'intention':
			return [1.0, 0.94, 0.78];
		case 'pass':
			return [0.22, 0.52, 0.58];
		case 'shadow':
			return [1.0, 0.64, 0.24];
		default:
			return [0.4, 0.9, 0.86];
	}
}

/** Colour of a cause in the sky: how it is related, by its recorded edge. */
export function edgeColor(linkType: string): [number, number, number] {
	switch (linkType) {
		case 'derived_from':
			return [1.0, 0.78, 0.36];
		case 'evidence_of':
			return [1.0, 0.62, 0.26];
		case 'closes':
		case 'closed_by':
			return [0.36, 1.0, 0.7];
		case 'supersedes':
		case 'corrects':
			return [1.0, 0.5, 0.4];
		case 'contradicts':
			return [1.0, 0.28, 0.36];
		default:
			return [0.46, 0.78, 0.82];
	}
}

/** The islands, in reading order, when a memory's type names its group. */
const TYPE_ORDER = ['decision', 'correction', 'fact', 'event', 'pattern', 'note', 'intention', 'stop', 'permit', 'pass', 'shadow'];
const MAX_ISLANDS = 12;
/** Islands the backdrop and the halo particles know (8 floats each in the island buffer). */
const ISLAND_SLOTS = 24;
/** A memory at rest on its island: an orb this radius in world units (about 13 px on a 900 px screen). */
const ORB = 0.029;
/** The smallest an orb gets, on a phone's small islands (about 8 px). */
const ORB_MIN = 0.018;
/** An opened island's orbs are bigger still (about 17 px). */
const ZOOM_ORB = 0.038;
/** Centre to centre between two orbs, in orb radii: room round every one. */
const ORB_GAP = 2.5;
/** The turn between two seeds of a sunflower. */
const GOLDEN = Math.PI * (3 - Math.sqrt(5));
/** Height kept under each island for its name and count, in world units. */
const LABEL_H = 0.1;
const FLAG_CLUSTER = 32;
/** The marker's shape by type (drawn in the marker shader). */
const SHAPE: Record<string, number> = {
	fact: 0,
	decision: 1,
	correction: 2,
	stop: 2,
	event: 3,
	pass: 3,
	pattern: 4,
	note: 5,
	shadow: 5,
	intention: 6,
	permit: 6
};
const SHAPE_CLUSTER = 7;

function hash01(s: string): number {
	let h = 2166136261;
	for (let i = 0; i < s.length; i++) {
		h ^= s.charCodeAt(i);
		h = Math.imul(h, 16777619);
	}
	return ((h >>> 0) % 100000) / 100000;
}

const TICK_STEPS_MIN = [5, 10, 15, 30, 60, 120, 180, 360, 720, 1440, 2880, 10080, 43200];

export interface PlacedMemory {
	id: string;
	x: number;
	y: number;
	size: number;
	kind: 'answer' | 'walk';
	depth: number;
	/** A column too full to show one by one: every memory it holds, this one first. */
	cluster?: string[];
	/** The island (group) it stands on. */
	lane?: string;
}

/** An opened island: which one, how many memories it holds, when they were written, and which page shows. */
export interface RowZoom {
	key: string;
	count: number;
	color: [number, number, number];
	from: number;
	to: number;
	/** NDC y of the top of the field, through the camera (where its heading goes). */
	top: number;
	/** NDC x of the island's centre. */
	x: number;
	/** The page of orbs on show (0 is the newest), of how many. */
	page: number;
	pages: number;
	/** How many orbs this page shows. */
	shown: number;
}

export type Axis = 'time' | 'order' | 'day' | 'week';
const DAY_MS = 86_400_000;
/** Where a moment falls in its local day (0..1). */
function inDay(ms: number): number {
	const d = new Date(ms);
	d.setHours(0, 0, 0, 0);
	return (ms - d.getTime()) / DAY_MS;
}
/** Where a moment falls in its week, Monday first (0..1). */
function inWeek(ms: number): number {
	const d = new Date(ms);
	const back = (d.getDay() + 6) % 7;
	d.setHours(0, 0, 0, 0);
	return (ms - (d.getTime() - back * DAY_MS)) / (7 * DAY_MS);
}

/** One island of the field, for its name. */
export interface Lane {
	label: string;
	/** NDC centre, through the camera. */
	x: number;
	y: number;
	/** Its radius in NDC y units (the name goes just under it). */
	r: number;
	/** The width its name may take, in NDC x units. */
	w: number;
	/** How many memories it holds, and how many show as orbs. */
	count: number;
	shown: number;
	color: [number, number, number];
}

export interface TimeTick {
	/** NDC position of the label. */
	x: number;
	y: number;
	label: string;
}

interface Island {
	key: string;
	x: number;
	y: number;
	rx: number;
	ry: number;
	/** World width of its cell, for its name. */
	cell: number;
	color: [number, number, number];
	count: number;
	shown: number;
}

export type VerifyState = 'idle' | 'running' | 'ok' | 'fail';

/** The story an organ tells with the field: how its answer arrives and keeps moving. */
export type Story = 'trace' | 'ledger' | 'guard' | 'healing' | 'causal' | 'system' | 'witness';
const STORY_ID: Record<Story, number> = { trace: 0, ledger: 1, guard: 2, healing: 3, causal: 4, system: 5, witness: 6 };
/** River deep, river bright and heat colour, per organ. */
const STORY_PALETTE: Record<Story, [number, number, number][]> = {
	trace: [
		[0.05, 0.3, 0.36],
		[0.22, 0.95, 0.9],
		[1.0, 0.76, 0.42]
	],
	ledger: [
		[0.32, 0.2, 0.06],
		[1.0, 0.78, 0.38],
		[1.0, 0.86, 0.55]
	],
	guard: [
		[0.22, 0.05, 0.16],
		[0.78, 0.36, 0.95],
		[1.0, 0.55, 0.75]
	],
	healing: [
		[0.04, 0.26, 0.17],
		[0.36, 1.0, 0.66],
		[0.75, 1.0, 0.85]
	],
	causal: [
		[0.3, 0.08, 0.04],
		[1.0, 0.52, 0.22],
		[1.0, 0.62, 0.3]
	],
	system: [
		[0.06, 0.14, 0.34],
		[0.45, 0.72, 1.0],
		[0.7, 0.85, 1.0]
	],
	witness: [
		[0.04, 0.2, 0.15],
		[0.5, 0.95, 0.72],
		[1.0, 0.84, 0.5]
	]
};

/** A block of the page's words, in NDC (x0 < x1, y0 < y1). */
export interface QuietRect {
	x0: number;
	y0: number;
	x1: number;
	y1: number;
}
const MAX_QUIET = 4;

export interface TracePass extends FramePass {
	/**
	 * The answer: memories along the river. `range` is the day (or null to
	 * span the set). `origin` is where the answer erupts from, in NDC.
	 */
	setAnswer(mems: TraceMemory[], range: { start: number; end: number } | null, origin?: { x: number; y: number }): void;
	setSelected(id: string | null): void;
	setWalk(walk: WalkResult | null): void;
	/** Recorded edges between memories in the answer, drawn as arches over the river. */
	setLinks(links: TraceLink[]): void;
	/** A "now" seam on the river (ms), or null. */
	setNow(ms: number | null): void;
	/**
	 * 'time' places memories by the minute they were written; 'order' spaces
	 * them evenly in that same order, for sets that span months unevenly;
	 * 'day' and 'week' read like a ledger book: x is the hour of its day (or
	 * the moment in its week), and the page puts each day (or week) on a row.
	 */
	setAxis(axis: Axis): void;
	/** Which organ's story the field tells. */
	setStory(story: Story): void;
	/**
	 * Start from the Palace's last frame: the next answer places every
	 * particle on a handed-off point and holds the picture until the organ's
	 * data arrives; then the particles reorganize in the organ's own story.
	 */
	beginHandoff(points: Float32Array<ArrayBuffer>, count: number): void;
	/**
	 * Another organ is taking over this same field: the next answer keeps
	 * every particle where it is and springs it into the new layout, so one
	 * organ's picture becomes the next instead of a new eruption.
	 */
	continueNext(): void;
	setHover(id: string | null): void;
	/** The receipt check for the picked memory: running, then ok or fail. */
	setVerify(state: VerifyState): void;
	/** Cursor position in NDC, or null when it leaves the stage. */
	setCursor(ndc: { x: number; y: number } | null): void;
	pickAt(ndcX: number, ndcY: number): PlacedMemory | null;
	/** Screen-space (NDC) position of a placed memory, for DOM labels. */
	ndcOf(id: string): { x: number; y: number } | null;
	placed(): PlacedMemory[];
	/** The hours round an opened clock (a ledger's day or week), in NDC; empty otherwise. */
	timeTicks(): { ticks: TimeTick[]; floorY: number };
	/** The islands on screen, in reading order, for their names. */
	lanes(): Lane[];
	/** Keep the left of the field for the dock: islands start this many CSS pixels from the left edge. */
	setInset(px: number): void;
	/** Which memory a placed orb stands for. */
	memberAt(p: PlacedMemory, ndcY: number): string;
	/**
	 * Open one island: it fills the field, every memory in it a big orb with
	 * room round it, a page at a time. Null shows every island.
	 */
	setFocusLane(key: string | null): void;
	/** Which page of an opened island shows (0 is the newest). */
	setPage(page: number): void;
	/** The island the pointer is over: its rim lights up. */
	setHoverLane(key: string | null): void;
	/** The opened island, when one is. */
	zoom(): RowZoom | null;
	/** The island under a point (overview only), for a click that opens it. */
	laneAt(ndcX: number, ndcY: number): string | null;
	/**
	 * The page's text blocks, in NDC. Memories are never placed in them and
	 * the field dims to a whisper behind them, so words never sit on light.
	 */
	setQuiet(rects: QuietRect[]): void;
	/** Inspection only: what the pass last uploaded. */
	debug(): { placed: number; edgeCount: number; edges: number[]; flags: number[] };
	dispose(): void;
}

export function createTracePass(engine: ObservatoryEngine): TracePass {
	const device = engine.gpuDevice;
	const params = engine.paramsBuffer;
	if (!device || !params) throw new Error('Trace: engine is not running');

	const tuArr = new Float32Array(56 + MAX_QUIET * 4 + 8);
	const memArr = new Float32Array(MAX_MEMS * 8);
	const edgeArr = new Float32Array(MAX_EDGES * EDGE_STRIDE);
	const densArr = new Float32Array(BINS);
	const partArr = new Float32Array(TOTAL * 4);

	const usage = GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST;
	const tuBuf = device.createBuffer({ label: 'trace-tu', size: tuArr.byteLength, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
	const memBuf = device.createBuffer({ label: 'trace-mems', size: memArr.byteLength, usage });
	const edgeBuf = device.createBuffer({ label: 'trace-edges', size: edgeArr.byteLength, usage });
	// The islands: x, y, rx, ry, rgb and lit, 8 floats each (the backdrop draws
	// them; the halo particles orbit them).
	const densBuf = device.createBuffer({ label: 'trace-islands', size: densArr.byteLength, usage });
	const handBuf = device.createBuffer({ label: 'trace-handoff', size: MAX_HANDOFF * HANDOFF_STRIDE * 4, usage });
	const partBuf = device.createBuffer({ label: 'trace-particles', size: partArr.byteLength, usage });
	// Simulation state: pos.xy vel.xy | generations, heat. Starts at zero.
	const stateBuf = device.createBuffer({ label: 'trace-state', size: TOTAL * 32, usage: GPUBufferUsage.STORAGE });

	// Static particle assignment: kind, owner, seed, u — deterministic.
	{
		let i = 0;
		const put = (kind: number, owner: number, seed: number, u: number) => {
			partArr.set([kind, owner, seed, u], i * 4);
			i++;
		};
		for (let k = 0; k < RIVER; k++) put(0, 0, hash01(`r${k}`), hash01(`ru${k}`));
		// Galaxy particles carry their pool index; the shaders share the pool
		// out across however many memories are placed.
		for (let k = 0; k < MEM_POOL; k++) put(1, k, hash01(`m${k}`), hash01(`mu${k}`));
		for (let e = 0; e < MAX_EDGES; e++) for (let k = 0; k < PER_EDGE; k++) put(2, e, hash01(`e${e}:${k}`), k / PER_EDGE);
		device.queue.writeBuffer(partBuf, 0, partArr);
	}

	const fmt = engine.sceneFormat;
	const modules = {
		compute: device.createShaderModule({ label: 'trace-compute', code: TRACE_COMPUTE_WGSL }),
		particles: device.createShaderModule({ label: 'trace-particles', code: TRACE_PARTICLES_WGSL }),
		backdrop: device.createShaderModule({ label: 'trace-backdrop', code: TRACE_BACKDROP_WGSL }),
		markers: device.createShaderModule({ label: 'trace-markers', code: TRACE_MARKERS_WGSL })
	};
	for (const [name, m] of Object.entries(modules)) {
		void m.getCompilationInfo().then((info) => {
			for (const msg of info.messages) {
				if (msg.type === 'error') console.error(`[trace:${name}] ${msg.lineNum}:${msg.linePos} ${msg.message}`);
			}
		});
	}
	const renderPipe = (module: GPUShaderModule, label: string) =>
		device.createRenderPipeline({
			label,
			layout: 'auto',
			vertex: { module, entryPoint: 'vs' },
			fragment: { module, entryPoint: 'fs', targets: [{ format: fmt, blend: ADDITIVE }] },
			primitive: { topology: 'triangle-list' }
		});
	// The simulation and the particle draw share one set of buffers. Their
	// layouts are spelled out, so a binding one shader stops reading never
	// invalidates the bind group.
	const fieldLayout = (visibility: number, stateType: GPUBufferBindingType) =>
		device.createBindGroupLayout({
			entries: [0, 1, 2, 3, 4, 5, 6, 7].map((binding) => ({
				binding,
				visibility,
				buffer: { type: binding < 2 ? 'uniform' : binding === 5 ? stateType : 'read-only-storage' }
			}))
		});
	const simLayout = fieldLayout(GPUShaderStage.COMPUTE, 'storage');
	const partLayout = fieldLayout(GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT, 'read-only-storage');
	const simPipe = device.createComputePipeline({
		label: 'trace-sim',
		layout: device.createPipelineLayout({ bindGroupLayouts: [simLayout] }),
		compute: { module: modules.compute, entryPoint: 'cs' }
	});
	const partPipe = device.createRenderPipeline({
		label: 'trace-particles',
		layout: device.createPipelineLayout({ bindGroupLayouts: [partLayout] }),
		vertex: { module: modules.particles, entryPoint: 'vs' },
		fragment: { module: modules.particles, entryPoint: 'fs', targets: [{ format: fmt, blend: ADDITIVE }] },
		primitive: { topology: 'triangle-list' }
	});
	const markPipe = device.createRenderPipeline({
		label: 'trace-markers',
		layout: 'auto',
		vertex: { module: modules.markers, entryPoint: 'vs' },
		fragment: {
			module: modules.markers,
			entryPoint: 'fs',
			targets: [
				{
					format: fmt,
					blend: {
						color: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha', operation: 'add' },
						alpha: { srcFactor: 'one', dstFactor: 'one-minus-src-alpha', operation: 'add' }
					}
				}
			]
		},
		primitive: { topology: 'triangle-list' }
	});
	const backPipe = renderPipe(modules.backdrop, 'trace-backdrop');
	const shared = (layout: GPUBindGroupLayout) =>
		device.createBindGroup({
			layout,
			entries: [
				{ binding: 0, resource: { buffer: params } },
				{ binding: 1, resource: { buffer: tuBuf } },
				{ binding: 2, resource: { buffer: memBuf } },
				{ binding: 3, resource: { buffer: edgeBuf } },
				{ binding: 4, resource: { buffer: partBuf } },
				{ binding: 5, resource: { buffer: stateBuf } },
				{ binding: 6, resource: { buffer: densBuf } },
				{ binding: 7, resource: { buffer: handBuf } }
			]
		});
	const simBG = shared(simLayout);
	const partBG = shared(partLayout);
	const backBG = device.createBindGroup({
		layout: backPipe.getBindGroupLayout(0),
		entries: [
			{ binding: 0, resource: { buffer: params } },
			{ binding: 1, resource: { buffer: tuBuf } },
			{ binding: 2, resource: { buffer: densBuf } }
		]
	});
	const markBG = device.createBindGroup({
		layout: markPipe.getBindGroupLayout(0),
		entries: [
			{ binding: 0, resource: { buffer: params } },
			{ binding: 1, resource: { buffer: tuBuf } },
			{ binding: 2, resource: { buffer: memBuf } }
		]
	});

	const now = () => engine.simSeconds;
	let answer: TraceMemory[] = [];
	let range: { start: number; end: number } | null = null;
	let walk: WalkResult | null = null;
	let links: TraceLink[] = [];
	let nowMs: number | null = null;
	let nowX = -99;
	let quiet: QuietRect[] = [];
	// Where memories may be placed along the river (world x), between the
	// dock and the page's reading lane; set by layout().
	let spanX0 = -1;
	let spanX1 = 1;
	let axis: Axis = 'time';
	let story: Story = 'trace';
	// The handoff: 'pending' until the first answer places particles on the
	// Palace's points; 'holding' until the organ's data lands; then 'released'.
	let handoffState: 'none' | 'pending' | 'holding' | 'released' = 'none';
	let handoffCount = 0;
	let handoffMode = 0;
	let holdUntil = 0;
	let continuing = false;
	let selectedId: string | null = null;
	let hoverId: string | null = null;
	let answerAt = -1e9;
	let igniteAt = -1e9;
	let walkAt = -1e9;
	let hoverAt = -1e9;
	let verifyAt = -1;
	let sealedAt = -1;
	let sealOk = 1;
	let answerGen = 0;
	let walkGen = 0;
	let origin = { x: -0.6, y: 0.8 };
	let cursor: { x: number; y: number } | null = null;
	let cursorVel = { x: 0, y: 0 };
	let cursorAt = 0;
	let lastTotal = -1;
	let substeps = 1;
	// The camera: at rest it shows the whole field; a pick flies it in.
	const cam = { x: 0, y: 0, z: 1 };
	const ZOOM_IN = 1.65;
	let perMem = BASE_PER_MEM;
	let placedList: PlacedMemory[] = [];
	// A memory folded into a column's cluster pill: the pill that holds it.
	let clusterHost = new Map<string, PlacedMemory>();
	let laneKeys: string[] = [];
	let laneColors: [number, number, number][] = [];
	let laneCounts: number[] = [];
	let insetPx = 0;
	// The opened island (a group key), what it holds, and the page on show.
	let focusLane: string | null = null;
	let zoomed: RowZoom | null = null;
	let page = 0;
	let pages = 1;
	let islands: Island[] = [];
	let hoverLane: string | null = null;
	// A ledger's islands are clocks: every write stands at its hour.
	let dialMode = false;
	// The hours round an opened clock, in world units.
	let dialTicks: TimeTick[] = [];
	let edgeCount = 0;
	let lastAspect = 0;
	let dirty = true;

	function aspectNow(): number {
		const c = engine.canvasElement;
		return c.width > 0 && c.height > 0 ? c.width / c.height : 16 / 9;
	}

	/**
	 * A sunflower: n orbs in a disc of radius r round (cx, cy), the first at
	 * the heart and each next one a golden turn further out, so they fill it
	 * evenly with room round every one.
	 */
	function sunflower(n: number, cx: number, cy: number, r: number, turn: number): { x: number; y: number }[] {
		const out: { x: number; y: number }[] = [];
		for (let k = 0; k < n; k++) {
			const rho = n === 1 ? 0 : r * Math.sqrt((k + 0.5) / n);
			const a = k * GOLDEN + turn;
			out.push({ x: cx + rho * Math.cos(a), y: cy + rho * Math.sin(a) });
		}
		return out;
	}

	/**
	 * Room for orbs in a wide oval (an opened island): a loose honeycomb inside
	 * it, each seat nudged a little so no row of it reads as a line, ordered
	 * from the heart outward.
	 */
	function honeycomb(cx: number, cy: number, ex: number, ey: number, gap: number): { x: number; y: number }[] {
		const seats: { x: number; y: number; d: number; a: number }[] = [];
		const rowH = gap * 0.866;
		const rows = Math.floor(ey / rowH);
		for (let j = -rows; j <= rows; j++) {
			const off = Math.abs(j) % 2 ? gap / 2 : 0;
			const cols = Math.ceil(ex / gap) + 1;
			for (let i = -cols; i <= cols; i++) {
				const jx = (hash01(`hx${i}:${j}`) - 0.5) * 0.18 * gap;
				const jy = (hash01(`hy${i}:${j}`) - 0.5) * 0.18 * gap;
				const x = cx + i * gap + off + jx;
				const y = cy + j * rowH + jy;
				const d = Math.hypot((x - cx) / ex, (y - cy) / ey);
				if (d <= 1) seats.push({ x, y, d, a: Math.atan2(y - cy, x - cx) });
			}
		}
		return seats.sort((p, q) => p.d - q.d || p.a - q.a);
	}

	/** A clock's rings: from the rim inward, how many seats each has. */
	function dialRings(R: number, orb: number): { rr: number; slots: number }[] {
		const gap = ORB_GAP * orb;
		const rings: { rr: number; slots: number }[] = [];
		for (let rr = R - orb * 1.7; rr >= gap * 0.55; rr -= gap) {
			const slots = Math.floor((2 * Math.PI * rr) / gap);
			if (slots < 4) break;
			rings.push({ rr, slots });
		}
		if (!rings.length) rings.push({ rr: 0, slots: 1 });
		return rings;
	}

	/**
	 * Seat writes on a clock: each at its moment (the top is where the clock
	 * starts, turning clockwise), on the outer ring first; one already taken
	 * moves to the nearest free seat, then inward. Strict, a write with no
	 * seat near its moment is left off rather than put at the wrong hour.
	 */
	function onDial(
		chosen: number[],
		rings: { rr: number; slots: number }[],
		cx: number,
		cy: number,
		frac: (i: number) => number,
		strict = false
	): { i: number; x: number; y: number }[] {
		const taken = rings.map(() => new Set<number>());
		const out: { i: number; x: number; y: number }[] = [];
		const seat = (i: number, j: number, slot: number) => {
			taken[j].add(slot);
			const a = (slot / rings[j].slots) * 2 * Math.PI;
			out.push({ i, x: cx + rings[j].rr * Math.sin(a), y: cy + rings[j].rr * Math.cos(a) });
		};
		for (const i of chosen) {
			const f = frac(i);
			let done = false;
			for (let j = 0; j < rings.length && !done; j++) {
				const n = rings[j].slots;
				const ideal = Math.round(f * n) % n;
				for (const d of [0, 1, -1, 2, -2]) {
					const slot = (((ideal + d) % n) + n) % n;
					if (!taken[j].has(slot)) {
						seat(i, j, slot);
						done = true;
						break;
					}
				}
			}
			// Every seat near its moment is taken: the nearest free one anywhere.
			for (let j = 0; j < rings.length && !done && !strict; j++) {
				const n = rings[j].slots;
				const ideal = Math.round(f * n) % n;
				for (let d = 0; d < n && !done; d++) {
					for (const sgn of [1, -1]) {
						const slot = (((ideal + sgn * d) % n) + n) % n;
						if (!taken[j].has(slot)) {
							seat(i, j, slot);
							done = true;
							break;
						}
					}
				}
			}
		}
		return out;
	}

	/** k of a list, spread evenly from its first to its last. */
	function evenly<T>(list: T[], k: number): T[] {
		if (list.length <= k) return list;
		if (k <= 1) return [list[list.length - 1]];
		return Array.from({ length: k }, (_, j) => list[Math.round((j * (list.length - 1)) / (k - 1))]);
	}

	function layout() {
		const A = aspectNow();
		lastAspect = A;
		const { y: RIVER_Y, halfH: RIVER_HALF_H } = riverFor(A);
		placedList = [];
		memArr.fill(0);
		edgeArr.fill(0);
		densArr.fill(0);

		// Every group this organ shows (its type, or the page's own label) is
		// one big round island; its memories are orbs on it, newest at its heart.
		const shown = answer.slice(0, MAX_ANSWER);
		const times = shown.map((m) => Date.parse(m.createdAt));
		// The room islands may use: right of the dock, left of any block of
		// words that stands beside the field.
		{
			const bandTop = RIVER_Y + RIVER_HALF_H;
			const bandBottom = RIVER_Y - RIVER_HALF_H;
			const cssW = Math.max(1, engine.canvasElement.clientWidth || engine.canvasElement.width);
			const inset = insetPx > 0 ? (insetPx / cssW) * 2 - 1 : -0.86;
			let x0 = Math.max(-0.92, inset);
			let x1 = 0.94;
			for (const q of quiet) {
				const besideBand = q.y1 > bandBottom && q.y0 < bandTop;
				const tall = q.y1 - q.y0 > 0.5;
				if (besideBand && tall && q.x0 > 0) x1 = Math.min(x1, q.x0 - 0.05);
			}
			if (x1 - x0 < 0.6) x1 = 0.94;
			spanX0 = x0 * A;
			spanX1 = x1 * A;
		}

		const keyOf = (m: TraceMemory) => m.laneLabel ?? m.type;
		const laneSort = new Map<string, number>();
		for (const m of shown) {
			const k = keyOf(m);
			if (laneSort.has(k)) continue;
			const ti = TYPE_ORDER.indexOf(m.type);
			laneSort.set(k, m.lane !== undefined ? -m.lane : 10 + (ti >= 0 ? ti : 20));
		}
		laneKeys = [...laneSort.entries()]
			.sort((a, b) => a[1] - b[1])
			.map(([k]) => k)
			.slice(0, MAX_ISLANDS);
		const laneIndex = new Map(laneKeys.map((k, i) => [k, i]));
		const n = Math.max(1, laneKeys.length);
		let bandTop = RIVER_Y + RIVER_HALF_H * 0.9;
		let bandBottom = RIVER_Y - RIVER_HALF_H * 0.9;
		// A wide block of words above or below the field (a phone's header, its
		// reading lane underneath) pushes the islands clear of it.
		{
			const sx0 = spanX0 / A;
			const sx1 = spanX1 / A;
			let top = bandTop;
			let bottom = bandBottom;
			for (const q of quiet) {
				const across = Math.min(q.x1, sx1) - Math.max(q.x0, sx0);
				if (across < 0.5 * (sx1 - sx0)) continue;
				if ((q.y0 + q.y1) / 2 > RIVER_Y) top = Math.min(top, q.y0 - 0.06);
				else bottom = Math.max(bottom, q.y1 + 0.06);
			}
			if (top - bottom > 0.22) {
				bandTop = top;
				bandBottom = bottom;
			}
		}
		// An island of one kind wears that kind's colour; one of many kinds (a
		// day, an agent) wears one quiet colour, so colour always means kind.
		laneColors = laneKeys.map((k) => {
			const kinds = new Set(shown.filter((m) => keyOf(m) === k).map((m) => m.type));
			return kinds.size === 1 ? typeColor([...kinds][0]) : [0.5, 0.82, 0.84];
		});
		laneCounts = laneKeys.map(() => 0);
		for (const m of shown) {
			const li = laneIndex.get(keyOf(m));
			if (li !== undefined) laneCounts[li]++;
		}
		clusterHost = new Map();
		// Oldest first.
		const byTime = [...shown.keys()].sort((a, b) => (times[a] || 0) - (times[b] || 0));
		const membersOf = (key: string) => byTime.filter((i) => keyOf(shown[i]) === key);
		dialMode = axis === 'day' || axis === 'week';
		const fracOf = (i: number) =>
			!Number.isFinite(times[i]) ? hash01(shown[i].id) : axis === 'week' ? inWeek(times[i]) : inDay(times[i]);
		islands = [];
		zoomed = null;
		dialTicks = [];
		const zi = focusLane !== null ? laneIndex.get(focusLane) : undefined;
		const place = (i: number, x: number, y: number, size: number, lane: string) =>
			placedList.push({ id: shown[i].id, x, y, size, kind: 'answer', depth: 0, lane });
		// The picked memory always has an orb, even on a crowded island.
		const picked = selectedId ? shown.findIndex((m) => m.id === selectedId) : -1;
		const keepPicked = (chosen: number[], members: number[]) =>
			picked >= 0 && chosen.length && !chosen.includes(picked) && members.includes(picked) ? [...chosen.slice(0, -1), picked] : chosen;

		if (zi !== undefined && focusLane !== null) {
			// One island, opened: it fills the field under its heading, every
			// memory in it a big orb with room round it, a page at a time.
			const members = membersOf(focusLane);
			const nM = members.length;
			// The heading wraps to two lines on a phone (its name, then its pages).
			const head = A < 1 ? 0.27 : 0.13;
			const top = bandTop - head;
			const cx = (spanX0 + spanX1) / 2;
			const cy = (top + bandBottom) / 2;
			const rx = (spanX1 - spanX0) / 2;
			const ry = (top - bandBottom) / 2;
			const color = laneColors[zi];
			// A phone is too narrow for a big clock: an opened day there uses the
			// whole width, in order, like any other island.
			if (dialMode && A >= 1) {
				// A day (or week) opened: one big clock, every write at its hour.
				const R = Math.min(rx, ry);
				const orb = Math.min(ORB * 1.05, Math.max(A < 1 ? 0.024 : ORB_MIN, 0.08 * R));
				const rings = dialRings(R, orb);
				const cap = Math.max(1, rings.reduce((t, r) => t + r.slots, 0));
				pages = Math.max(1, Math.ceil(nM / cap));
				page = Math.min(page, pages - 1);
				// Page 0 is the newest stretch. The opened clock runs over this
				// page's own stretch of time, from its first write at the top round
				// to its last, so a busy hour opens out instead of piling up; the
				// quarters are labelled with their real times.
				const end = nM - page * cap;
				const chunk = keepPicked(members.slice(Math.max(0, end - cap), end), members).sort((a, b) => (times[a] || 0) - (times[b] || 0));
				const known = chunk.map((i) => times[i]).filter(Number.isFinite);
				const from = known.length ? Math.min(...known) : 0;
				const to = known.length ? Math.max(...known) : 0;
				const span = to - from;
				const SWEEP = 0.94;
				const winFrac = (i: number) => {
					const t = times[i];
					if (!Number.isFinite(t) || span <= 0) return (chunk.indexOf(i) / Math.max(1, chunk.length)) * SWEEP;
					return ((t - from) / span) * SWEEP;
				};
				for (const pl of onDial(chunk, rings, cx, cy, winFrac)) place(pl.i, pl.x, pl.y, orb, focusLane);
				islands.push({ key: focusLane, x: cx, y: cy, rx: R, ry: R, cell: 2 * R, color, count: nM, shown: chunk.length });
				const labelR = R + orb * 1.4;
				const oneDay = span < 36 * 3_600_000;
				for (const f of [0, 0.25, 0.5, 0.75]) {
					const a = f * 2 * Math.PI;
					const at = new Date(from + (f / SWEEP) * span);
					const label =
						span <= 0
							? ''
							: oneDay
								? at.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
								: at.toLocaleDateString([], { month: 'short', day: 'numeric' });
					// A stretch inside one minute names its moment once.
					if (label && !dialTicks.some((tk) => tk.label === label)) dialTicks.push({ x: cx + labelR * Math.sin(a), y: cy + labelR * Math.cos(a), label });
				}
			} else {
				// Big enough to read and hit even on a phone; more pages rather than smaller orbs.
				const orb = Math.min(ZOOM_ORB, Math.max(A < 1 ? 0.026 : ORB_MIN, 0.09 * Math.min(rx, ry)));
				const seats = honeycomb(cx, cy, rx - orb * 1.5, ry - orb * 1.5, ORB_GAP * orb);
				const cap = Math.max(1, seats.length);
				pages = Math.max(1, Math.ceil(nM / cap));
				page = Math.min(page, pages - 1);
				// Newest at the heart of the page.
				const newest = [...members].reverse();
				const chunk = keepPicked(newest.slice(page * cap, page * cap + cap), members);
				chunk.forEach((i, k) => place(i, seats[k].x, seats[k].y, orb, focusLane!));
				islands.push({ key: focusLane, x: cx, y: cy, rx, ry, cell: 2 * rx, color, count: nM, shown: chunk.length });
			}
			const ts = members.map((i) => times[i]).filter(Number.isFinite);
			zoomed = {
				key: focusLane,
				count: nM,
				color,
				from: ts.length ? Math.min(...ts) : 0,
				to: ts.length ? Math.max(...ts) : 0,
				top: bandTop,
				x: cx,
				page,
				pages,
				shown: islands[0].shown
			};
		} else {
			page = 0;
			pages = 1;
			// The overview: the islands on a grid shaped to the field, as big as
			// it allows, each with room under it for its name.
			const W = spanX1 - spanX0;
			const H = bandTop - bandBottom;
			const labelH = A < 1 ? 0.12 : LABEL_H;
			// Every island's cell is wide enough for its name to read.
			const minCell = A < 1 ? 0.2 : 0.3;
			let grid = { rows: n, cols: 1, R: 0 };
			for (let rows = 1; rows <= n; rows++) {
				const cols = Math.ceil(n / rows);
				if (W / cols < minCell && rows < n) continue;
				const R = Math.min((W / cols) * 0.5 * 0.86, (H / rows - labelH) * 0.5 * 0.9);
				if (R > grid.R) grid = { rows, cols, R };
			}
			const Rmax = Math.max(0.035, Math.min(grid.R, A < 1 ? 0.3 : 0.42));
			const cw = W / grid.cols;
			// Rows only as tall as their islands need, the grid centred in the field.
			const rowH = Math.min(H / grid.rows, (2 * Rmax) / 0.9 + labelH);
			const gridTop = (bandTop + bandBottom) / 2 + (rowH * grid.rows) / 2;
			const maxCount = Math.max(1, ...laneCounts);
			laneKeys.forEach((key, li) => {
				const row = Math.floor(li / grid.cols);
				const inRow = Math.min(grid.cols, laneKeys.length - row * grid.cols);
				const col = li % grid.cols;
				const x = spanX0 + (W - inRow * cw) / 2 + (col + 0.5) * cw;
				const y = gridTop - row * rowH - (rowH - labelH) / 2;
				// Bigger groups are bigger islands; the smallest is still easy to hit.
				const r = Rmax * (0.76 + 0.24 * Math.sqrt(laneCounts[li] / maxCount));
				const orb = Math.min(ORB, Math.max(ORB_MIN, r * 0.24));
				const members = membersOf(key);
				let count = 0;
				if (dialMode) {
					const rings = dialRings(r, orb);
					const cap = rings.reduce((t, q) => t + q.slots, 0);
					const chosen = keepPicked(evenly(members, cap), members);
					const seated = onDial(chosen, rings, x, y, fracOf, true);
					for (const pl of seated) place(pl.i, pl.x, pl.y, orb, key);
					count = seated.length;
				} else {
					const gap = ORB_GAP * orb;
					const inner = r - orb * 1.6;
					const cap = Math.max(1, Math.floor(((Math.PI * inner * inner) / (gap * gap)) * 0.86));
					const chosen = keepPicked([...members].reverse().slice(0, cap), members);
					sunflower(chosen.length, x, y, inner, hash01(key) * 6.283).forEach((pt, k) => place(chosen[k], pt.x, pt.y, orb, key));
					count = chosen.length;
				}
				islands.push({ key, x, y, rx: r, ry: r, cell: cw, color: laneColors[li], count: laneCounts[li], shown: count });
			});
		}
		writeIslands();

		// The walk stands in rings round the picked memory, one ring per depth,
		// each cause coloured by the recorded edge that reached it.
		const byId = new Map(placedList.map((p) => [p.id, p]));
		const origin0 = selectedId ? byId.get(selectedId) : undefined;
		const walkColor = new Map<string, [number, number, number]>();
		const causeIds = new Set<string>();
		if (walk && origin0) {
			for (const n of walk.nodes) if (n.id !== origin0.id) causeIds.add(n.id);
			const nodes = walk.nodes
				.filter((n) => n.id !== origin0.id && !byId.has(n.id))
				.slice(0, Math.min(SKY_RESERVE, MAX_MEMS - placedList.length));
			const byDepth = new Map<number, typeof nodes>();
			for (const n of nodes) {
				const d = Math.max(1, n.depth);
				byDepth.set(d, [...(byDepth.get(d) ?? []), n]);
				walkColor.set(n.id, edgeColor(n.path[n.path.length - 1]?.link_type ?? 'touched'));
			}
			// The causes ring the picked memory: the nearest depth closest, each
			// ring as full as room allows before the next one opens further out.
			const depths = [...byDepth.keys()].sort((x, y) => x - y);
			const gap = ORB_GAP * ORB;
			let ringR = Math.max(origin0.size * 3.2, 0.11);
			const xLo = -0.96 * A;
			const xHi = 0.96 * A;
			for (const d of depths) {
				const list = byDepth.get(d)!;
				let i = 0;
				while (i < list.length) {
					const cap = Math.max(6, Math.floor((2 * Math.PI * ringR) / gap));
					const take = list.slice(i, i + cap);
					const step = (2 * Math.PI) / cap;
					take.forEach((node, k) => {
						// Centred on the top of the ring, spreading round it.
						const a = Math.PI / 2 + (k - (take.length - 1) / 2) * step;
						const x = Math.min(xHi, Math.max(xLo, origin0.x + ringR * Math.cos(a)));
						const y = Math.min(0.94, Math.max(-0.94, origin0.y + ringR * Math.sin(a)));
						const p: PlacedMemory = { id: node.id, x, y, size: ORB, kind: 'walk', depth: d };
						placedList.push(p);
						byId.set(node.id, p);
					});
					i += cap;
					ringR += gap;
				}
				ringR += gap * 0.35;
			}
			// Streams: every recorded edge the walk crossed, cause into effect.
			// Depth comes from the walk itself, so a cause that also sits on the
			// river still flows into the memory it explains.
			const walkDepth = new Map(walk.nodes.map((n) => [n.id, n.depth]));
			const depthOf = (id: string) => (id === origin0.id ? 0 : (walkDepth.get(id) ?? 99));
			const seen = new Set<string>();
			edgeCount = 0;
			for (const n of walk.nodes) {
				for (const e of n.path) {
					const k = `${e.source_id}|${e.link_type}|${e.target_id}`;
					if (seen.has(k) || edgeCount >= MAX_EDGES) continue;
					seen.add(k);
					const a = byId.get(e.source_id);
					const b = byId.get(e.target_id);
					if (!a || !b) continue;
					const [from, to] = depthOf(a.id) >= depthOf(b.id) ? [a, b] : [b, a];
					const causal = isCausalEdge(e.link_type);
					// A soft curve from the cause into the effect.
					const bow = 0.14;
					edgeArr.set(
						[from.x, from.y, to.x, to.y, causal ? 1 : 0, causal ? 1 : 0.5, 0.15 + Math.max(0, depthOf(from.id) - 1) * 0.3, bow, ...edgeColor(e.link_type), 0],
						edgeCount * EDGE_STRIDE
					);
					edgeCount++;
				}
			}
		} else {
			edgeCount = 0;
		}

		// Recorded links that came with the answer: arcs between the orbs, on
		// the answer's clock, cascading in once the eruption settles.
		links.forEach((l, i) => {
			if (edgeCount >= MAX_EDGES) return;
			const a = byId.get(l.source);
			const b = byId.get(l.target);
			if (!a || !b || a.kind !== 'answer' || b.kind !== 'answer' || a.id === b.id) return;
			const [from, to] = a.x <= b.x ? [a, b] : [b, a];
			// An arc between them, bowing up, so links between islands never lie flat.
			const bow = from.lane === to.lane ? 0.3 : 0.18;
			edgeArr.set([from.x, from.y, to.x, to.y, 1, 1, 1.5 + i * 0.02, bow, ...edgeColor(l.type), 1], edgeCount * EDGE_STRIDE);
			edgeCount++;
		});

		// Islands have no straight clock to draw a seam on.
		nowX = -99;

		// Galaxies grow when there are few of them and shrink when there are many.
		perMem = Math.max(1, Math.floor(MEM_POOL / (shown.length + SKY_RESERVE)));

		// Upload memory slots.
		const memById = new Map(answer.map((m) => [m.id, m]));
		placedList.slice(0, MAX_MEMS).forEach((p, i) => {
			const mem = memById.get(p.id);
			const [r, g, b] = mem ? typeColor(mem.type) : (walkColor.get(p.id) ?? [1.0, 0.8, 0.45]);
			const retention = mem ? Math.min(1, Math.max(0, mem.retention)) : 0.85;
			const shape = p.cluster ? SHAPE_CLUSTER : (SHAPE[mem?.type ?? ''] ?? 0);
			const flags =
				(p.kind === 'answer' ? FLAG_ANSWER : FLAG_WALK) |
				(mem?.future ? FLAG_FUTURE : 0) |
				(p.id === selectedId || p.cluster?.includes(selectedId ?? '') ? FLAG_PICKED : 0) |
				(p.kind === 'answer' && causeIds.has(p.id) ? FLAG_CAUSE : 0) |
				(p.cluster ? FLAG_CLUSTER : 0) |
				(shape << 8);
			memArr.set([p.x, p.y, p.size, retention, r, g, b, flags], i * 8);
		});
		device!.queue.writeBuffer(memBuf, 0, memArr);
		device!.queue.writeBuffer(edgeBuf, 0, edgeArr);
		dirty = false;
	}

	/** The islands for the backdrop and the halos: x, y, rx, ry, rgb, lit. */
	function writeIslands() {
		densArr.fill(0);
		islands.slice(0, ISLAND_SLOTS).forEach((I, k) => {
			densArr.set([I.x, I.y, I.rx, I.ry, I.color[0], I.color[1], I.color[2], I.key === hoverLane && !zoomed ? 1 : 0], k * 8);
		});
		device!.queue.writeBuffer(densBuf, 0, densArr);
	}

	function writeUniforms() {
		const A = aspectNow();
		if (dirty || Math.abs(A - lastAspect) > 1e-3) layout();
		// Real time even when frames are dropped: the elapsed sim time is split
		// into up to 8 stable sub-steps instead of playing in slow motion.
		// simSeconds moves between 60 Hz ticks, so a 120 Hz display gets new motion every frame.
		const total = engine.simSeconds;
		const elapsed = lastTotal < 0 ? 0 : Math.min(0.25, Math.max(0, total - lastTotal));
		lastTotal = total;
		substeps = Math.min(8, Math.max(1, Math.ceil(elapsed / (1 / 30))));
		const dt = elapsed / substeps;
		const t = now();
		const at = (id: string | null) => {
			if (!id) return -1;
			const i = placedList.findIndex((p) => p.id === id);
			if (i >= 0) return i;
			const host = clusterHost.get(id);
			return host ? placedList.indexOf(host) : -1;
		};
		const sel = at(selectedId);
		const hov = at(hoverId);
		const sp = sel >= 0 ? placedList[sel] : null;
		// Fly toward the picked memory, setting it left of the reading lane
		// (or above the sheet on a phone); ease back out when nothing is picked.
		const target = sp
			? (() => {
					const [nx, ny] = A < 1 ? [0, 0.18] : [-0.3, 0.02];
					return { x: sp.x - (nx * A) / ZOOM_IN, y: sp.y - ny / ZOOM_IN, z: ZOOM_IN };
				})()
			: { x: 0, y: 0, z: 1 };
		const ease = 1 - Math.exp(-elapsed * 3.2);
		cam.x += (target.x - cam.x) * ease;
		cam.y += (target.y - cam.y) * ease;
		cam.z += (target.z - cam.z) * ease;
		// The cursor's velocity eases to rest when it stops moving.
		if (performance.now() - cursorAt > 60) {
			cursorVel = { x: cursorVel.x * 0.85, y: cursorVel.y * 0.85 };
		}
		const river = riverFor(A);
		tuArr.set([A, Math.min(MAX_MEMS, placedList.length), edgeCount, river.y], 0);
		tuArr.set([sel, t - igniteAt, t - walkAt, t - answerAt], 4);
		tuArr.set([hov, 0.9 * A, t - hoverAt, dt], 8);
		tuArr.set([sp?.x ?? 0, sp?.y ?? 0, origin.x * A, origin.y], 12);
		// The cursor in world space, through the camera.
		tuArr.set([((cursor?.x ?? 0) * A) / cam.z + cam.x, (cursor?.y ?? 0) / cam.z + cam.y, cursorVel.x / cam.z, cursorVel.y / cam.z], 16);
		tuArr.set([verifyAt >= 0 ? t - verifyAt : -1, sealedAt >= 0 ? t - sealedAt : -1, sealOk, river.halfH], 20);
		tuArr.set([answerGen, walkGen, sel >= 0 ? 1 : 0, cursor ? 1 : 0], 24);
		tuArr.set([nowX, MEM_POOL, perMem, BASE_PER_MEM], 28);
		const hp = hov >= 0 ? placedList[hov] : null;
		tuArr.set([hp?.x ?? 0, hp?.y ?? 0, Math.hypot(cursorVel.x, cursorVel.y), 0], 32);
		const [deep, bright, hot] = STORY_PALETTE[story];
		// The handoff's hold ends a moment after the organ's data lands, so the
		// picture is seen before it reorganizes.
		const holding = handoffState === 'holding' && (holdUntil === 0 || t < holdUntil);
		if (handoffState === 'holding' && holdUntil > 0 && t >= holdUntil) handoffState = 'released';
		tuArr.set([STORY_ID[story], handoffMode, holding ? 1 : 0, handoffCount], 36);
		// u18: islands (count, clocks 1/0, opened 1/0, a clock's marks: 24 hours or 7 days)   u19: orb radius, device pixel ratio, span x0, span x1
		const cnv = engine.canvasElement;
		const dpr = cnv.clientWidth > 0 ? cnv.width / cnv.clientWidth : 1;
		tuArr.set([Math.min(islands.length, ISLAND_SLOTS), dialMode ? 1 : 0, zoomed ? 1 : 0, dialMode ? (axis === 'week' ? 7 : 24) : 0], 72);
		tuArr.set([ORB, dpr, spanX0, spanX1], 76);
		// u14..u17: the page's blocks of words (NDC), where the field goes quiet.
		for (let k = 0; k < MAX_QUIET; k++) {
			const q = quiet[k];
			tuArr.set(q ? [q.x0, q.y0, q.x1, q.y1] : [0, 0, 0, 0], 56 + k * 4);
		}
		tuArr.set([...deep, 0], 40);
		tuArr.set([...bright, 0], 44);
		tuArr.set([...hot, 0], 48);
		tuArr.set([cam.x, cam.y, cam.z, 0], 52);
		device!.queue.writeBuffer(tuBuf, 0, tuArr);
	}

	return {
		setAnswer(mems, r, o) {
			answer = mems;
			range = r;
			selectedId = null;
			walk = null;
			verifyAt = -1;
			sealedAt = -1;
			if (o) origin = o;
			answerAt = now();
			answerGen++;
			if (handoffState === 'pending') {
				// The first answer after a Palace click: start on its points.
				handoffMode = 1;
				handoffState = 'holding';
				// The second explosion plays at once; the layout takes it as soon as the data is here.
				holdUntil = mems.length ? now() + 0.15 : 0;
			} else if (handoffState === 'holding') {
				// The organ's data: release from where the picture is.
				handoffMode = 2;
				holdUntil = now() + 0.15;
			} else if (continuing) {
				// Another organ took over: keep every particle where it is and
				// let it spring into this organ's layout. An empty answer (the
				// page still loading) keeps the flag for the real one.
				handoffMode = 2;
				if (mems.length) continuing = false;
			} else {
				handoffMode = 0;
			}
			dirty = true;
			engine.requestRender();
		},
		continueNext() {
			continuing = true;
		},
		beginHandoff(points, count) {
			const n = Math.min(MAX_HANDOFF, count);
			device!.queue.writeBuffer(handBuf, 0, points.subarray(0, n * HANDOFF_STRIDE));
			handoffCount = n;
			handoffState = 'pending';
		},
		setSelected(id) {
			if (id === selectedId) return;
			selectedId = id;
			walk = null;
			verifyAt = -1;
			sealedAt = -1;
			igniteAt = now();
			dirty = true;
		},
		setWalk(w) {
			walk = w;
			walkAt = now();
			walkGen++;
			dirty = true;
		},
		setLinks(l) {
			links = l;
			dirty = true;
		},
		setNow(ms) {
			nowMs = ms;
			dirty = true;
		},
		setStory(st) {
			story = st;
		},
		setAxis(a) {
			if (a === axis) return;
			axis = a;
			dirty = true;
		},
		setHover(id) {
			if (id === hoverId) return;
			hoverId = id;
			hoverAt = now();
		},
		setVerify(state) {
			if (state === 'idle') {
				verifyAt = -1;
				sealedAt = -1;
			} else if (state === 'running') {
				verifyAt = now();
				sealedAt = -1;
			} else {
				verifyAt = -1;
				sealedAt = now();
				sealOk = state === 'ok' ? 1 : 0;
			}
		},
		setCursor(c) {
			const at = performance.now();
			if (c && cursor) {
				const dtc = Math.max(0.008, (at - cursorAt) / 1000);
				const A = aspectNow();
				const vx = ((c.x - cursor.x) * A) / dtc;
				const vy = (c.y - cursor.y) / dtc;
				cursorVel = { x: cursorVel.x * 0.5 + vx * 0.5, y: cursorVel.y * 0.5 + vy * 0.5 };
			} else {
				cursorVel = { x: 0, y: 0 };
			}
			cursor = c;
			cursorAt = at;
		},
		pickAt(ndcX, ndcY) {
			const A = aspectNow();
			const wx = (ndcX * A) / cam.z + cam.x;
			const wy = ndcY / cam.z + cam.y;
			// Magnetic: an orb takes the pointer a little beyond its edge and
			// keeps it further out, so the cursor never flickers between neighbours.
			const cnv = engine.canvasElement;
			const pxWorld = 2 / Math.max(1, cnv.clientHeight || cnv.height) / cam.z;
			let best: PlacedMemory | null = null;
			let bestD = Infinity;
			for (const p of placedList) {
				const d = Math.hypot(wx - p.x, wy - p.y);
				const reach = Math.max(p.size * 1.25, 16 * pxWorld) + (p.id === hoverId ? 10 : 0) * pxWorld;
				if (d < reach && d < bestD) {
					best = p;
					bestD = d;
				}
			}
			return best;
		},
		ndcOf(id) {
			const p = placedList.find((q) => q.id === id) ?? clusterHost.get(id);
			if (!p) return null;
			return { x: ((p.x - cam.x) * cam.z) / aspectNow(), y: (p.y - cam.y) * cam.z };
		},
		placed: () => placedList,
		setFocusLane(key) {
			if (key === focusLane) return;
			focusLane = key;
			page = 0;
			dirty = true;
			engine.requestRender();
		},
		setPage(p) {
			const next = Math.max(0, Math.floor(p));
			if (next === page) return;
			page = next;
			dirty = true;
			engine.requestRender();
		},
		setHoverLane(key) {
			if (key === hoverLane) return;
			hoverLane = key;
			writeIslands();
		},
		zoom: () =>
			zoomed ? { ...zoomed, top: (zoomed.top - cam.y) * cam.z, x: ((zoomed.x - cam.x) * cam.z) / aspectNow() } : null,
		laneAt(ndcX, ndcY) {
			if (zoomed || !islands.length) return null;
			const A = aspectNow();
			const wx = (ndcX * A) / cam.z + cam.x;
			const wy = ndcY / cam.z + cam.y;
			for (const I of islands) {
				if (Math.hypot((wx - I.x) / I.rx, (wy - I.y) / I.ry) <= 1.12) return I.key;
			}
			return null;
		},
		lanes() {
			if (zoomed) return [];
			const A = aspectNow();
			return islands.map((I) => ({
				label: I.key,
				x: ((I.x - cam.x) * cam.z) / A,
				y: (I.y - cam.y) * cam.z,
				r: I.ry * cam.z,
				w: (I.cell * cam.z) / A,
				count: I.count,
				shown: I.shown,
				color: I.color
			}));
		},
		setInset(px) {
			if (Math.abs(px - insetPx) < 1) return;
			insetPx = px;
			dirty = true;
		},
		memberAt(p) {
			return p.id;
		},
		setQuiet(rects) {
			const next = rects.slice(0, MAX_QUIET);
			const same =
				next.length === quiet.length &&
				next.every((r, i) => Math.abs(r.x0 - quiet[i].x0) + Math.abs(r.y0 - quiet[i].y0) + Math.abs(r.x1 - quiet[i].x1) + Math.abs(r.y1 - quiet[i].y1) < 0.01);
			if (same) return;
			quiet = next;
			dirty = true;
		},
		timeTicks() {
			const A = aspectNow();
			const floorY = -0.86;
			if (!zoomed || !dialTicks.length) return { ticks: [], floorY };
			// An opened clock names its hours (or its days) round its rim.
			return {
				ticks: dialTicks.map((tk) => ({ x: ((tk.x - cam.x) * cam.z) / A, y: (tk.y - cam.y) * cam.z, label: tk.label })),
				floorY
			};
		},
		highRefresh: true,
		debug() {
			return {
				placed: placedList.length,
				edgeCount,
				edges: Array.from(edgeArr.slice(0, edgeCount * EDGE_STRIDE)),
				flags: placedList.map((_, i) => memArr[i * 8 + 7])
			};
		},
		compute(encoder: GPUCommandEncoder) {
			writeUniforms();
			const cp = encoder.beginComputePass({ label: 'trace-sim' });
			cp.setPipeline(simPipe);
			cp.setBindGroup(0, simBG);
			for (let k = 0; k < substeps; k++) cp.dispatchWorkgroups(Math.ceil(TOTAL / 256));
			cp.end();
		},
		render(pass: GPURenderPassEncoder) {
			pass.setPipeline(backPipe);
			pass.setBindGroup(0, backBG);
			pass.draw(3);
			pass.setPipeline(partPipe);
			pass.setBindGroup(0, partBG);
			pass.draw(6, TOTAL);
			const count = Math.min(MAX_MEMS, placedList.length);
			if (count > 0) {
				pass.setPipeline(markPipe);
				pass.setBindGroup(0, markBG);
				pass.draw(6, count);
			}
		},
		dispose() {
			tuBuf.destroy();
			memBuf.destroy();
			edgeBuf.destroy();
			densBuf.destroy();
			handBuf.destroy();
			partBuf.destroy();
			stateBuf.destroy();
		}
	};
}
