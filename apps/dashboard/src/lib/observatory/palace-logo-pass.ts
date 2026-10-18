/**
 * The VESTIGE logo at the top of the Palace, drawn in light.
 *
 * Every letter is a skeleton of strokes, and every stroke is a few fibres
 * twisting round each other, green into violet, like nerve tissue. Dendrites
 * branch off the strokes, synapses glow where strokes meet, impulses run
 * through the word from left to right, and sparks stream out behind it. The
 * word is born out of the brain below it as the Palace opens, and it fades
 * the moment a click flies into an organ, so no letter is ever carried into
 * the organ's page.
 */
import type { FramePass, ObservatoryEngine } from './engine';

type Pt = [number, number];
/** Letter skeletons in a unit box (y up), with their advance width. */
const GLYPHS: Record<string, { w: number; strokes: Pt[][] }> = {
	V: { w: 0.82, strokes: [[[0, 1], [0.41, 0], [0.82, 1]]] },
	E: {
		w: 0.68,
		strokes: [
			[[0.68, 1], [0, 1], [0, 0], [0.68, 0]],
			[[0, 0.5], [0.54, 0.5]]
		]
	},
	S: {
		w: 0.76,
		strokes: [
			[
				[0.74, 0.84], [0.6, 0.99], [0.38, 1.0], [0.16, 0.97], [0.03, 0.84], [0.04, 0.66], [0.18, 0.55],
				[0.4, 0.5], [0.6, 0.45], [0.74, 0.34], [0.76, 0.17], [0.62, 0.02], [0.38, 0.0], [0.16, 0.02], [0.02, 0.16]
			]
		]
	},
	T: {
		w: 0.8,
		strokes: [
			[[0, 1], [0.8, 1]],
			[[0.4, 1], [0.4, 0]]
		]
	},
	I: {
		w: 0.3,
		strokes: [
			[[0, 1], [0.3, 1]],
			[[0.15, 1], [0.15, 0]],
			[[0, 0], [0.3, 0]]
		]
	},
	G: {
		w: 0.8,
		strokes: [
			[
				[0.78, 0.82], [0.62, 0.98], [0.4, 1.0], [0.18, 0.97], [0.03, 0.8], [0.0, 0.5], [0.03, 0.2], [0.18, 0.03],
				[0.4, 0.0], [0.62, 0.03], [0.8, 0.2], [0.8, 0.46], [0.46, 0.46]
			]
		]
	}
};
const WORD = 'VESTIGE';
const GAP = 0.24;

interface Seg {
	a: Pt;
	b: Pt;
	/** Where it starts along the whole word, and its length. */
	at: number;
	len: number;
}

function fract(v: number): number {
	return v - Math.floor(v);
}
function hh(x: number): number {
	return fract(Math.sin(x * 12.9898 + 78.233) * 43758.5453);
}
function rnd(id: number, k: number): number {
	return hh(id * (1.618 + k * 0.731) + k * 9.17);
}

/** The word as segments in logo units (height 1), its width, and its corners. */
function wordSegments(): { segs: Seg[]; width: number; total: number; corners: Pt[] } {
	const segs: Seg[] = [];
	const corners: Pt[] = [];
	let x = 0;
	let total = 0;
	for (const ch of WORD) {
		const g = GLYPHS[ch];
		for (const stroke of g.strokes) {
			for (let i = 0; i < stroke.length; i++) {
				const p: Pt = [stroke[i][0] + x, stroke[i][1]];
				if (i === 0 || i === stroke.length - 1 || ch !== 'S' && ch !== 'G') corners.push(p);
				if (i === 0) continue;
				const a: Pt = [stroke[i - 1][0] + x, stroke[i - 1][1]];
				const len = Math.hypot(p[0] - a[0], p[1] - a[1]);
				segs.push({ a, b: p, at: total, len });
				total += len;
			}
		}
		x += g.w + GAP;
	}
	return { segs, width: x - GAP, total, corners };
}

const WGSL = /* wgsl */ `
struct U {
	view: vec4f,  // width px, height px, time s, fade
	place: vec4f, // centre x, centre y (NDC), NDC x per logo unit, aspect
	misc: vec4f,  // birth 0..1, word width (units), reduced motion, 0
};
@group(0) @binding(0) var<uniform> u: U;

const QUAD = array<vec2f, 6>(
	vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
	vec2f(-1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, 1.0)
);
const TAU = 6.2831853;

struct VO {
	@builtin(position) pos: vec4f,
	@location(0) local: vec2f,
	@location(1) col: vec3f,
};

@vertex
fn vs(@location(0) a: vec4f, @location(1) b: vec4f, @builtin(vertex_index) vi: u32) -> VO {
	// a: x, y (logo units), flow along the word (0..1), kind (0 fibre, 1 dendrite, 2 synapse, 3 spark)
	// b: direction xy (sparks, fibres' twist), seed, size (px)
	let t = u.view.z;
	let still = u.misc.z > 0.5;
	let kind = u32(a.w + 0.5);
	let seed = b.z;
	var p = a.xy;
	var glow = 1.0;
	if (kind == 0u && !still) {
		// The fibres twist round each other along the stroke, and breathe.
		p += b.xy * 0.018 * sin(t * 1.3 + seed * TAU);
	}
	if (kind == 3u) {
		// A spark flies out from the word along its own ray, and dims as it goes.
		let ph = select(fract(seed * 7.31 + t * (0.05 + seed * 0.07)), seed, still);
		p = a.xy + b.xy * ph * 2.4;
		glow = (1.0 - ph) * (1.0 - ph);
	}
	let unit = u.place.z;
	var ndc = vec2f(u.place.x + (p.x - u.misc.y * 0.5) * unit, u.place.y + (p.y - 0.5) * unit * u.place.w);
	// Born out of the brain below, letter light arriving a little out of step.
	let birth = clamp((u.misc.x - seed * 0.35) / 0.65, 0.0, 1.0);
	let e = birth * birth * (3.0 - 2.0 * birth);
	ndc = mix(vec2f(0.0, -0.06), ndc, e);

	let green = vec3f(0.3, 1.0, 0.58);
	let violet = vec3f(0.7, 0.4, 1.0);
	let wave = 0.5 + 0.5 * sin(p.x * 1.6 - t * 0.7 + seed * 2.0);
	var col = mix(green, violet, wave);
	// Impulses run through the word from left to right.
	let impulse = select(pow(0.5 + 0.5 * cos(TAU * (a.z * 3.0 - t * 0.2)), 18.0), 0.0, still);
	var energy = 0.75 + impulse * 2.2;
	if (kind == 1u) {
		col = mix(green, vec3f(0.3, 0.92, 0.95), 0.45);
		energy = 0.55 + impulse * 1.6;
	} else if (kind == 2u) {
		col = mix(violet, vec3f(1.0), 0.4);
		energy = 1.6 + 1.4 * select(pow(0.5 + 0.5 * sin(t * 2.6 + seed * 31.0), 6.0), 0.5, still);
	} else if (kind == 3u) {
		col = mix(green, violet, step(0.5, fract(seed * 13.0)));
		energy = 1.1 * glow;
	}
	let fade = u.view.w * e;
	let q = QUAD[vi];
	var o: VO;
	o.pos = vec4f(ndc + q * b.w * vec2f(2.0 / max(u.view.x, 1.0), 2.0 / max(u.view.y, 1.0)), 0.0, 1.0);
	o.local = q;
	o.col = col * energy * fade;
	if (fade <= 0.002) { o.pos = vec4f(3.0, 3.0, 0.0, 1.0); }
	return o;
}

@fragment
fn fs(i: VO) -> @location(0) vec4f {
	let r = clamp(length(i.local), 0.0, 1.0);
	if (r >= 1.0) { discard; }
	let profile = pow(1.0 - r, 2.4) * 1.3 + (1.0 - r) * 0.35;
	return vec4f(i.col * profile, 1.0);
}
`;

export class PalaceLogoPass implements FramePass {
	readonly highRefresh = true;
	private engine: ObservatoryEngine;
	private pipeline: GPURenderPipeline | null = null;
	private bind: GPUBindGroup | null = null;
	private ubuf: GPUBuffer | null = null;
	private vbuf: GPUBuffer | null = null;
	private count = 0;
	private width = 1;
	private uni = new Float32Array(12);
	private place = { x: 0, y: 0.76, w: 1.0 };
	private bornMs: number;
	private leftMs: number | null = null;
	private reduced = false;

	constructor(engine: ObservatoryEngine, reduced = false) {
		this.engine = engine;
		this.reduced = reduced;
		this.bornMs = engine.wallNowMs;
		this.build();
	}

	/** Where the word stands: its centre (NDC) and its width (NDC x). */
	setPlace(x: number, y: number, w: number): void {
		this.place = { x, y, w };
		this.engine.requestRender();
	}

	/** A click is flying into an organ: the word fades at once. */
	leave(): void {
		if (this.leftMs === null) this.leftMs = this.engine.wallNowMs;
	}

	private build(): void {
		const device = this.engine.gpuDevice;
		if (!device) return;
		const { segs, width, total, corners } = wordSegments();
		this.width = width;
		const small = typeof window !== 'undefined' && window.innerWidth < 760;
		const n = this.reduced ? 9000 : small ? 14_000 : 26_000;
		const data = new Float32Array(n * 8);
		const at = (s: number): { p: Pt; dir: Pt; seg: Seg } => {
			let lo = 0;
			let hi = segs.length - 1;
			while (lo < hi) {
				const mid = (lo + hi + 1) >> 1;
				if (segs[mid].at <= s) lo = mid;
				else hi = mid - 1;
			}
			const seg = segs[lo];
			const f = Math.min(1, Math.max(0, (s - seg.at) / Math.max(seg.len, 1e-6)));
			const dx = (seg.b[0] - seg.a[0]) / Math.max(seg.len, 1e-6);
			const dy = (seg.b[1] - seg.a[1]) / Math.max(seg.len, 1e-6);
			return { p: [seg.a[0] + (seg.b[0] - seg.a[0]) * f, seg.a[1] + (seg.b[1] - seg.a[1]) * f], dir: [dx, dy], seg };
		};
		for (let i = 0; i < n; i++) {
			const id = i + 1;
			const pick = rnd(id, 0);
			const o = i * 8;
			let x = 0;
			let y = 0;
			let flow = 0;
			let kind = 0;
			let bx = 0;
			let by = 0;
			let size = 1.6;
			if (pick < 0.5) {
				// A fibre: one of three strands twisting along the stroke.
				const s = rnd(id, 1) * total;
				const { p, dir } = at(s);
				const strand = Math.floor(rnd(id, 2) * 3);
				const nx = -dir[1];
				const ny = dir[0];
				const twist = 0.03 * Math.sin(s * 34 + strand * 2.1) + (rnd(id, 3) - 0.5) * 0.012;
				x = p[0] + nx * twist;
				y = p[1] + ny * twist;
				flow = p[0] / width;
				bx = nx;
				by = ny;
				size = 1.3 + rnd(id, 4) * 1.1;
			} else if (pick < 0.68) {
				// A dendrite: a thin branch reaching out from the stroke, forking once.
				const base = Math.floor(rnd(id, 1) * 140);
				const s0 = hh(base * 3.17 + 1) * total;
				const { p } = at(s0);
				const ang = hh(base * 7.7 + 2) * Math.PI * 2;
				const len = 0.1 + hh(base * 1.9 + 3) * 0.26;
				const t = rnd(id, 2);
				const fork = rnd(id, 3) < 0.4 && t > 0.5;
				const a2 = ang + (hh(base * 5.3 + 4) - 0.5) * 1.6;
				const ex = fork ? p[0] + Math.cos(ang) * len * 0.5 + Math.cos(a2) * len * (t - 0.5) : p[0] + Math.cos(ang) * len * t;
				const ey = fork ? p[1] + Math.sin(ang) * len * 0.5 + Math.sin(a2) * len * (t - 0.5) : p[1] + Math.sin(ang) * len * t;
				x = ex + (rnd(id, 4) - 0.5) * 0.006;
				y = ey + (rnd(id, 5) - 0.5) * 0.006;
				flow = p[0] / width;
				kind = 1;
				size = 1.1 + (1 - t) * 0.8;
			} else if (pick < 0.75) {
				// A synapse: a bright knot where strokes meet, or somewhere along one.
				const onCorner = rnd(id, 1) < 0.6;
				const c = onCorner ? corners[Math.floor(rnd(id, 2) * corners.length)] : at(hh(Math.floor(rnd(id, 2) * 40) * 2.7) * total).p;
				const r = 0.035 * Math.cbrt(rnd(id, 3));
				const a = rnd(id, 4) * Math.PI * 2;
				x = c[0] + Math.cos(a) * r;
				y = c[1] + Math.sin(a) * r;
				flow = c[0] / width;
				kind = 2;
				size = 1.8 + rnd(id, 5) * 1.6;
			} else {
				// A spark: streams out from behind the word along its own ray.
				const ox = rnd(id, 1) * width;
				const oy = 0.2 + rnd(id, 2) * 0.6;
				const cx = width / 2;
				let dx = ox - cx + (rnd(id, 3) - 0.5) * 0.8;
				let dy = (oy - 0.5) * 2.2 + (rnd(id, 4) - 0.5) * 0.9;
				const m = Math.hypot(dx, dy) || 1;
				dx /= m;
				dy /= m;
				x = ox;
				y = oy;
				bx = dx;
				by = dy;
				flow = ox / width;
				kind = 3;
				size = 1.0 + rnd(id, 5) * 1.3;
			}
			data.set([x, y, flow, kind, bx, by, rnd(id, 9), size], o);
		}
		this.count = n;
		this.vbuf = device.createBuffer({ label: 'palace-logo-points', size: data.byteLength, usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST });
		device.queue.writeBuffer(this.vbuf, 0, data);
		this.ubuf = device.createBuffer({ label: 'palace-logo-uniforms', size: this.uni.byteLength, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
		const module = device.createShaderModule({ label: 'palace-logo', code: WGSL });
		void module.getCompilationInfo().then((info) => {
			for (const m of info.messages) if (m.type === 'error') console.error(`[palace-logo] ${m.lineNum}:${m.linePos} ${m.message}`);
		});
		this.pipeline = device.createRenderPipeline({
			label: 'palace-logo',
			layout: 'auto',
			vertex: {
				module,
				entryPoint: 'vs',
				buffers: [
					{
						arrayStride: 32,
						stepMode: 'instance',
						attributes: [
							{ shaderLocation: 0, offset: 0, format: 'float32x4' },
							{ shaderLocation: 1, offset: 16, format: 'float32x4' }
						]
					}
				]
			},
			fragment: {
				module,
				entryPoint: 'fs',
				targets: [
					{
						format: this.engine.sceneFormat,
						blend: {
							color: { srcFactor: 'one', dstFactor: 'one', operation: 'add' },
							alpha: { srcFactor: 'one', dstFactor: 'one', operation: 'add' }
						}
					}
				]
			},
			primitive: { topology: 'triangle-list' }
		});
		this.bind = device.createBindGroup({
			layout: this.pipeline.getBindGroupLayout(0),
			entries: [{ binding: 0, resource: { buffer: this.ubuf } }]
		});
	}

	compute(): void {
		const device = this.engine.gpuDevice;
		if (!device || !this.ubuf) return;
		const now = this.engine.wallNowMs;
		const w = this.engine.params[6] || 1;
		const h = this.engine.params[7] || 1;
		const birth = this.reduced ? 1 : Math.min(1, (now - this.bornMs) / 1900);
		const fade = this.leftMs === null ? 1 : Math.max(0, 1 - (now - this.leftMs) / 260);
		const aspect = w / h;
		this.uni.set([w, h, this.engine.simSeconds, fade, this.place.x, this.place.y, this.place.w / this.width, aspect, birth, this.width, this.reduced ? 1 : 0, 0]);
		device.queue.writeBuffer(this.ubuf, 0, this.uni);
	}

	render(pass: GPURenderPassEncoder): void {
		if (!this.pipeline || !this.bind || !this.vbuf || this.count === 0) return;
		pass.setPipeline(this.pipeline);
		pass.setBindGroup(0, this.bind);
		pass.setVertexBuffer(0, this.vbuf);
		pass.draw(6, this.count);
	}

	dispose(): void {
		this.vbuf?.destroy();
		this.ubuf?.destroy();
		this.vbuf = null;
		this.ubuf = null;
	}
}
