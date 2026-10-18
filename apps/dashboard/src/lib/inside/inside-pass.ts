/**
 * One memory, inside — the WebGPU pass.
 *
 * Owns about 128k particles that draw a single memory: its own form (a
 * superformula surface from its type and id hash), the receipt ring, one
 * stream per recorded edge out to a moon for each neighbour, and a haze. The
 * page hands it real data (memory get / state, ghostlink map, receipt get);
 * nothing here is invented, and the shape's likeness to another shape carries
 * no meaning (it comes from a hash of the id).
 *
 * The visual owns one region of the screen (the left on a wide screen, the
 * top on a phone) and never draws outside it, so the words beside it are
 * never covered.
 */
import type { FramePass, ObservatoryEngine } from '$lib/observatory/engine';
import { HANDOFF_STRIDE, MAX_HANDOFF } from '$lib/trace/handoff';
import { edgeColor, typeColor, type VerifyState } from '$lib/trace/trace-pass';
import { isCausalEdge } from '$lib/trace/trace-data';
import { INSIDE_BACKDROP_WGSL, INSIDE_COMPUTE_WGSL, INSIDE_PARTICLES_WGSL } from './inside-wgsl';

const TOTAL = 128_000;
const MAX_MOONS = 24;
const MOON_FLOATS = 12;
const UNIFORM_VEC4 = 11;

const ADDITIVE: GPUBlendState = {
	color: { srcFactor: 'one', dstFactor: 'one', operation: 'add' },
	alpha: { srcFactor: 'one', dstFactor: 'one', operation: 'add' }
};

export interface InsideMemory {
	id: string;
	type: string;
	retention: number;
	/** FSRS stability, days. */
	stability?: number;
	/** Length of the memory's words. */
	length?: number;
}
export interface InsideMoon {
	id: string;
	type: string;
	edge: string;
	outgoing: boolean;
	label: string;
}
export type InsidePick = { kind: 'form' } | { kind: 'ring' } | { kind: 'moon'; index: number; id: string };
export interface NdcRect {
	left: number;
	right: number;
	top: number;
	bottom: number;
}

export interface InsidePass extends FramePass {
	/** A new memory arrives: a new generation that assembles from its source. */
	arrive(id: string): void;
	/** The memory's data, as it loads (type, retention, stability, length). */
	setMemory(m: InsideMemory): void;
	/** Its neighbours by recorded edge (the first MAX_MOONS, causal first). */
	setMoons(list: InsideMoon[]): void;
	moons(): InsideMoon[];
	setVerify(v: VerifyState): void;
	setCursor(ndc: { x: number; y: number } | null): void;
	setHover(h: number | 'form' | null): void;
	pickAt(ndcX: number, ndcY: number): InsidePick | null;
	moonNdc(i: number): { x: number; y: number; r: number } | null;
	formNdc(): { x: number; y: number; r: number };
	/** The receipt ring's centre (NDC) and radius (world). */
	ringNdc(): { x: number; y: number; r: number };
	/** The part of the screen the visual owns (NDC). */
	region(): NdcRect;
	/** The next arrival starts on these points (a picture handed over by the page that was clicked). */
	beginHandoff(points: Float32Array<ArrayBuffer>, count: number): void;
	/** The next arrival starts from this screen point (a moon walked to). */
	arriveFrom(ndc: { x: number; y: number }): void;
	dispose(): void;
}

/** The visual's own region of the screen, in NDC: never under the words or the dock. */
export function insideRegion(aspect: number): NdcRect {
	return aspect >= 1
		? { left: -0.86, right: 0.16, top: 0.86, bottom: -0.9 }
		: { left: -0.94, right: 0.94, top: 0.9, bottom: 0.04 };
}

function hashUnits(id: string, count = 8): number[] {
	const out: number[] = [];
	let h = 2166136261 >>> 0;
	for (let k = 0; k < count; k++) {
		for (let i = 0; i < id.length; i++) {
			h ^= id.charCodeAt(i) + k * 131;
			h = Math.imul(h, 16777619) >>> 0;
		}
		h ^= h >>> 13;
		h = Math.imul(h, 0x5bd1e995) >>> 0;
		h ^= h >>> 15;
		out.push((h >>> 0) / 4294967295);
	}
	return out;
}
const pick = <T>(arr: T[], u: number): T => arr[Math.min(arr.length - 1, Math.floor(u * arr.length))];

interface FormParams {
	a: [number, number, number, number];
	b: [number, number, number, number];
	twist: number;
}

/**
 * The family of the form comes from the memory's type, its exact shape from
 * its id hash: a decision is crystalline, a fact round and whole, a correction
 * a twisted star, an event a burst, a pattern a many-lobed bloom, a note soft.
 */
export function formParams(type: string, id: string): FormParams {
	const h = hashUnits(id);
	switch (type) {
		case 'decision':
			return {
				a: [pick([4, 6, 8], h[0]), 6 + h[1] * 24, 4 + h[2] * 14, 4 + h[3] * 14],
				b: [pick([2, 4], h[4]), 5 + h[5] * 20, 4 + h[6] * 12, 4 + h[6] * 12],
				twist: 0
			};
		case 'fact':
			return {
				a: [pick([0, 3, 4, 5], h[0]), 2 + h[1] * 3, 2 + h[2] * 2.5, 2 + h[2] * 2.5],
				b: [pick([0, 2], h[4]), 2 + h[5] * 2, 2, 2],
				twist: 0
			};
		case 'correction':
			return {
				a: [pick([3, 5, 7], h[0]), 0.35 + h[1] * 0.6, 1 + h[2] * 1.4, 1 + h[3] * 1.4],
				b: [pick([2, 3], h[4]), 1.2 + h[5] * 1.5, 1.4, 1.4],
				twist: 0.6 + h[6] * 1.4
			};
		case 'event':
			return {
				a: [pick([7, 9, 11, 13], h[0]), 0.22 + h[1] * 0.3, 0.9 + h[2] * 1.2, 0.9 + h[3] * 1.2],
				b: [pick([3, 5, 7], h[4]), 0.3 + h[5] * 0.5, 1 + h[6], 1 + h[6]],
				twist: h[7] * 0.6
			};
		case 'pattern':
			return {
				a: [pick([8, 10, 12, 16], h[0]), 1.2 + h[1] * 2.2, 1 + h[2] * 1.4, 1 + h[3] * 1.4],
				b: [pick([2, 4, 6], h[4]), 1.4 + h[5] * 2, 1.2 + h[6], 1.2 + h[6]],
				twist: 0.2 + h[7] * 0.6
			};
		case 'note':
			return {
				a: [pick([2, 3], h[0]), 1 + h[1] * 1.2, 1.4 + h[2] * 1.4, 1.4 + h[3] * 1.4],
				b: [pick([0, 2], h[4]), 1.2 + h[5], 1.6, 1.6],
				twist: h[6] * 0.4
			};
		default:
			return {
				a: [pick([4, 5, 6], h[0]), 1.5 + h[1] * 3, 2 + h[2] * 3, 2 + h[3] * 3],
				b: [pick([2, 3, 4], h[4]), 2 + h[5] * 3, 2 + h[6] * 2, 2 + h[6] * 2],
				twist: h[7] * 0.5
			};
	}
}

function sf(a: number, m: number, n1: number, n2: number, n3: number): number {
	const t1 = Math.pow(Math.abs(Math.cos((m * a) / 4)), n2);
	const t2 = Math.pow(Math.abs(Math.sin((m * a) / 4)), n3);
	return Math.pow(Math.max(t1 + t2, 1e-5), -1 / n1);
}

/** 1 / the form's reach, so every memory's form fills the same radius. */
function normaliser(p: FormParams): number {
	const radii: number[] = [];
	for (let i = 0; i < 4096; i++) {
		const u = ((i * 0.6180339887) % 1) * 2 - 1;
		const v = ((i * 0.7548776662) % 1) * 2 - 1;
		const th = u * Math.PI;
		const ph = Math.asin(v);
		const r1 = sf(th + p.twist * ph, ...p.a);
		const r2 = sf(ph, ...p.b);
		radii.push(Math.hypot(r1 * Math.cos(th) * r2 * Math.cos(ph), r2 * Math.sin(ph), r1 * Math.sin(th) * r2 * Math.cos(ph)));
	}
	radii.sort((a, b) => a - b);
	const reach = radii[Math.floor(radii.length * 0.995)];
	return reach > 1e-6 && Number.isFinite(reach) ? 1 / reach : 1;
}

export function createInsidePass(engine: ObservatoryEngine): InsidePass {
	const device = engine.gpuDevice!;
	const params = engine.paramsBuffer!;

	const uArr = new Float32Array(UNIFORM_VEC4 * 4);
	const moonArr = new Float32Array(MAX_MOONS * MOON_FLOATS);
	const uBuf = device.createBuffer({ label: 'inside-u', size: uArr.byteLength, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
	const moonBuf = device.createBuffer({ label: 'inside-moons', size: moonArr.byteLength, usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST });
	const stateBuf = device.createBuffer({ label: 'inside-state', size: TOTAL * 32, usage: GPUBufferUsage.STORAGE });
	const handBuf = device.createBuffer({
		label: 'inside-handoff',
		size: MAX_HANDOFF * HANDOFF_STRIDE * 4,
		usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST
	});

	const C = GPUShaderStage.COMPUTE;
	const VF = GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT;
	const V = GPUShaderStage.VERTEX;
	const computeLayout = device.createBindGroupLayout({
		label: 'inside-compute',
		entries: [
			{ binding: 0, visibility: C, buffer: { type: 'uniform' } },
			{ binding: 1, visibility: C, buffer: { type: 'uniform' } },
			{ binding: 2, visibility: C, buffer: { type: 'read-only-storage' } },
			{ binding: 3, visibility: C, buffer: { type: 'storage' } },
			{ binding: 4, visibility: C, buffer: { type: 'read-only-storage' } }
		]
	});
	const drawLayout = device.createBindGroupLayout({
		label: 'inside-draw',
		entries: [
			{ binding: 0, visibility: VF, buffer: { type: 'uniform' } },
			{ binding: 1, visibility: VF, buffer: { type: 'uniform' } },
			{ binding: 2, visibility: V, buffer: { type: 'read-only-storage' } },
			{ binding: 3, visibility: V, buffer: { type: 'read-only-storage' } },
			{ binding: 4, visibility: V, buffer: { type: 'read-only-storage' } }
		]
	});
	const backLayout = device.createBindGroupLayout({
		label: 'inside-back',
		entries: [
			{ binding: 0, visibility: VF, buffer: { type: 'uniform' } },
			{ binding: 1, visibility: VF, buffer: { type: 'uniform' } }
		]
	});
	const modules = {
		compute: device.createShaderModule({ label: 'inside-compute', code: INSIDE_COMPUTE_WGSL }),
		particles: device.createShaderModule({ label: 'inside-particles', code: INSIDE_PARTICLES_WGSL }),
		backdrop: device.createShaderModule({ label: 'inside-backdrop', code: INSIDE_BACKDROP_WGSL })
	};
	for (const [name, m] of Object.entries(modules)) {
		void m.getCompilationInfo().then((info) => {
			for (const msg of info.messages) {
				if (msg.type === 'error') console.error(`[inside:${name}] ${msg.lineNum}:${msg.linePos} ${msg.message}`);
			}
		});
	}
	const fmt = engine.sceneFormat;
	const simPipe = device.createComputePipeline({
		label: 'inside-sim',
		layout: device.createPipelineLayout({ bindGroupLayouts: [computeLayout] }),
		compute: { module: modules.compute, entryPoint: 'cs' }
	});
	const draw = (module: GPUShaderModule, layout: GPUBindGroupLayout, label: string) =>
		device.createRenderPipeline({
			label,
			layout: device.createPipelineLayout({ bindGroupLayouts: [layout] }),
			vertex: { module, entryPoint: 'vs' },
			fragment: { module, entryPoint: 'fs', targets: [{ format: fmt, blend: ADDITIVE }] },
			primitive: { topology: 'triangle-list' }
		});
	const partPipe = draw(modules.particles, drawLayout, 'inside-particles');
	const backPipe = draw(modules.backdrop, backLayout, 'inside-backdrop');
	const five = (layout: GPUBindGroupLayout) =>
		device.createBindGroup({
			layout,
			entries: [
				{ binding: 0, resource: { buffer: params } },
				{ binding: 1, resource: { buffer: uBuf } },
				{ binding: 2, resource: { buffer: moonBuf } },
				{ binding: 3, resource: { buffer: stateBuf } },
				{ binding: 4, resource: { buffer: handBuf } }
			]
		});
	const simBG = five(computeLayout);
	const partBG = five(drawLayout);
	const backBG = device.createBindGroup({
		layout: backLayout,
		entries: [
			{ binding: 0, resource: { buffer: params } },
			{ binding: 1, resource: { buffer: uBuf } }
		]
	});

	// ── State ───────────────────────────────────────────────────────────────
	const now = () => engine.simSeconds;
	let memId = '';
	let mem: InsideMemory | null = null;
	let form = formParams('', 'vestige');
	let norm = normaliser(form);
	let moonList: InsideMoon[] = [];
	let moonPos: { x: number; y: number; size: number }[] = [];
	let generation = 0;
	let arrivedAt = now();
	let arrival = 0;
	let arrivalPoint = { x: 0, y: 0 };
	let pendingArrival: { mode: 1; count: number } | { mode: 2; x: number; y: number } | null = null;
	let handCount = 0;
	let verify: VerifyState = 'idle';
	let verifyAt = 0;
	let cursor: { x: number; y: number } | null = null;
	let hover = -1;
	let yaw = 0;
	let pitch = 0;
	let last = -1;

	const aspectNow = () => (engine.params[6] || 1) / Math.max(1, engine.params[7] || 1);

	function geometry(A: number) {
		const r = insideRegion(A);
		const cx = ((r.left + r.right) / 2) * A;
		const cy = (r.top + r.bottom) / 2;
		const halfW = ((r.right - r.left) / 2) * A;
		const halfH = (r.top - r.bottom) / 2;
		const orbit = Math.min(halfW, halfH) * 0.84;
		const len = mem?.length ?? 400;
		const R = orbit * 0.4 * (0.9 + 0.2 * Math.min(1, Math.log10(len + 1) / 3.5));
		return { cx, cy, R, orbit, ring: orbit * 0.62 };
	}

	function placeMoons() {
		const g = geometry(aspectNow());
		const n = moonList.length;
		const scale = g.orbit / 0.66;
		moonPos = moonList.map((m, i) => {
			const a = Math.PI / 2 - ((i + 0.5) / Math.max(1, n)) * Math.PI * 2;
			const rr = g.orbit * (n > 8 && i % 2 ? 0.9 : 1);
			const size = (isCausalEdge(m.edge) ? 0.034 : 0.026) * scale;
			return { x: g.cx + Math.cos(a) * rr, y: g.cy + Math.sin(a) * rr, size };
		});
		const age = now() - arrivedAt;
		moonArr.fill(0);
		moonList.forEach((m, i) => {
			const p = moonPos[i];
			const tc = typeColor(m.type);
			const ec = edgeColor(m.edge);
			moonArr.set(
				[p.x, p.y, p.size, m.outgoing ? 1 : 0, tc[0], tc[1], tc[2], isCausalEdge(m.edge) ? 1 : 0, ec[0], ec[1], ec[2], Math.max(0.2, age) + 0.15 + i * 0.07],
				i * MOON_FLOATS
			);
		});
		device.queue.writeBuffer(moonBuf, 0, moonArr);
	}

	let lastAspect = -1;
	function writeUniforms() {
		const A = aspectNow();
		if (Math.abs(A - lastAspect) > 1e-3) {
			lastAspect = A;
			if (moonList.length) placeMoons();
		}
		const t = now();
		const dt = last < 0 ? 1 / 60 : Math.min(0.05, Math.max(0, t - last));
		last = t;
		const g = geometry(A);
		// The form leans toward the cursor, a little.
		const k = 1 - Math.exp(-dt * 3);
		const ty = cursor ? Math.max(-1, Math.min(1, (cursor.x - g.cx) / g.orbit)) * 0.5 : 0;
		const tp = cursor ? -Math.max(-1, Math.min(1, (cursor.y - g.cy) / g.orbit)) * 0.3 : 0;
		yaw += (ty - yaw) * k;
		pitch += (tp - pitch) * k;
		const tc = typeColor(mem?.type ?? '');
		const retention = Math.max(0, Math.min(1, mem?.retention ?? 0.8));
		const stability = mem?.stability ?? 10;
		const breath = Math.max(2.2, Math.min(7, 2.2 + Math.log2(1 + Math.max(0, stability)) * 0.6));
		const loose = Math.max(0, Math.min(0.6, (1 - retention) * 0.7));
		const proof = verify === 'running' ? 1 : verify === 'ok' ? 2 : verify === 'fail' ? 3 : 0;
		uArr.set([A, t - arrivedAt, dt, t], 0);
		uArr.set([g.cx, g.cy, g.R, g.orbit], 4);
		uArr.set([tc[0], tc[1], tc[2], retention], 8);
		uArr.set(form.a, 12);
		uArr.set(form.b, 16);
		uArr.set([form.twist, norm, breath, loose], 20);
		uArr.set([cursor?.x ?? 0, cursor?.y ?? 0, cursor ? 1 : 0, hover], 24);
		uArr.set([proof, proof ? t - verifyAt : 0, g.ring, 0], 28);
		uArr.set([moonList.length, generation, arrival, handCount], 32);
		uArr.set([arrivalPoint.x, arrivalPoint.y, yaw, pitch], 36);
		uArr.set([0, 0, 1, 0], 40);
		device.queue.writeBuffer(uBuf, 0, uArr);
	}

	function reshape() {
		form = formParams(mem?.type ?? '', memId || 'vestige');
		norm = normaliser(form);
	}

	return {
		highRefresh: true,
		arrive(id: string) {
			memId = id;
			mem = null;
			moonList = [];
			moonPos = [];
			verify = 'idle';
			reshape();
			generation++;
			arrivedAt = now();
			if (pendingArrival?.mode === 1) {
				arrival = 1;
				handCount = pendingArrival.count;
			} else if (pendingArrival?.mode === 2) {
				arrival = 2;
				arrivalPoint = { x: pendingArrival.x, y: pendingArrival.y };
			} else {
				arrival = 0;
			}
			pendingArrival = null;
			engine.requestRender();
		},
		setMemory(m: InsideMemory) {
			const typeChanged = m.type !== mem?.type;
			mem = m;
			if (typeChanged) reshape();
			if (moonList.length) placeMoons();
		},
		setMoons(list: InsideMoon[]) {
			const causalFirst = [...list].sort((a, b) => Number(isCausalEdge(b.edge)) - Number(isCausalEdge(a.edge)));
			moonList = causalFirst.slice(0, MAX_MOONS);
			placeMoons();
		},
		moons: () => moonList,
		setVerify(v: VerifyState) {
			if (v === verify) return;
			verify = v;
			verifyAt = now();
		},
		setCursor(ndc) {
			cursor = ndc ? { x: ndc.x * aspectNow(), y: ndc.y } : null;
		},
		setHover(h) {
			hover = h === 'form' ? 99 : h === null ? -1 : h;
		},
		pickAt(ndcX: number, ndcY: number): InsidePick | null {
			const A = aspectNow();
			const w = { x: ndcX * A, y: ndcY };
			let best = -1;
			let bestD = Infinity;
			moonPos.forEach((p, i) => {
				const d = Math.hypot(w.x - p.x, w.y - p.y);
				if (d < p.size * 2.6 && d < bestD) {
					best = i;
					bestD = d;
				}
			});
			if (best >= 0) return { kind: 'moon', index: best, id: moonList[best].id };
			const g = geometry(A);
			const d = Math.hypot(w.x - g.cx, w.y - g.cy);
			if (d < g.R * 1.1) return { kind: 'form' };
			if (Math.abs(d - g.ring) < 0.04) return { kind: 'ring' };
			return null;
		},
		moonNdc(i: number) {
			const p = moonPos[i];
			if (!p) return null;
			const A = aspectNow();
			return { x: p.x / A, y: p.y, r: p.size };
		},
		ringNdc() {
			const A = aspectNow();
			const g = geometry(A);
			return { x: g.cx / A, y: g.cy, r: g.ring };
		},
		formNdc() {
			const A = aspectNow();
			const g = geometry(A);
			return { x: g.cx / A, y: g.cy, r: g.R };
		},
		region: () => insideRegion(aspectNow()),
		beginHandoff(points: Float32Array<ArrayBuffer>, count: number) {
			const n = Math.min(count, MAX_HANDOFF);
			if (n <= 0) return;
			device.queue.writeBuffer(handBuf, 0, points, 0, n * HANDOFF_STRIDE);
			pendingArrival = { mode: 1, count: n };
		},
		arriveFrom(ndc) {
			pendingArrival = { mode: 2, x: ndc.x * aspectNow(), y: ndc.y };
		},
		compute(encoder: GPUCommandEncoder) {
			writeUniforms();
			const cp = encoder.beginComputePass({ label: 'inside-sim' });
			cp.setPipeline(simPipe);
			cp.setBindGroup(0, simBG);
			cp.dispatchWorkgroups(Math.ceil(TOTAL / 256));
			cp.end();
		},
		render(pass: GPURenderPassEncoder) {
			pass.setPipeline(backPipe);
			pass.setBindGroup(0, backBG);
			pass.draw(3);
			pass.setPipeline(partPipe);
			pass.setBindGroup(0, partBG);
			pass.draw(6, TOTAL);
		},
		dispose() {
			uBuf.destroy();
			moonBuf.destroy();
			stateBuf.destroy();
			handBuf.destroy();
		}
	};
}
