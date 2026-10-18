/**
 * alive — every letter on an organ is part of the field.
 *
 *   use:alive={{ text }}                    decode: each letter resolves out of
 *                                           drifting glyphs when it appears or
 *                                           changes, left to right;
 *   use:alive={{ text, glow: true }}        and the letters near the cursor
 *                                           light up and lift as it passes;
 *   use:alive={{ text, count: true }}       a number counts to its new value.
 *
 * The action owns the element's content (give the element no children), so
 * Svelte never fights it for the text. Reduced motion skips the animation.
 */

const GLYPHS = '·:+*/\\|<>=~01░▒';
const DECODE_MS = 520;
const STAGGER_MS = 9;

export interface AliveOptions {
	text: string;
	/** Letters near the cursor glow and lift. */
	glow?: boolean;
	/** Count numbers up to their new value instead of decoding. */
	count?: boolean;
	/** Skip the decode (still glows). */
	still?: boolean;
}

const reduced = () => typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches;

// One pointer listener for the whole document; glowing elements subscribe.
const glowing = new Set<GlowTarget>();
let pointer = { x: -9999, y: -9999 };
let glowFrame = 0;
function onPointer(e: PointerEvent) {
	pointer = { x: e.clientX, y: e.clientY };
	if (!glowFrame) glowFrame = requestAnimationFrame(paintGlow);
}
function onLeaveDoc() {
	pointer = { x: -9999, y: -9999 };
	if (!glowFrame) glowFrame = requestAnimationFrame(paintGlow);
}
function paintGlow() {
	glowFrame = 0;
	for (const g of glowing) g.paint();
}
function subscribe(g: GlowTarget) {
	if (glowing.size === 0 && typeof window !== 'undefined') {
		window.addEventListener('pointermove', onPointer, { passive: true });
		document.documentElement.addEventListener('pointerleave', onLeaveDoc);
	}
	glowing.add(g);
}
function unsubscribe(g: GlowTarget) {
	glowing.delete(g);
	if (glowing.size === 0 && typeof window !== 'undefined') {
		window.removeEventListener('pointermove', onPointer);
		document.documentElement.removeEventListener('pointerleave', onLeaveDoc);
	}
}

interface GlowTarget {
	paint(): void;
}

export function alive(node: HTMLElement, opts: AliveOptions) {
	let current = opts;
	let letters: HTMLSpanElement[] = [];
	let centers: { x: number; y: number }[] = [];
	let raf = 0;
	let lastGlow: number[] = [];
	let shownNumber: number | null = null;

	const target: GlowTarget = {
		paint() {
			if (!letters.length) return;
			if (!centers.length) measure();
			for (let i = 0; i < letters.length; i++) {
				const c = centers[i];
				const d = Math.hypot(c.x - pointer.x, c.y - pointer.y);
				// A soft, short reach: the words brighten as the pointer reads them.
				const g = d > 80 ? 0 : Math.round((1 - d / 80) ** 2 * 60) / 100;
				if (g !== lastGlow[i]) {
					lastGlow[i] = g;
					letters[i].style.setProperty('--g', String(g));
				}
			}
		}
	};

	function measure() {
		centers = letters.map((l) => {
			const r = l.getBoundingClientRect();
			return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
		});
	}
	const onResize = () => {
		centers = [];
	};

	function build(text: string) {
		cancelAnimationFrame(raf);
		node.textContent = '';
		letters = [];
		lastGlow = [];
		centers = [];
		node.setAttribute('aria-label', text);
		// Words stay unbreakable; letters inside them animate on their own.
		const words = text.split(/(\s+)/);
		for (const w of words) {
			if (/^\s+$/.test(w)) {
				node.appendChild(document.createTextNode(w));
				continue;
			}
			const word = document.createElement('span');
			word.className = 'alive-word';
			word.setAttribute('aria-hidden', 'true');
			for (const ch of w) {
				const s = document.createElement('span');
				s.className = 'alive-l';
				s.textContent = ch;
				s.dataset.ch = ch;
				word.appendChild(s);
				letters.push(s);
			}
			node.appendChild(word);
		}
	}

	function decode(fromScratch: boolean) {
		if (reduced() || current.still) return;
		const start = performance.now();
		const n = letters.length;
		const step = Math.min(STAGGER_MS, 420 / Math.max(1, n));
		const tick = (now: number) => {
			let pending = false;
			for (let i = 0; i < n; i++) {
				const s = letters[i];
				const at = start + i * step;
				const done = at + DECODE_MS * (0.45 + 0.55 * ((i * 7919) % 13) / 13);
				if (now >= done) {
					if (s.textContent !== s.dataset.ch) s.textContent = s.dataset.ch ?? '';
					s.classList.remove('alive-hot');
				} else {
					pending = true;
					if (now >= at || fromScratch) {
						s.textContent = /\s/.test(s.dataset.ch ?? '') ? s.dataset.ch! : GLYPHS[(Math.random() * GLYPHS.length) | 0];
						s.classList.add('alive-hot');
					}
				}
			}
			if (pending) raf = requestAnimationFrame(tick);
		};
		raf = requestAnimationFrame(tick);
	}

	function countTo(text: string) {
		const target = Number(text.replace(/,/g, ''));
		if (!Number.isFinite(target) || reduced()) {
			node.textContent = text;
			shownNumber = Number.isFinite(target) ? target : null;
			return;
		}
		const from = shownNumber ?? 0;
		shownNumber = target;
		const start = performance.now();
		const dur = 700;
		const decimals = (text.split('.')[1] ?? '').length;
		const fmt = (v: number) => v.toLocaleString(undefined, { minimumFractionDigits: decimals, maximumFractionDigits: decimals });
		cancelAnimationFrame(raf);
		const tick = (now: number) => {
			const t = Math.min(1, (now - start) / dur);
			const e = 1 - (1 - t) ** 3;
			node.textContent = fmt(from + (target - from) * e);
			if (t < 1) raf = requestAnimationFrame(tick);
			else node.textContent = text;
		};
		raf = requestAnimationFrame(tick);
	}

	function render(next: AliveOptions, first: boolean) {
		if (next.count) {
			countTo(next.text);
			return;
		}
		build(next.text);
		decode(first);
		if (next.glow) subscribe(target);
		else unsubscribe(target);
	}

	node.classList.add('alive');
	render(opts, true);
	if (typeof window !== 'undefined') window.addEventListener('resize', onResize);

	return {
		update(next: AliveOptions) {
			const changed = next.text !== current.text || next.glow !== current.glow || next.count !== current.count;
			current = next;
			if (changed) render(next, false);
		},
		destroy() {
			cancelAnimationFrame(raf);
			unsubscribe(target);
			if (typeof window !== 'undefined') window.removeEventListener('resize', onResize);
		}
	};
}
