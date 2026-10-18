/**
 * Going inside a memory from anywhere it is drawn.
 *
 * The view is captured where the memory is, the camera dives into it (the
 * root-mounted HandoffVeil), and the memory page starts every particle on the
 * captured picture, landed exactly where the memory's own form will assemble,
 * so the light you clicked becomes the memory.
 */
import { goto } from '$app/navigation';
import { base } from '$app/paths';
import { captureFrame, samplePoints, setHandoff, veil } from '$lib/trace/handoff';
import { insideRegion } from './inside-pass';

export function memoryHref(id: string): string {
	return `${base}/memory?id=${encodeURIComponent(id)}`;
}

export async function enterMemory(
	id: string,
	from?: {
		canvas: HTMLCanvasElement | null;
		focus: { x: number; y: number } | null;
		color?: string;
		/** The engine drawing the canvas, so the copy is taken right after a frame it renders. */
		engine?: { afterNextRender(cb: () => void): void } | null;
	}
): Promise<void> {
	const href = memoryHref(id);
	const reduced = typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches;
	const canvas = from?.canvas ?? null;
	const focus = from?.focus ?? null;
	if (reduced || !canvas || !focus) {
		await goto(href);
		return;
	}
	const rect = canvas.getBoundingClientRect();
	const aspect = rect.width / Math.max(1, rect.height);
	const region = insideRegion(aspect);
	const to = { x: (region.left + region.right) / 2, y: (region.top + region.bottom) / 2 };
	const zoom = 3;
	const frame = await captureFrame(canvas, 0.5, from?.engine);
	const { points, count } = samplePoints(frame, focus, zoom, aspect, to);
	setHandoff({ points, count, createdAt: performance.now(), href: '/memory' });
	veil.set({
		frame,
		focusX: rect.left + ((focus.x + 1) / 2) * rect.width,
		focusY: rect.top + ((1 - focus.y) / 2) * rect.height,
		toX: rect.left + ((to.x + 1) / 2) * rect.width,
		toY: rect.top + ((1 - to.y) / 2) * rect.height,
		scale: zoom,
		color: from?.color ?? '#7ff3e6',
		startedAt: performance.now(),
		ready: false
	});
	await goto(href);
}
