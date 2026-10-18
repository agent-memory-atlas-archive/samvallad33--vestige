/**
 * The handoff — one field across two routes.
 *
 * When an organ is clicked in the Palace, the frame on screen is captured
 * (inside the render frame, while the WebGPU canvas still holds its pixels),
 * and every lit pixel becomes a point with its own colour. The camera then
 * dives into the clicked organ (HandoffVeil, which survives the navigation),
 * and the destination's field starts every particle at those same points,
 * zoomed exactly as the dive ends: the particles you were looking at are the
 * ones that reorganize into the new organ. Nothing about the destination is
 * faked; only its starting positions and colours come from the Palace.
 */
import { writable } from 'svelte/store';

/** Floats per handoff point: world x, world y, brightness, 0, r, g, b, 0. */
export const HANDOFF_STRIDE = 8;
export const MAX_HANDOFF = 56_000;

export interface Handoff {
	points: Float32Array<ArrayBuffer>;
	count: number;
	createdAt: number;
	href: string;
}

export interface VeilState {
	/** The captured frame. */
	frame: HTMLCanvasElement;
	/** Where the dive aims, in client pixels. */
	focusX: number;
	focusY: number;
	/** How far the dive zooms. */
	scale: number;
	/** Where the focus lands, in client pixels (the centre of the screen when absent). */
	toX?: number;
	toY?: number;
	color: string;
	startedAt: number;
	/** The destination's field is on screen with the handed-off particles. */
	ready: boolean;
	/**
	 * The least time (ms) the captured frame stays before it may lift. A dive
	 * (the default) holds until its zoom lands; a Palace burst lifts as soon
	 * as the page is up, so its particles carry the burst on, moving.
	 */
	hold?: number;
	/** A Palace burst: the frame keeps flying out (spinning, every colour) instead of a straight dive. */
	burst?: boolean;
}

export const veil = writable<VeilState | null>(null);

let pending: Handoff | null = null;

/**
 * Copy the next presented frame of a WebGPU canvas. Runs as a frame callback
 * queued after the engine's own, so the canvas still holds the frame it just
 * rendered (a WebGPU canvas drops its pixels once the frame is presented).
 */
export function captureFrame(
	canvas: HTMLCanvasElement,
	scale = 0.5,
	/** The engine drawing the canvas: the copy then happens right after a frame it renders. */
	engine?: { afterNextRender(cb: () => void): void } | null
): Promise<HTMLCanvasElement> {
	return new Promise((resolve) => {
		const copy = () => {
			const out = document.createElement('canvas');
			out.width = Math.max(1, Math.round(canvas.width * scale));
			out.height = Math.max(1, Math.round(canvas.height * scale));
			out.getContext('2d', { willReadFrequently: true })?.drawImage(canvas, 0, 0, out.width, out.height);
			resolve(out);
		};
		if (engine) engine.afterNextRender(copy);
		else requestAnimationFrame(copy);
	});
}

/**
 * Every lit pixel of the frame as a point, already moved by the dive: the
 * focus point goes to the centre of the screen and everything scales by
 * `zoom` around it. Points the dive pushes off screen are dropped; the rest
 * are sampled down to MAX_HANDOFF and returned in the destination's world
 * space (x times aspect, y as is).
 */
export function samplePoints(
	frame: HTMLCanvasElement,
	focus: { x: number; y: number },
	zoom: number,
	aspect: number,
	/** Where the focus lands on the destination, in NDC (the centre when absent). */
	to: { x: number; y: number } = { x: 0, y: 0 }
): { points: Float32Array<ArrayBuffer>; count: number } {
	const g = frame.getContext('2d', { willReadFrequently: true });
	if (!g) return { points: new Float32Array(0), count: 0 };
	const { width: w, height: h } = frame;
	const data = g.getImageData(0, 0, w, h).data;
	const candidates: number[] = [];
	for (let py = 0; py < h; py++) {
		for (let px = 0; px < w; px++) {
			const o = (py * w + px) * 4;
			const lum = (data[o] + data[o + 1] + data[o + 2]) / 3;
			if (lum < 34) continue;
			const nx = ((px + 0.5) / w) * 2 - 1;
			const ny = 1 - ((py + 0.5) / h) * 2;
			const zx = (nx - focus.x) * zoom + to.x;
			const zy = (ny - focus.y) * zoom + to.y;
			if (Math.abs(zx) > 1.12 || Math.abs(zy) > 1.12) continue;
			candidates.push(o, zx, zy);
		}
	}
	const total = candidates.length / 3;
	const count = Math.min(MAX_HANDOFF, total);
	const points = new Float32Array(count * HANDOFF_STRIDE);
	// An even stride through the candidates keeps the picture's shape.
	const step = total / Math.max(1, count);
	for (let i = 0; i < count; i++) {
		const c = Math.floor(i * step) * 3;
		const o = candidates[c];
		const lum = (data[o] + data[o + 1] + data[o + 2]) / (3 * 255);
		points.set(
			[candidates[c + 1] * aspect, candidates[c + 2], lum, 0, data[o] / 255, data[o + 1] / 255, data[o + 2] / 255, 0],
			i * HANDOFF_STRIDE
		);
	}
	return { points, count };
}

export function setHandoff(h: Handoff) {
	pending = h;
}

/** The handoff for this route, if one was made in the last few seconds. Taken once. */
export function takeHandoff(maxAgeMs = 4000): Handoff | null {
	const h = pending;
	pending = null;
	if (!h || performance.now() - h.createdAt > maxAgeMs || h.count === 0) return null;
	return h;
}

/** The destination's field is showing the handed-off particles: the veil may lift. */
export function markVeilReady() {
	veil.update((v) => (v ? { ...v, ready: true } : v));
}

/**
 * The destination's first real frame: mark the veil ready once the browser
 * has painted `frames` more frames (the GPU has drawn what was just uploaded).
 */
export function markVeilReadyAfterPaint(frames = 2) {
	if (typeof requestAnimationFrame === 'undefined') return markVeilReady();
	const step = (n: number) => (n <= 0 ? markVeilReady() : requestAnimationFrame(() => step(n - 1)));
	step(frames);
}
