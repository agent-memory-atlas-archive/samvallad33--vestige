<script lang="ts">
	/**
	 * The dashboard's own cursor, drawn BY the system cursor: the dot and the
	 * ring are CSS cursor images, so the operating system draws them at the
	 * hardware pointer with zero lag, and there is only ever one pointer. Over
	 * anything that can be clicked (a memory in the field, a link, a button, a
	 * field) the dot opens into a ring. A faint glow eases after it on real
	 * time, so it feels the same at 60 Hz and 120 Hz. Touch screens and reduced
	 * motion keep the platform cursor.
	 */
	import { onMount } from 'svelte';
	import { cursorHot } from './cursor';

	/** A memory under the pointer in the field (the page knows; the DOM does not). */
	let { hot = false }: { hot?: boolean } = $props();

	let glow = $state<HTMLDivElement | null>(null);
	let on = $state(false);
	let overControl = $state(false);
	let at = { x: -999, y: -999 };
	let trail = { x: -999, y: -999 };
	let raf = 0;
	let last = 0;

	const CONTROL = 'a, button, input, textarea, select, label, [role="button"], [data-trace-id]';

	function frame(t: number) {
		raf = 0;
		const dt = last ? Math.min(0.1, (t - last) / 1000) : 1 / 60;
		last = t;
		const k = 1 - Math.exp(-dt * 11);
		trail.x += (at.x - trail.x) * k;
		trail.y += (at.y - trail.y) * k;
		if (glow) glow.style.transform = `translate(${trail.x}px, ${trail.y}px)`;
		if (Math.abs(at.x - trail.x) + Math.abs(at.y - trail.y) > 0.3) raf = requestAnimationFrame(frame);
		else last = 0;
	}

	$effect(() => {
		document.documentElement.classList.toggle('cursor-light-hot', on && (hot || $cursorHot || overControl));
	});

	onMount(() => {
		const usable = matchMedia('(hover: hover) and (pointer: fine)').matches && !matchMedia('(prefers-reduced-motion: reduce)').matches;
		if (!usable) return;
		const root = document.documentElement;
		root.classList.add('cursor-light-on');
		const move = (e: PointerEvent) => {
			at = { x: e.clientX, y: e.clientY };
			if (trail.x < -900) trail = { ...at };
			overControl = !!(e.target as Element | null)?.closest?.(CONTROL);
			on = true;
			if (!raf) raf = requestAnimationFrame(frame);
		};
		const out = (e: PointerEvent) => {
			if (e.relatedTarget) return;
			on = false;
		};
		window.addEventListener('pointermove', move, { passive: true });
		window.addEventListener('pointerout', out);
		return () => {
			window.removeEventListener('pointermove', move);
			window.removeEventListener('pointerout', out);
			root.classList.remove('cursor-light-on', 'cursor-light-hot');
			cancelAnimationFrame(raf);
		};
	});
</script>

<div class="cursor-glow" class:on bind:this={glow} aria-hidden="true"></div>

<style>
	/*
	 * One pointer, drawn by the system at the hardware position: a soft dot, and
	 * a ring over anything clickable. Plain fallbacks keep a cursor if an image
	 * cannot load.
	 */
	:global(html.cursor-light-on),
	:global(html.cursor-light-on *) {
		cursor:
			url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='24' height='24'%3E%3Ccircle cx='12' cy='12' r='6' fill='%237ff3e6' fill-opacity='0.18'/%3E%3Ccircle cx='12' cy='12' r='3.6' fill='%23e8fffb' stroke='%23020307' stroke-opacity='0.7' stroke-width='1'/%3E%3C/svg%3E")
				12 12,
			auto !important;
	}
	:global(html.cursor-light-on.cursor-light-hot),
	:global(html.cursor-light-on.cursor-light-hot *) {
		cursor:
			url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='32' height='32'%3E%3Ccircle cx='16' cy='16' r='11' fill='none' stroke='%23020307' stroke-opacity='0.6' stroke-width='2.6'/%3E%3Ccircle cx='16' cy='16' r='11' fill='none' stroke='%23e8fffb' stroke-width='1.3'/%3E%3Ccircle cx='16' cy='16' r='1.6' fill='%23e8fffb'/%3E%3C/svg%3E")
				16 16,
			pointer !important;
	}
	:global(html.cursor-light-on input[type='text']),
	:global(html.cursor-light-on input[type='search']),
	:global(html.cursor-light-on textarea) {
		cursor: text !important;
	}
	.cursor-glow {
		position: fixed;
		left: 0;
		top: 0;
		width: 0;
		height: 0;
		pointer-events: none;
		opacity: 0;
		transition: opacity 0.25s ease;
		z-index: 29;
		mix-blend-mode: screen;
	}
	.on {
		opacity: 1;
	}
	.cursor-glow::before {
		content: '';
		position: absolute;
		left: -60px;
		top: -60px;
		width: 120px;
		height: 120px;
		border-radius: 50%;
		background: radial-gradient(circle, rgba(127, 243, 230, 0.07) 0%, rgba(127, 243, 230, 0.02) 45%, transparent 70%);
	}
</style>
