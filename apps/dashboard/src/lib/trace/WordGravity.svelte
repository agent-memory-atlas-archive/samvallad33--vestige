<script lang="ts">
	/**
	 * The second explosion writes the page.
	 *
	 * After a Palace burst the destination page is traced into points: every
	 * letter of every word on screen, drawn with its own font where it sits.
	 * The burst's light (the captured frame the veil holds) goes off once
	 * more, then every orb is pulled onto a letter, in reading order, running
	 * through every colour as it flies and taking the word's colour as it
	 * lands. Each word is hidden until its orbs arrive, then glows in.
	 *
	 * One overlay for every page: it reads what the page shows, so the words
	 * it forms are the page's own. Its own small WebGPU device, made when a
	 * burst starts, so it is ready by the time the page is.
	 */
	import { onMount } from 'svelte';
	import { veil, type VeilState } from './handoff';

	let canvas = $state<HTMLCanvasElement | null>(null);
	let running = $state(false);

	const MAX_ORBS = 26_000;
	const FLOATS = 12;
	/** When the first words begin to form, and how long the whole page takes. */
	const FIRST_AT = 0.45;
	const SPREAD = 0.95;
	const FLIGHT = 0.62;

	interface Piece {
		x: number;
		y: number;
		w: number;
		h: number;
		el: HTMLElement;
		color: [number, number, number];
		at: number;
	}

	type Gpu = { device: GPUDevice; format: GPUTextureFormat };
	let gpu: Gpu | null = null;
	let gpuWait: Promise<Gpu | null> | null = null;
	function ensureGpu(): Promise<Gpu | null> {
		if (gpu) return Promise.resolve(gpu);
		if (gpuWait) return gpuWait;
		gpuWait = (async (): Promise<Gpu | null> => {
			if (typeof navigator === 'undefined' || !navigator.gpu) return null;
			const adapter = await navigator.gpu.requestAdapter();
			if (!adapter) return null;
			const device = await adapter.requestDevice();
			gpu = { device, format: navigator.gpu.getPreferredCanvasFormat() };
			return gpu;
		})().catch(() => null);
		return gpuWait;
	}

	const SKIP = '.os-dock, .os-mobilebar, .veil, script, style, svg, canvas, noscript, [data-wg-skip], .telemetry, .tick';
	function parseColor(c: string): [number, number, number] {
		const m = c.match(/[\d.]+/g);
		return m && m.length >= 3 ? [+m[0] / 255, +m[1] / 255, +m[2] / 255] : [0.8, 0.95, 0.95];
	}

	/** Every word on screen as a piece: where it is, its element, its colour. */
	function tracePage(g: CanvasRenderingContext2D, W: number, H: number): Piece[] {
		const pieces: Piece[] = [];
		const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
		const range = document.createRange();
		const styleOf = new Map<Element, CSSStyleDeclaration>();
		let node: Node | null;
		while ((node = walker.nextNode())) {
			const text = node.textContent ?? '';
			if (!text.trim()) continue;
			const parent = node.parentElement;
			if (!parent || parent.closest(SKIP)) continue;
			let cs = styleOf.get(parent);
			if (!cs) {
				cs = getComputedStyle(parent);
				styleOf.set(parent, cs);
			}
			if (cs.visibility === 'hidden' || cs.display === 'none' || +cs.opacity === 0) continue;
			const upper = cs.textTransform === 'uppercase';
			g.font = `${cs.fontStyle} ${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
			try {
				(g as unknown as { letterSpacing: string }).letterSpacing = cs.letterSpacing === 'normal' ? '0px' : cs.letterSpacing;
			} catch {
				/* older canvas: no letter spacing */
			}
			const color = parseColor(cs.color);
			const re = /\S+/g;
			let m: RegExpExecArray | null;
			while ((m = re.exec(text))) {
				range.setStart(node, m.index);
				range.setEnd(node, m.index + m[0].length);
				const r = range.getBoundingClientRect();
				if (r.width < 1 || r.bottom < 0 || r.top > H || r.right < 0 || r.left > W) continue;
				const word = upper ? m[0].toUpperCase() : m[0];
				const mt = g.measureText(word);
				const asc = mt.fontBoundingBoxAscent || parseFloat(cs.fontSize) * 0.8;
				const desc = mt.fontBoundingBoxDescent || parseFloat(cs.fontSize) * 0.2;
				g.fillText(word, r.left, r.top + (r.height - (asc + desc)) / 2 + asc);
				pieces.push({ x: r.left, y: r.top, w: r.width, h: r.height, el: parent, color, at: 0 });
			}
		}
		return pieces;
	}

	/** Lit points of the burst the veil holds, in CSS pixels, with their colours. */
	function burstPoints(frame: HTMLCanvasElement, W: number, H: number, n: number): Float32Array {
		const out = new Float32Array(n * 5);
		const g = frame.getContext('2d', { willReadFrequently: true });
		const lit: number[] = [];
		let data: Uint8ClampedArray | null = null;
		if (g) {
			data = g.getImageData(0, 0, frame.width, frame.height).data;
			for (let i = 0; i < data.length; i += 4 * 3) if (data[i] + data[i + 1] + data[i + 2] > 120) lit.push(i);
		}
		// The veil has carried the frame on, about a quarter larger, about the middle.
		const grow = 1.25;
		for (let k = 0; k < n; k++) {
			if (lit.length && data) {
				const i = lit[(k * 7919) % lit.length];
				const px = (i / 4) % frame.width;
				const py = Math.floor(i / 4 / frame.width);
				const x = ((px / frame.width - 0.5) * grow + 0.5) * W;
				const y = ((py / frame.height - 0.5) * grow + 0.5) * H;
				out.set([x, y, data[i] / 255, data[i + 1] / 255, data[i + 2] / 255], k * 5);
			} else {
				const a = (k / n) * Math.PI * 2 * 7.13;
				const r = Math.sqrt((k * 0.618) % 1) * Math.min(W, H) * 0.4;
				out.set([W / 2 + Math.cos(a) * r, H / 2 + Math.sin(a) * r, 1, 1, 1], k * 5);
			}
		}
		return out;
	}

	const WGSL = /* wgsl */ `
struct U { a: vec4f }; // time s, width px, height px, dpr
@group(0) @binding(0) var<uniform> u: U;
const QUAD = array<vec2f, 6>(vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, 1.0));
fn rainbow(x: f32) -> vec3f { return 0.5 + 0.5 * cos(6.2831853 * (x + vec3f(0.0, 0.33, 0.67))); }
struct VO { @builtin(position) pos: vec4f, @location(0) q: vec2f, @location(1) c: vec3f };

// Where an orb is at time t: thrown out by the second explosion, then pulled
// onto its letter, arriving at t1.
fn place(src: vec2f, dst: vec2f, dir: vec2f, seed: f32, t0: f32, t1: f32, t: f32) -> vec2f {
	let throwR = (0.18 + 0.5 * seed) * min(u.a.y, u.a.z);
	let thrown = src + dir * throwR * (1.0 - pow(1.0 - clamp(t / 0.55, 0.0, 1.0), 3.0));
	if (t <= t0) { return thrown; }
	let p0 = src + dir * throwR * (1.0 - pow(1.0 - clamp(t0 / 0.55, 0.0, 1.0), 3.0));
	let k = clamp((t - t0) / max(t1 - t0, 0.01), 0.0, 1.0);
	// Gravity: slow to let go, faster and faster to the letter.
	let e = k * k * (3.0 - 2.0 * k);
	let side = vec2f(-dir.y, dir.x) * (seed - 0.5) * throwR * 0.9;
	let ctrl = p0 + dir * throwR * 0.35 + side;
	return mix(mix(p0, ctrl, e), mix(ctrl, dst, e), e);
}

@vertex
fn vs(@location(0) a: vec4f, @location(1) b: vec4f, @location(2) c: vec4f, @builtin(vertex_index) vi: u32) -> VO {
	// a: src xy, dst xy (px)   b: t0, t1, seed, 0   c: word rgb, 0
	let t = u.a.x;
	let W = u.a.y;
	let H = u.a.z;
	let dpr = u.a.w;
	let seed = b.z;
	let mid = vec2f(W, H) * 0.5;
	var dir = a.xy - mid;
	let dl = length(dir);
	dir = select(vec2f(cos(seed * 40.0), sin(seed * 40.0)), dir / max(dl, 1e-3), dl > 4.0);
	let p = place(a.xy, a.zw, dir, seed, b.x, b.y, t);
	let pp = place(a.xy, a.zw, dir, seed, b.x, b.y, max(t - 0.016, 0.0));
	let v = p - pp;
	let sp = length(v);
	let d = select(vec2f(1.0, 0.0), v / max(sp, 1e-4), sp > 0.05);
	let k = clamp((t - b.x) / max(b.y - b.x, 0.01), 0.0, 1.0);
	// Every colour while it flies; the word's own colour, flashing, as it lands.
	var col = rainbow(seed * 1.7 + t * 0.9 + length(p - mid) / max(W, 1.0)) * 1.7;
	col = mix(col, c.rgb * 1.9, smoothstep(0.55, 1.0, k));
	col += vec3f(1.0) * exp(-pow((t - b.y) / 0.07, 2.0)) * 1.4;
	col += vec3f(1.0, 0.95, 0.85) * exp(-t * 6.0) * 1.2;
	// After landing the orb dims away and leaves the glowing word behind.
	let fade = 1.0 - smoothstep(b.y + 0.18, b.y + 0.75, t);
	let size = (select(1.5, 2.3, k < 1.0)) * dpr;
	let stretch = 1.0 + min(sp * dpr * 0.6, 10.0);
	let q = QUAD[vi];
	let off = d * q.x * size * stretch + vec2f(-d.y, d.x) * q.y * size;
	let px = (p * dpr + off);
	var o: VO;
	o.pos = vec4f(px.x / (W * dpr) * 2.0 - 1.0, 1.0 - px.y / (H * dpr) * 2.0, 0.0, 1.0);
	o.q = q;
	o.c = col * fade / sqrt(stretch);
	if (fade <= 0.001) { o.pos = vec4f(3.0, 3.0, 0.0, 1.0); }
	return o;
}
@fragment
fn fs(i: VO) -> @location(0) vec4f {
	let r = length(i.q);
	if (r >= 1.0) { discard; }
	let pr = pow(1.0 - r, 2.2) * 1.3 + (1.0 - r) * 0.3;
	let c = i.c * pr;
	return vec4f(c, clamp(max(c.r, max(c.g, c.b)), 0.0, 1.0));
}
`;

	let raf = 0;
	let timers: ReturnType<typeof setTimeout>[] = [];
	let shownFor = 0;
	let hidden: HTMLElement[] = [];

	function stop() {
		if (raf) cancelAnimationFrame(raf);
		raf = 0;
		for (const t of timers) clearTimeout(t);
		timers = [];
		for (const el of hidden) el.classList.remove('wg-hide', 'wg-in');
		hidden = [];
		if (typeof document !== 'undefined') document.documentElement.classList.remove('wg-pending');
		running = false;
	}

	/** Wait until the page's words stop changing (its data has landed), at most `max` ms. */
	function settled(max: number, quiet: number): Promise<void> {
		return new Promise((resolve) => {
			let last = performance.now();
			const mo = new MutationObserver(() => (last = performance.now()));
			mo.observe(document.body, { childList: true, subtree: true, characterData: true });
			const start = performance.now();
			const tick = () => {
				const now = performance.now();
				if (now - last >= quiet || now - start >= max) {
					mo.disconnect();
					resolve();
				} else requestAnimationFrame(tick);
			};
			requestAnimationFrame(tick);
		});
	}

	async function play(v: VeilState) {
		// The page's words stay dark from the moment it appears: they are the
		// second explosion's to write.
		document.documentElement.classList.add('wg-pending');
		const [g0] = await Promise.all([ensureGpu(), settled(520, 160)]);
		if (!g0 || !canvas || shownFor !== v.startedAt) {
			document.documentElement.classList.remove('wg-pending');
			return;
		}
		const W = innerWidth;
		const H = innerHeight;
		const dpr = Math.min(2, devicePixelRatio || 1);
		// Trace the page's words into points.
		const tc = document.createElement('canvas');
		tc.width = W;
		tc.height = H;
		const g = tc.getContext('2d', { willReadFrequently: true });
		if (!g) return;
		g.fillStyle = '#fff';
		// Trace with the words showing (they are drawn to a canvas, never seen).
		document.documentElement.classList.remove('wg-pending');
		const pieces = tracePage(g, W, H);
		if (!pieces.length) return;
		// Reading order: each word's orbs land a little after the one before.
		pieces.forEach((p, i) => (p.at = FIRST_AT + (i / Math.max(1, pieces.length - 1)) * SPREAD));
		const img = g.getImageData(0, 0, W, H).data;
		const lit: number[] = [];
		for (let y = 0; y < H; y += 1) for (let x = 0; x < W; x += 1) if (img[(y * W + x) * 4] > 110) lit.push(y * W + x);
		if (!lit.length) return;
		// Hide every traced word straight away, before the next paint.
		const firstAt = new Map<HTMLElement, number>();
		for (const p of pieces) if (!firstAt.has(p.el) || firstAt.get(p.el)! > p.at) firstAt.set(p.el, p.at);
		for (const el of firstAt.keys()) {
			el.classList.add('wg-hide');
			hidden.push(el);
		}
		const n = Math.min(MAX_ORBS, lit.length);
		const stride = lit.length / n;
		// Which word each lit pixel belongs to: the pieces' boxes, looked up by row bands.
		const band = 24;
		const rows = new Map<number, number[]>();
		pieces.forEach((p, i) => {
			for (let b = Math.floor(p.y / band); b <= Math.floor((p.y + p.h) / band); b++) {
				const list = rows.get(b) ?? [];
				list.push(i);
				rows.set(b, list);
			}
		});
		const pieceAt = (x: number, y: number) => {
			for (const i of rows.get(Math.floor(y / band)) ?? []) {
				const p = pieces[i];
				if (x >= p.x - 2 && x <= p.x + p.w + 2 && y >= p.y - 2 && y <= p.y + p.h + 2) return i;
			}
			return -1;
		};
		const src = burstPoints(v.frame, W, H, n);
		const data = new Float32Array(n * FLOATS);
		for (let k = 0; k < n; k++) {
			const idx = lit[Math.floor(k * stride)];
			const x = idx % W;
			const y = Math.floor(idx / W);
			const pi = pieceAt(x, y);
			const piece = pi >= 0 ? pieces[pi] : null;
			const seed = ((k * 0.61803398875) % 1 + (x * 0.0137) % 1) % 1;
			const t1 = (piece?.at ?? FIRST_AT + SPREAD) + FLIGHT * (0.85 + 0.3 * seed);
			const t0 = t1 - FLIGHT;
			const c = piece?.color ?? [0.85, 0.95, 0.95];
			data.set([src[k * 5], src[k * 5 + 1], x, y, t0, t1, seed, 0, c[0], c[1], c[2], 0], k * FLOATS);
		}
		// Each word glows in as its orbs land.
		for (const [el, at] of firstAt) {
			timers.push(
				setTimeout(() => {
					el.classList.remove('wg-hide');
					el.classList.add('wg-in');
					timers.push(setTimeout(() => el.classList.remove('wg-in'), 1300));
				}, (at + FLIGHT * 0.9) * 1000)
			);
		}

		const { device, format } = g0;
		canvas.width = Math.round(W * dpr);
		canvas.height = Math.round(H * dpr);
		const ctx = canvas.getContext('webgpu');
		if (!ctx) return;
		ctx.configure({ device, format, alphaMode: 'premultiplied' });
		const vbuf = device.createBuffer({ size: data.byteLength, usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST });
		device.queue.writeBuffer(vbuf, 0, data);
		const ubuf = device.createBuffer({ size: 16, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
		const module = device.createShaderModule({ code: WGSL });
		const pipe = device.createRenderPipeline({
			layout: 'auto',
			vertex: {
				module,
				entryPoint: 'vs',
				buffers: [
					{
						arrayStride: FLOATS * 4,
						stepMode: 'instance',
						attributes: [
							{ shaderLocation: 0, offset: 0, format: 'float32x4' },
							{ shaderLocation: 1, offset: 16, format: 'float32x4' },
							{ shaderLocation: 2, offset: 32, format: 'float32x4' }
						]
					}
				]
			},
			fragment: {
				module,
				entryPoint: 'fs',
				targets: [
					{
						format,
						blend: {
							color: { srcFactor: 'one', dstFactor: 'one', operation: 'add' },
							alpha: { srcFactor: 'one', dstFactor: 'one', operation: 'add' }
						}
					}
				]
			},
			primitive: { topology: 'triangle-list' }
		});
		const bind = device.createBindGroup({ layout: pipe.getBindGroupLayout(0), entries: [{ binding: 0, resource: { buffer: ubuf } }] });
		const end = FIRST_AT + SPREAD + FLIGHT * 1.15 + 0.8;
		const t0 = performance.now();
		running = true;
		const frame = (now: number) => {
			const t = (now - t0) / 1000;
			if (t > end || shownFor !== v.startedAt) {
				vbuf.destroy();
				ubuf.destroy();
				ctx.unconfigure();
				running = false;
				return;
			}
			device.queue.writeBuffer(ubuf, 0, new Float32Array([t, W, H, dpr]));
			const enc = device.createCommandEncoder();
			const pass = enc.beginRenderPass({
				colorAttachments: [{ view: ctx.getCurrentTexture().createView(), loadOp: 'clear', storeOp: 'store', clearValue: { r: 0, g: 0, b: 0, a: 0 } }]
			});
			pass.setPipeline(pipe);
			pass.setBindGroup(0, bind);
			pass.setVertexBuffer(0, vbuf);
			pass.draw(6, n);
			pass.end();
			device.queue.submit([enc.finish()]);
			raf = requestAnimationFrame(frame);
		};
		raf = requestAnimationFrame(frame);
	}

	const reduced = () => typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches;
	onMount(() => {
		const unsub = veil.subscribe((v) => {
			if (!v?.burst || reduced()) return;
			// The device is made while the Palace flies, so it is ready with the page.
			void ensureGpu();
			if (v.ready && shownFor !== v.startedAt) {
				stop();
				shownFor = v.startedAt;
				void play(v);
			}
		});
		return () => {
			unsub();
			stop();
		};
	});
</script>

<canvas bind:this={canvas} class="wg" class:on={running} aria-hidden="true"></canvas>

<style>
	.wg {
		position: fixed;
		inset: 0;
		width: 100vw;
		height: 100vh;
		z-index: 139;
		pointer-events: none;
		display: none;
	}
	.wg.on {
		display: block;
	}
	/* A word waits for its orbs, then glows in out of their light. */
	:global(.wg-hide) {
		opacity: 0 !important;
	}
	/* While the page is being traced, its words wait in the dark. */
	:global(html.wg-pending) :global(:is(.tr-ask, .tr-lane, .tr-beats, .tr-row, .lane-name, .row-head, .obs-col, main)) {
		opacity: 0 !important;
	}
	:global(.wg-in) {
		animation: wg-in 1.2s cubic-bezier(0.2, 0.7, 0.2, 1) both;
	}
	@keyframes -global-wg-in {
		0% {
			opacity: 0;
			filter: brightness(3.2) blur(2px);
			text-shadow:
				0 0 10px currentColor,
				0 0 26px currentColor;
		}
		30% {
			opacity: 1;
		}
		100% {
			opacity: 1;
			filter: none;
		}
	}
</style>
