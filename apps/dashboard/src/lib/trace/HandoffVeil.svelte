<script lang="ts">
	/**
	 * The dive between the Palace and an organ. Lives in the root layout so it
	 * outlives the Palace route: it shows the frame captured at the click, then
	 * carries the camera into the clicked organ (that organ slides to the
	 * centre and fills the screen) while the destination boots underneath.
	 * When the destination's field is showing the same particles (handoff.ts),
	 * the veil lifts, and what is left on screen is the destination itself.
	 */
	import { veil } from './handoff';

	const DIVE_MS = 720;
	const LIFT_MS = 380;
	const GIVE_UP_MS = 3000;

	let host = $state<HTMLDivElement | null>(null);
	let diving = $state(false);
	let lifting = $state(false);
	let shownAt = 0;
	let liftTimer: ReturnType<typeof setTimeout> | null = null;
	let giveUp: ReturnType<typeof setTimeout> | null = null;

	const current = $derived($veil);

	$effect(() => {
		const v = current;
		if (!v || !host) return;
		if (shownAt !== v.startedAt) {
			shownAt = v.startedAt;
			diving = false;
			lifting = false;
			host.replaceChildren(v.frame);
			v.frame.className = 'veil-frame';
			v.frame.style.transformOrigin = `${v.focusX}px ${v.focusY}px`;
			// Next frame: start the dive from exactly where the Palace was.
			const started = v.startedAt;
			requestAnimationFrame(() =>
				requestAnimationFrame(() => {
					// A throttled frame can arrive after the veil has already lifted.
					if (shownAt !== started || lifting || !$veil) return;
					diving = true;
					// A burst keeps flying on its own (the burst animation below).
					if (v.burst) return;
					v.frame.style.transform = `translate(${(v.toX ?? innerWidth / 2) - v.focusX}px, ${(v.toY ?? innerHeight / 2) - v.focusY}px) scale(${v.scale})`;
				})
			);
			if (giveUp) clearTimeout(giveUp);
			giveUp = setTimeout(lift, GIVE_UP_MS);
		}
		if (v.ready && !lifting) {
			// Never cut the dive short: lift once it has landed.
			const wait = Math.max(0, (v.hold ?? DIVE_MS) - (performance.now() - v.startedAt));
			if (liftTimer) clearTimeout(liftTimer);
			liftTimer = setTimeout(lift, wait);
		}
	});

	function lift() {
		if (lifting) return;
		lifting = true;
		// One lift per dive: a timer left from this dive must never lift the next one.
		if (giveUp) clearTimeout(giveUp);
		if (liftTimer) clearTimeout(liftTimer);
		giveUp = null;
		liftTimer = null;
		setTimeout(() => {
			veil.set(null);
			host?.replaceChildren();
			diving = false;
			lifting = false;
		}, LIFT_MS);
	}
</script>

<div
	class="veil"
	class:on={!!current}
	class:diving
	class:lifting
	class:burst={!!current?.burst}
	style:--accent={current?.color ?? '#7ff3e6'}
	aria-hidden="true"
>
	<div class="veil-host" bind:this={host}></div>
</div>

<style>
	.veil {
		position: fixed;
		inset: 0;
		z-index: 140;
		pointer-events: none;
		background: #020307;
		opacity: 0;
		visibility: hidden;
	}
	.veil.on {
		opacity: 1;
		visibility: visible;
	}
	.veil.lifting {
		opacity: 0;
		transition: opacity 0.38s ease;
	}
	.veil-host {
		position: absolute;
		inset: 0;
		overflow: hidden;
	}
	.veil-host :global(.veil-frame) {
		position: absolute;
		inset: 0;
		width: 100%;
		height: 100%;
		/* The camera arrives already moving (the Palace's dolly, the organ's
		   flight to a memory): the plunge starts fast and settles. */
		transition:
			transform 0.72s cubic-bezier(0.22, 0.55, 0.32, 1),
			filter 0.72s ease-out;
		will-change: transform, filter;
	}
	/* A Palace burst never stands still while the page boots: the captured
	   light keeps flying out, turning and running through every colour. */
	.veil.burst .veil-host :global(.veil-frame) {
		transition: none;
		animation: veil-burst 1.2s cubic-bezier(0.15, 0.55, 0.3, 1) forwards;
	}
	@keyframes veil-burst {
		from {
			transform: scale(1) rotate(0deg);
			filter: hue-rotate(0deg) saturate(1.3) brightness(1.15);
		}
		to {
			transform: scale(1.7) rotate(14deg);
			filter: hue-rotate(200deg) saturate(2.2) brightness(1.05) contrast(1.15);
		}
	}
	.veil.diving:not(.burst) .veil-host :global(.veil-frame) {
		filter: brightness(1.18) saturate(1.15) blur(0.6px);
	}
	@media (prefers-reduced-motion: reduce) {
		.veil-host :global(.veil-frame) {
			transition: none;
			animation: none !important;
		}
	}
</style>
