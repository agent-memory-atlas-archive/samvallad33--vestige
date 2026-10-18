<script lang="ts">
	import { onDestroy } from 'svelte';
	import { page } from '$app/stores';
	import { base } from '$app/paths';
	import ObservatoryCanvas from '$lib/components/ObservatoryCanvas.svelte';
	import type { ObservatoryEngine } from '$lib/observatory/engine';
	import { DIVE_END, PalaceBrainPass } from '$lib/observatory/palace-brain-pass';
	import { PalaceLogoPass } from '$lib/observatory/palace-logo-pass';
	import { buildOrganLabels } from '$lib/observatory/palace-labels';
	import { TextLayerPass, type TextLayerItem } from '$lib/observatory/text/text-layer';
	import {
		BITEMPORAL,
		IMMUNE,
		RETENTION,
		rgb01
	} from '$lib/observatory/cognitive-palette';
	import { ORGAN_REGIONS, regionByHref, type OrganRegion } from '$lib/observatory/palace-map';
	import { findRoute } from '$lib/os-routes';
	import { goto, preloadCode } from '$app/navigation';
	import { captureFrame, samplePoints, setHandoff, veil } from '$lib/trace/handoff';
	import { cursorHot } from '$lib/trace/cursor';

	/** Organs whose field takes the Palace's particles over (they run on TraceStage). */
	const TAKES_HANDOFF = new Set(['/trace', '/ledger', '/guard', '/healing', '/causal', '/system', '/graph']);

	const TITLE = [...rgb01('#F5FFF2'), 1] satisfies [number, number, number, number];
	const SUB = [...rgb01('#9DFFEB'), 1] satisfies [number, number, number, number];
	const DIM = [...rgb01('#7DAFA9'), 0.82] satisfies [number, number, number, number];

	const FAMILY_COLOR: Record<OrganRegion['family'], string> = {
		reasoning: RETENTION.bridge,
		memory: RETENTION.recall,
		immune: IMMUNE.veto,
		signal: BITEMPORAL.supersession,
		temporal: BITEMPORAL.txShadow,
		system: RETENTION.luciferin
	};

	let hostEl: HTMLDivElement | null = $state(null);
	let engineRef: ObservatoryEngine | null = null;
	let pass: PalaceBrainPass | null = null;
	let textPass: TextLayerPass | null = null;
	let logoPass: PalaceLogoPass | null = null;
	let cursorSmoothed: { x: number; y: number } | null = null;
	let hoveredHref: string | null = $state(null);
	let pressed: { x: number; y: number; href: string | null } | null = null;
	let clickPoint = { x: 0, y: 0 };
	let navigationStarted = false;
	let reducedMotion = false;
	let lastLabelKey = '';

	let freezeFrame = $derived.by(() => {
		const raw = $page.url.searchParams.get('frame');
		if (raw === null) return null;
		const n = Number(raw);
		return Number.isFinite(n) ? Math.floor(n) : null;
	});

	onDestroy(() => {
		cursorHot.set(false);
		if (engineRef && pass) engineRef.removePass(pass);
		else pass?.dispose();
		if (engineRef && textPass) engineRef.removePass(textPass);
		else textPass?.dispose();
		if (engineRef && logoPass) engineRef.removePass(logoPass);
		else logoPass?.dispose();
		logoPass = null;
		pass = null;
		textPass = null;
		engineRef = null;
	});

	async function handleReady(engine: ObservatoryEngine) {
		try {
			engineRef = engine;
			reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
			const swarm = new PalaceBrainPass(engine);
			pass = swarm;
			swarm.setReducedMotion(reducedMotion);
			engine.addPass(swarm);
			swarm.uploadRegions(ORGAN_REGIONS);
			// The logo stands top-centre, over the brain it is born from.
			const logo = new PalaceLogoPass(engine, reducedMotion);
			logoPass = logo;
			placeLogo();
			engine.addPass(logo);
			const text = new TextLayerPass(engine);
			textPass = text;
			await text.init();
			engine.addPass(text);
			engine.demoClock.reset();
			// Words and organ names together, now that the font is loaded (a frame
			// can come before it, and its layout would be lost).
			refreshText();
		} catch (error) {
			console.error('[palace] Failed to initialize swarm:', error);
		}
	}

	function sanitizeAscii(value: string): string {
		return value
			.replace(/[—–]/g, '-')
			.replace(/[‘’]/g, "'")
			.replace(/[“”]/g, '"')
			.replace(/…/g, '...')
			.replace(/[^\x20-\x7E]/g, '?');
	}

	/** The logo's place: top-centre, as wide as the screen allows. */
	function placeLogo() {
		const w = engineRef?.params[6] || innerWidth;
		const h = engineRef?.params[7] || innerHeight;
		const aspect = w / Math.max(1, h);
		if (aspect >= 1) logoPass?.setPlace(0, 0.73, Math.min(1.05, 1.9 / aspect));
		else logoPass?.setPlace(0, 0.8, 1.6);
	}

	function buildText(): TextLayerItem[] {
		// Under the logo, centred (the font is monospaced: about 0.6 em a letter).
		const sub = sanitizeAscii(`MEMORY PALACE  -  ${ORGAN_REGIONS.length} LIVING ORGANS  -  CLICK ONE TO ENTER`);
		const subSize = 0.026;
		const items: TextLayerItem[] = [
			{
				id: 'palace:sub',
				kind: 'palace-hud',
				text: sub,
				x: -(sub.length * 0.6 * subSize) / 2,
				y: 0.53,
				size: subSize,
				color: SUB,
				depth: 1,
				weight: 0.86,
				revealSpan: 28,
				maxWidthEm: 90
			}
		];

		if (hoveredHref) {
			// The hovered organ's name and what it is for take the subtitle's
			// place under the logo, centred, never on a shape.
			const region = regionByHref(hoveredHref);
			const route = findRoute(hoveredHref);
			if (region) {
				const accent = [...rgb01(FAMILY_COLOR[region.family]), 1] satisfies [number, number, number, number];
				const name = navigationStarted ? `ENTERING ${region.label}` : region.label;
				const purpose = sanitizeAscii(route?.purpose ?? 'ENTER THIS ORGAN');
				const purposeShown = purpose.length > 96 ? `${purpose.slice(0, 95)}...` : purpose;
				items.length = 0;
				items.push(
					{
						id: 'palace:focus-label',
						kind: 'palace-focus',
						text: name,
						x: -(name.length * 0.6 * 0.034) / 2,
						y: 0.535,
						size: 0.034,
						color: accent,
						depth: 1,
						weight: 1,
						revealSpan: 18
					},
					{
						id: 'palace:focus-purpose',
						kind: 'palace-focus',
						text: purposeShown,
						x: -(purposeShown.length * 0.6 * 0.019) / 2,
						y: 0.48,
						size: 0.019,
						color: TITLE,
						depth: 1,
						weight: 0.8,
						revealSpan: 30,
						maxWidthEm: 100
					}
				);
			}
		}
		return items;
	}

	function refreshText() {
		const hud = buildText();
		const labels = pass
			? buildOrganLabels(pass.getScreenPositions(), {
					hoveredHref,
					dimUnhovered: Boolean(hoveredHref),
					aspect:
						(engineRef?.params[6] || 0) / Math.max(1, engineRef?.params[7] || 1)
				})
			: [];
		textPass?.setText([...hud, ...labels]);
	}

	// The organs never move, so their names are laid out once per canvas size
	// (hover and navigation refresh them directly), never every frame.
	function handleFrame() {
		const key = `${engineRef?.params[6] ?? 0}x${engineRef?.params[7] ?? 0}`;
		if (key === lastLabelKey) return;
		lastLabelKey = key;
		placeLogo();
		refreshText();
	}

	function pointerToNdc(e: PointerEvent): { x: number; y: number } | null {
		if (!hostEl) return null;
		const rect = hostEl.getBoundingClientRect();
		if (rect.width <= 0 || rect.height <= 0) return null;
		return {
			x: ((e.clientX - rect.left) / rect.width) * 2 - 1,
			y: -(((e.clientY - rect.top) / rect.height) * 2 - 1)
		};
	}

	function writeCursorLens(ndc: { x: number; y: number }) {
		if (!hostEl || !engineRef) return;
		const rect = hostEl.getBoundingClientRect();
		const aspect = Math.max(0.0001, rect.width / Math.max(1, rect.height));
		const raw = { x: ndc.x * Math.max(aspect, 1), y: ndc.y / Math.min(aspect, 1) };
		const prev = cursorSmoothed ?? raw;
		const next = { x: prev.x + (raw.x - prev.x) * 0.35, y: prev.y + (raw.y - prev.y) * 0.35 };
		cursorSmoothed = next;
		engineRef.setCursorPreNdc(next.x, next.y, next.x - prev.x, next.y - prev.y);
	}

	function handlePointerMove(e: PointerEvent) {
		const ndc = pointerToNdc(e);
		if (!ndc) return;
		writeCursorLens(ndc);
		if (!pass || pass.isBursting) return;
		const hit = pass.pickAt(ndc.x, ndc.y);
		const nextHref = hit?.href ?? null;
		if (nextHref !== hoveredHref) {
			hoveredHref = nextHref;
			// The organ's code is fetched while the pointer rests on it, so the
			// click's flight never waits on the network.
			if (nextHref) void preloadCode(`${base}${nextHref}`).catch(() => {});
			pass.setHovered(hit?.index ?? -1);
			refreshText();
			if (hostEl) hostEl.style.cursor = nextHref ? 'pointer' : 'crosshair';
			cursorHot.set(!!nextHref);
		}
	}

	function handlePointerLeave() {
		if (pass?.isBursting) return;
		pressed = null;
		cursorSmoothed = null;
		hoveredHref = null;
		pass?.setHovered(-1);
		cursorHot.set(false);
		engineRef?.setCursorPreNdc(999, 999, 0, 0);
		refreshText();
		if (hostEl) hostEl.style.cursor = 'crosshair';
	}

	function handlePointerDown(e: PointerEvent) {
		if (e.button !== 0 || !pass || pass.isBursting) return;
		const ndc = pointerToNdc(e);
		if (!ndc) return;
		pressed = { x: e.clientX, y: e.clientY, href: pass.pickAt(ndc.x, ndc.y)?.href ?? null };
	}

	function handlePointerCancel() {
		pressed = null;
	}

	function handlePointerUp(e: PointerEvent) {
		const down = pressed;
		pressed = null;
		if (!down || !pass || pass.isBursting) return;
		if (Math.hypot(e.clientX - down.x, e.clientY - down.y) > 9) return;
		const ndc = pointerToNdc(e);
		if (!ndc) return;
		const hit = pass.pickAt(ndc.x, ndc.y);
		if (!hit || hit.href !== down.href) return;
		clickPoint = { x: e.clientX, y: e.clientY };
		hoveredHref = hit.href;
		pass.setHovered(hit.index);
		navigationStarted = true;
		// The words leave with the click: the flight is light only, and the
		// frame the organ's page starts from carries no letters.
		textPass?.setText([]);
		logoPass?.leave();
		if (hostEl) hostEl.style.cursor = 'progress';
		void enter(hit);
	}

	/**
	 * Go into the organ, in one continuous flight:
	 *   1. live, on the GPU: the clicked organ flies to the centre of the
	 *      screen and grows while the rest of the Palace is drawn in (the
	 *      burst's dive, about 0.6 s);
	 *   2. the frame is captured there and the camera keeps going into the
	 *      organ (HandoffVeil) while the organ's page boots underneath;
	 *   3. the destination's field starts on the captured particles (the
	 *      organs on TraceStage, lib/trace/handoff.ts), or the veil holds
	 *      until the destination says its first real frame is up, then lifts.
	 */
	async function enter(hit: { index: number; href: string }) {
		const href = `${base}${hit.href}`;
		void preloadCode(href).catch(() => {});
		const canvas = hostEl?.querySelector('canvas');
		if (reducedMotion || !canvas || !hostEl || !pass) {
			await goto(href);
			return;
		}
		if (!pass.startBurst(hit.href, () => void land(hit, canvas), DIVE_END)) await land(hit, canvas);
	}

	/** The organ has burst wide: capture its light and go in on it. */
	async function land(hit: { index: number; href: string }, canvas: HTMLCanvasElement) {
		const href = `${base}${hit.href}`;
		if (!hostEl) {
			await goto(href);
			return;
		}
		const region = regionByHref(hit.href);
		const rect = hostEl.getBoundingClientRect();
		const focus = { x: 0, y: 0 };
		// The burst already fills the screen; the camera only pushes on a little.
		const zoom = 1.3;
		const frame = await captureFrame(canvas, 0.5, engineRef);
		const takes = TAKES_HANDOFF.has(hit.href);
		if (takes) {
			const { points, count } = samplePoints(frame, focus, zoom, rect.width / Math.max(1, rect.height));
			setHandoff({ points, count, createdAt: performance.now(), href: hit.href });
		}
		veil.set({
			frame,
			focusX: rect.left + rect.width / 2,
			focusY: rect.top + rect.height / 2,
			scale: zoom,
			color: region ? FAMILY_COLOR[region.family] : RETENTION.luciferin,
			startedAt: performance.now(),
			// Every destination says when its first real frame is up (TraceStage,
			// Field, Cinema, Witness); the veil gives up after a few seconds.
			ready: false,
			// The burst keeps moving on the page itself: lift as soon as it is up.
			hold: 160,
			burst: true
		});
		await goto(href);
	}
</script>

<svelte:head>
	<title>Memory Palace · VestigeOS</title>
</svelte:head>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
	bind:this={hostEl}
	class="palace-host fixed inset-0 bg-[#020307]"
	role="application"
	aria-label="VestigeOS Memory Palace. Nine living cognitive organs. Use the Command palette for keyboard navigation."
	onpointerdown={handlePointerDown}
	onpointerup={handlePointerUp}
	onpointercancel={handlePointerCancel}
	onpointermove={handlePointerMove}
	onpointerleave={handlePointerLeave}
>
	<ObservatoryCanvas
		demo="recall-path"
		seed="vestige-palace-swarm-v2"
		{freezeFrame}
		onframe={handleFrame}
		onready={handleReady}
	/>
</div>

<style>
	.palace-host {
		cursor: crosshair;
		touch-action: manipulation;
		isolation: isolate;
	}

	/* Atmosphere visible during adapter boot; the GPU canvas takes over without a
	   color discontinuity because both layers share the blackwater medium. */
	.palace-host::before {
		content: '';
		position: absolute;
		inset: 0;
		z-index: -1;
		background:
			radial-gradient(circle at 50% 44%, rgba(41, 242, 169, 0.09), transparent 26%),
			radial-gradient(circle at 24% 18%, rgba(29, 214, 255, 0.06), transparent 34%),
			#020307;
	}
</style>
