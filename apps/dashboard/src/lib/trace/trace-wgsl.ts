/**
 * Trace — WGSL.
 *
 * A compute-simulated particle field (about 150k particles) and one instanced
 * draw of velocity-stretched sprites over a fullscreen backdrop. Every
 * particle belongs to something real:
 *
 *   kind 0  halo    — light orbiting just outside every island (one big
 *                     round world per group of the answer), in its colour;
 *   kind 1  memory  — a small galaxy per real memory, coloured by type,
 *                     sized by length, lit by retention;
 *   kind 2  stream  — light flowing along a recorded edge of the causal
 *                     walk, from the cause into the memory it explains.
 *
 * The compute pass springs each particle toward its target, adds curl-noise
 * flow, and applies the events as forces: an answer erupts from the question
 * (a new generation re-seeds every particle at the question and lets it fly),
 * a pick sends a shockwave through the field, the receipt check runs a hot
 * front along the river from the start of the log to the memory and seals it
 * with a second wave, a walk unfolds its causes out of the picked memory,
 * and the cursor drags a wake through whatever it passes.
 */

const COMMON = /* wgsl */ `
struct EP {
	frame: f32, loop_phase: f32, node_count: f32, edge_count: f32,
	path_count: f32, pulse: f32, viewport_w: f32, viewport_h: f32,
	brightness: f32, demo_id: f32, time: f32, capture_mode: f32,
	live_kind: f32, live_frame: f32, live_energy: f32, projection_days: f32,
};
// u0: aspect, memCount, edgeCount, riverY
// u1: selected index (-1 none), ignite age, walk age, answer age (s)
// u2: hover index (-1 none), river half-width, hover age, dt
// u3: selected xy, answer origin xy (world)
// u4: cursor xy, cursor velocity xy (world, per s)
// u5: verify age (-1 idle), sealed age (-1 none), seal ok (1/0), river half-height
// u6: answer generation, walk generation, focus (1 when a memory is picked), cursor active
// u7: now x (world; <= -9 when hidden), galaxy pool size, particles per memory, base per memory
// u8: hovered memory xy (world), cursor speed, 0
// u9: story (0 trace, 1 ledger, 2 guard, 3 healing, 4 causal, 5 system, 6 witness),
//     arrival mode (0 the story's own, 1 from the handoff, 2 keep positions),
//     hold (1 while the handed-off picture waits for the organ's data),
//     handoff point count
// u10: river deep rgb, 0   u11: river bright rgb, 0   u12: heat rgb, 0
// u13: camera centre xy (world), zoom, 0 — the view flies into a picked memory
// u14..u17: the page's blocks of words (NDC x0, y0, x1, y1; empty when x1 <= x0)
// u18: islands (count, clocks 1/0, opened 1/0, a clock's marks)   u19: orb radius, device pixel ratio, span x0, span x1
struct TU {
	u0: vec4f, u1: vec4f, u2: vec4f, u3: vec4f, u4: vec4f, u5: vec4f, u6: vec4f,
	u7: vec4f, u8: vec4f, u9: vec4f, u10: vec4f, u11: vec4f, u12: vec4f, u13: vec4f,
	u14: vec4f, u15: vec4f, u16: vec4f, u17: vec4f, u18: vec4f, u19: vec4f,
};
// How far inside one of the page's blocks of words a screen point is (0..1,
// soft at the edges): the field goes quiet there so words never sit on light.
fn quietIn(r: vec4f, p: vec2f) -> f32 {
	if (r.z <= r.x) { return 0.0; }
	let e = 0.11;
	let ix = smoothstep(r.x - e, r.x + e, p.x) * (1.0 - smoothstep(r.z - e, r.z + e, p.x));
	let iy = smoothstep(r.y - e, r.y + e, p.y) * (1.0 - smoothstep(r.w - e, r.w + e, p.y));
	return ix * iy;
}
fn quietAt(p: vec2f) -> f32 {
	return max(max(quietIn(tu.u14, p), quietIn(tu.u15, p)), max(quietIn(tu.u16, p), quietIn(tu.u17, p)));
}
// World to clip space through the camera.
fn toClip(w: vec2f) -> vec4f {
	let z = max(tu.u13.z, 0.1);
	return vec4f((w.x - tu.u13.x) * z / tu.u0.x, (w.y - tu.u13.y) * z, 0.0, 1.0);
}
// a: x, y, size, retention   b: r, g, b, flags (1 answer, 2 walk, 4 picked, 8 cause on the river, 16 future)
struct Mem { a: vec4f, b: vec4f };
// a: from xy, to xy   b: causal (1) or touched (0), strength, appear delay, arch height (0: bow)
// c: r, g, b, clock (0: the walk's, 1: the answer's — a recorded link that came with it)
struct Edge { a: vec4f, b: vec4f, c: vec4f };
// kind, owner, seed, u
struct P { v: vec4f };
// pv: pos xy, vel xy   m: answer gen seen, walk gen seen, heat, handoff point + 1 (0 none)
struct S { pv: vec4f, m: vec4f };
// One handed-off point from the Palace: a: world xy, brightness   b: rgb
struct H { a: vec4f, b: vec4f };

const PI = 3.14159265;
const BINS = 256u;

// Sine-free hashes (Hoskins), cheap on every GPU.
fn h11(n: f32) -> f32 {
	var p = fract(n * 0.1031);
	p *= p + 33.33;
	p *= p + p;
	return fract(p);
}
fn h21(q: vec2f) -> f32 {
	var p3 = fract(vec3f(q.x, q.y, q.x) * 0.1031);
	p3 += dot(p3, p3.yzx + 33.33);
	return fract((p3.x + p3.y) * p3.z);
}
// Every colour, round a wheel.
fn rainbow(x: f32) -> vec3f {
	return 0.5 + 0.5 * cos(6.2831853 * (x + vec3f(0.0, 0.33, 0.67)));
}
fn ease(t: f32) -> f32 {
	let x = clamp(t, 0.0, 1.0);
	return x * x * (3.0 - 2.0 * x);
}
// Every organ tells its own story with the same particles.
const TRACE = 0u;
const LEDGER = 1u;
const GUARD = 2u;
const HEALING = 3u;
const CAUSAL = 4u;
const SYSTEM = 5u;
const WITNESS = 6u;
fn story() -> u32 {
	return u32(tu.u9.x + 0.5);
}
// Guard's gate: a level across the middle of the field; whatever falls
// through it flashes.
fn gateY() -> f32 {
	return tu.u0.w;
}
// System's heartbeat: a lub and a dub, about 51 a minute.
fn heartbeat(t: f32) -> f32 {
	let p = fract(t * 0.85);
	return exp(-p * 10.0) + 0.6 * select(0.0, exp(-(p - 0.2) * 12.0), p > 0.2);
}
// Ledger's clock hand: it keeps going round, re-reading every day.
fn handAngle(t: f32) -> f32 {
	return fract(t / 8.0) * 6.2831853;
}
// When the particles arriving at x have landed, per organ (matches arriveRelease).
fn landAt(x: f32) -> f32 {
	let halfW = max(tu.u2.y, 0.01);
	let xn = clamp((x + halfW) / (2.0 * halfW), 0.0, 1.0);
	let st = story();
	if (st == LEDGER) { return 0.65 + xn * 2.4; }
	if (st == GUARD) { return 0.9; }
	if (st == HEALING) { return 1.4 + xn * 1.7; }
	if (st == CAUSAL) { return 0.7 + (1.0 - xn) * 2.2; }
	if (st == SYSTEM) { return 1.9; }
	if (st == WITNESS) { return 0.7 + (1.0 - abs(xn - 0.5) * 2.0) * 1.6; }
	return 1.3;
}
// A galaxy particle's memory: the pool is shared out evenly over the placed
// memories (u7.z particles each), so its owner is its pool index divided.
fn galaxyOwner(poolIndex: u32, perMem: f32) -> u32 {
	return poolIndex / max(1u, u32(perMem + 0.5));
}
`;

const ISLANDS = /* wgsl */ `
// The islands: x, y, rx, ry, rgb and lit, 8 floats each.
struct Isl { c: vec2f, rx: f32, ry: f32, col: vec3f, lit: f32 };
fn island(i: u32) -> Isl {
	let k = min(i, 31u) * 8u;
	var o: Isl;
	o.c = vec2f(density[k], density[k + 1u]);
	o.rx = max(density[k + 2u], 1e-3);
	o.ry = max(density[k + 3u], 1e-3);
	o.col = vec3f(density[k + 4u], density[k + 5u], density[k + 6u]);
	o.lit = density[k + 7u];
	return o;
}
fn islandCount() -> u32 {
	return u32(tu.u18.x + 0.5);
}
// The island a halo particle circles.
fn haloOf(seed: f32) -> u32 {
	let n = max(f32(islandCount()), 1.0);
	return u32(min(floor(h11(seed * 13.7 + 0.31) * n), n - 1.0));
}
`;


const UNIVERSE = /* wgsl */ `
// ── Each organ's universe: the world its lanes live in ──────────────────────
fn vnoise(p: vec2f) -> f32 {
	let i = floor(p);
	let f = fract(p);
	let u = f * f * (3.0 - 2.0 * f);
	return mix(mix(h21(i), h21(i + vec2f(1.0, 0.0)), u.x), mix(h21(i + vec2f(0.0, 1.0)), h21(i + vec2f(1.0, 1.0)), u.x), u.y);
}
fn fbm(p0: vec2f) -> f32 {
	var v = 0.0;
	var a = 0.5;
	var p = p0;
	for (var k = 0; k < 4; k++) {
		v += a * vnoise(p);
		p = p * 2.03 + vec2f(1.7, 9.2);
		a *= 0.5;
	}
	return v;
}
fn h22(p: vec2f) -> vec2f {
	return vec2f(h21(p), h21(p + vec2f(19.19, 7.7)));
}
// Cells: distance to the nearest centre, to the second nearest, and the cell's id.
fn cells(p: vec2f, t: f32) -> vec3f {
	let i = floor(p);
	let f = fract(p);
	var d1 = 8.0;
	var d2 = 8.0;
	var id = 0.0;
	for (var y = -1; y <= 1; y++) {
		for (var x = -1; x <= 1; x++) {
			let g = vec2f(f32(x), f32(y));
			let o = h22(i + g);
			let m = 0.5 + 0.42 * sin(t * 0.5 + 6.2831 * o);
			let r = g + m - f;
			let d = dot(r, r);
			if (d < d1) {
				d2 = d1;
				d1 = d;
				id = h21(i + g);
			} else if (d < d2) {
				d2 = d;
			}
		}
	}
	return vec3f(sqrt(d1), sqrt(d2), id);
}
// A hexagonal tiling: distance from the cell's edge (0 at the edge) and the cell's centre.
fn hexes(p: vec2f) -> vec3f {
	let s = vec2f(1.0, 1.7320508);
	let a = p - s * floor(p / s) - s * 0.5;
	let pb = p - s * 0.5;
	let b = pb - s * floor(pb / s) - s * 0.5;
	let gv = select(b, a, dot(a, a) < dot(b, b));
	let q = abs(gv);
	let d = max(dot(q, normalize(s)), q.x);
	let centre = p - gv;
	return vec3f(0.5 - d, centre);
}
fn spectrum(x: f32) -> vec3f {
	return 0.5 + 0.5 * cos(6.2831853 * (x + vec3f(0.0, 0.33, 0.67)));
}

// The world behind the lanes, per organ. It answers the pointer, and a pick
// sends a ring through it.
fn universe(w: vec2f, ndc: vec2f, t: f32) -> vec3f {
	let st = story();
	let cur = select(vec2f(99.0), tu.u4.xy, tu.u6.w > 0.5);
	let near = exp(-dot(w - cur, w - cur) * 7.0);
	let x0 = tu.u19.z;
	let x1 = tu.u19.w;
	let xn = clamp((w.x - x0) / max(x1 - x0, 0.01), 0.0, 1.0);
	var c = vec3f(0.0);
	if (st == TRACE) {
		// A day's sky: night, dawn, noon, dusk and night across the hours, with an
		// aurora drifting through it and stars that bend toward the pointer.
		let night = vec3f(0.02, 0.03, 0.10);
		let dawn = vec3f(0.22, 0.07, 0.18);
		let noon = vec3f(0.03, 0.16, 0.24);
		let dusk = vec3f(0.24, 0.10, 0.03);
		let k = xn * 4.0;
		var sky = mix(night, dawn, clamp(k, 0.0, 1.0));
		sky = mix(sky, noon, clamp(k - 1.0, 0.0, 1.0));
		sky = mix(sky, dusk, clamp(k - 2.0, 0.0, 1.0));
		sky = mix(sky, night, clamp(k - 3.0, 0.0, 1.0));
		c = sky * (0.55 + 0.45 * ndc.y);
		let n = fbm(vec2f(w.x * 0.9 + t * 0.03, w.y * 2.4 - t * 0.06));
		let band = exp(-pow(w.y - 0.42 - 0.18 * sin(w.x * 1.2 + t * 0.12), 2.0) * 5.0);
		let curtain = smoothstep(0.42, 0.78, n) * band;
		c += mix(vec3f(0.15, 0.95, 0.65), vec3f(0.55, 0.3, 1.0), fbm(w * 0.7 + t * 0.02)) * curtain * (0.32 + near * 0.6);
		let sp = floor(w * 70.0);
		let star = step(0.9965, h21(sp)) * (0.55 + 0.45 * sin(t * 2.4 + h21(sp + 3.1) * 40.0));
		c += vec3f(0.9, 0.95, 1.0) * star * (0.6 + near * 1.6);
	} else if (st == LEDGER) {
		// A golden ledger: warm parchment light, ruled hour columns, and ink that
		// drifts up the page and gathers where the pointer writes.
		let parch = fbm(w * 1.8 + vec2f(0.0, t * 0.02));
		c = mix(vec3f(0.035, 0.022, 0.008), vec3f(0.11, 0.07, 0.02), parch);
		let dcol = abs(fract(xn * 8.0 + 0.5) - 0.5) * (x1 - x0) / 8.0 / 0.003;
		c += vec3f(0.55, 0.38, 0.12) * exp(-dcol * dcol) * 0.14 * step(x0, w.x) * step(w.x, x1);
		let ink = vec2f(w.x * 14.0, w.y * 14.0 - t * 0.6);
		let mote = step(0.985, h21(floor(ink))) * smoothstep(0.5, 0.0, length(fract(ink) - 0.5));
		c += vec3f(1.0, 0.78, 0.36) * mote * (0.45 + near * 2.0);
		c += vec3f(1.0, 0.7, 0.3) * near * 0.06;
	} else if (st == GUARD) {
		// A shield: a hexagonal field shimmering violet, brightest at the gate,
		// lighting up wherever the pointer touches it.
		let hx = hexes(w * 9.0);
		let edge = exp(-hx.x * hx.x * 900.0);
		let flick = vnoise(hx.yz * 0.7 + t * 0.4);
		let gate = exp(-abs(w.y - gateY()) * 3.0);
		let touch = exp(-dot(hx.yz / 9.0 - cur, hx.yz / 9.0 - cur) * 22.0);
		c = vec3f(0.05, 0.01, 0.07);
		c += vec3f(0.62, 0.3, 1.0) * edge * (0.06 + 0.12 * flick + 0.35 * gate + 1.2 * touch);
		c += vec3f(0.85, 0.35, 1.0) * (1.0 - smoothstep(0.0, 0.5, hx.x)) * touch * 0.18;
	} else if (st == HEALING) {
		// Living tissue: cells that breathe and glow green at their walls, and part
		// around the pointer.
		let q = w * 5.5 + (w - cur) * near * 0.6;
		let cl = cells(q, t);
		let wall = exp(-pow((cl.y - cl.x) * 9.0, 2.0));
		let breath = 0.75 + 0.25 * sin(t * 1.3 + cl.z * 6.28);
		c = vec3f(0.01, 0.05, 0.035) + vec3f(0.02, 0.09, 0.06) * (1.0 - cl.x);
		c += vec3f(0.25, 1.0, 0.6) * wall * 0.22 * breath * (1.0 + near * 2.5);
		c += vec3f(0.6, 1.0, 0.8) * exp(-cl.x * cl.x * 60.0) * 0.08 * breath;
	} else if (st == CAUSAL) {
		// Fault lines: dark rock split by glowing cracks whose heat runs back from
		// now to the past, with embers rising out of them.
		let rock = fbm(w * 2.2);
		let cl = cells(w * 4.0 + vec2f(t * 0.04, 0.0), 0.0);
		let crack = exp(-pow((cl.y - cl.x) * 14.0, 2.0));
		let flow = 0.5 + 0.5 * sin(w.x * 6.0 + t * 1.6 + cl.z * 6.28);
		c = vec3f(0.04, 0.012, 0.008) * (0.6 + rock);
		c += mix(vec3f(1.0, 0.3, 0.08), vec3f(1.0, 0.75, 0.3), flow) * crack * (0.16 + 0.22 * flow + near * 1.2);
		let em = vec2f(w.x * 22.0, w.y * 22.0 - t * 1.4);
		let ember = step(0.992, h21(floor(em))) * smoothstep(0.5, 0.0, length(fract(em) - 0.5));
		c += vec3f(1.0, 0.55, 0.2) * ember * 0.8;
	} else if (st == SYSTEM) {
		// The store's heart: strata of light stacked through the dark, and rings
		// beating out from the core at about fifty-one a minute.
		let r = length(w - vec2f(0.0, -0.1));
		let beat = fract(t * 0.85);
		let ring = exp(-pow((r - beat * 2.4) / 0.05, 2.0)) * (1.0 - beat);
		let ring2 = exp(-pow((r - fract(beat + 0.5) * 2.4) / 0.04, 2.0)) * (1.0 - fract(beat + 0.5)) * 0.5;
		let strata = fbm(vec2f(w.x * 0.6, w.y * 7.0));
		c = vec3f(0.01, 0.025, 0.07) + vec3f(0.05, 0.1, 0.22) * smoothstep(0.35, 0.75, strata);
		c += vec3f(0.45, 0.72, 1.0) * (ring + ring2) * 0.45;
		c += vec3f(0.6, 0.8, 1.0) * exp(-r * r * 18.0) * (0.12 + 0.2 * heartbeat(t));
		c += vec3f(0.5, 0.75, 1.0) * near * 0.1;
	} else if (st == WITNESS) {
		// A prism: rays of every colour converging from below onto one point of
		// light above the lanes (the decision), a brightness travelling up them,
		// and the rays near the pointer catching it.
		let focus = vec2f(0.0, 1.15);
		let d = w - focus;
		let ang = atan2(d.x, -d.y);
		let k = ang * 9.0;
		let ray = exp(-pow((fract(k) - 0.5) / 0.045, 2.0));
		let rid = floor(k);
		let dist = length(d);
		let travel = exp(-pow(fract(dist * 0.6 + t * 0.25 + h21(vec2f(rid, 3.0))) - 0.5, 2.0) * 40.0);
		let tint = spectrum(h21(vec2f(rid, 1.0)) * 0.6 + ang * 0.25 + t * 0.03);
		c = vec3f(0.012, 0.03, 0.022) + tint * 0.02;
		c += tint * ray * (0.05 + 0.3 * travel + 0.45 * near) * smoothstep(2.6, 0.3, dist);
		c += vec3f(0.9, 1.0, 0.85) * exp(-dist * dist * 6.0) * 0.18;
	}
	// A pick sends a ring through the whole world.
	if (tu.u1.x >= 0.0) {
		let rr = distance(w, tu.u3.xy);
		let q = (rr - tu.u1.y * 1.6) / 0.03;
		c += tu.u11.rgb * exp(-q * q) * exp(-tu.u1.y * 1.4) * 0.5;
	}
	return c;
}
`;

const TARGETS = /* wgsl */ `
fn bez(a: vec2f, b: vec2f, t: f32, bow: f32) -> vec2f {
	let m = (a + b) * 0.5 + vec2f(-(b.y - a.y), b.x - a.x) * bow;
	return mix(mix(a, m, t), mix(m, b, t), t);
}

// Where a particle wants to be right now, and how hard it is pulled there.
fn targetOf(kind: u32, owner: u32, seed: f32, u: f32, t: f32) -> vec3f {
	if (kind == 0u) {
		if (islandCount() == 0u) {
			// Nothing to show yet: a fine dust drifting through the whole field.
			let x = (h11(seed * 3.7 + 0.1) * 2.0 - 1.0) * tu.u2.y + 0.05 * sin(t * 0.1 + seed * 30.0);
			let y = (u * 2.0 - 1.0) * 0.9 + 0.05 * cos(t * 0.08 + seed * 20.0);
			return vec3f(x, y, 2.0);
		}
		// A halo round every island: the particles orbit just outside its rim,
		// the outer ones slower, so each world turns in its own light. Causal's
		// turn backward, toward the past.
		let I = island(haloOf(seed));
		let band = h11(seed * 71.3 + 0.17);
		let off = 0.012 + 0.045 * band * band + 0.004 * sin(t * 0.7 + seed * 40.0);
		let dir = select(1.0, -1.0, story() == CAUSAL);
		let a = u * 6.2831853 + t * dir * (0.04 + 0.08 * seed) / (1.0 + band * 2.0);
		return vec3f(I.c + vec2f(sin(a) * (I.rx + off), cos(a) * (I.ry + off)), 9.0);
	}
	if (kind == 1u) {
		let m = mems[min(owner, arrayLength(&mems) - 1u)];
		let flags = u32(m.b.w + 0.5);
		let picked = (flags & 4u) != 0u;
		let hovered = f32(owner) == tu.u2.x;
		if (hovered || picked) {
			// A ring of light spins round the memory under the pointer.
			let a = seed * 6.2832 + t * (1.6 + seed);
			let rr = m.a.z * (1.45 + 0.6 * u);
			return vec3f(m.a.xy + vec2f(cos(a), sin(a)) * rr, 40.0);
		}
		// At rest the memory's light gathers into its marker.
		let a = seed * 6.2832;
		return vec3f(m.a.xy + vec2f(cos(a), sin(a)) * m.a.z * 0.55 * sqrt(u), 46.0);
	}
	let e = edges[owner];
	let causal = e.b.x > 0.5;
	let grow = ease((select(tu.u1.z, tu.u1.w, e.c.w > 0.5) - e.b.z) / 1.1);
	let flow = fract(u + t * select(0.09, 0.26, causal) + seed * 0.13);
	// A soft curve from the cause into the effect, its light running along it.
	let q = bez(e.a.xy, e.a.zw, flow * grow, e.b.w);
	return vec3f(q, 60.0);
}
`;

export const TRACE_COMPUTE_WGSL = /* wgsl */ `
${COMMON}
@group(0) @binding(0) var<uniform> ep: EP;
@group(0) @binding(1) var<uniform> tu: TU;
@group(0) @binding(2) var<storage, read> mems: array<Mem>;
@group(0) @binding(3) var<storage, read> edges: array<Edge>;
@group(0) @binding(4) var<storage, read> ps: array<P>;
@group(0) @binding(5) var<storage, read_write> state: array<S>;
@group(0) @binding(6) var<storage, read> density: array<f32>;
@group(0) @binding(7) var<storage, read> handoff: array<H>;
${ISLANDS}
${TARGETS}

// Value noise and its curl: divergence-free flow, so the field swirls
// without clumping.
fn vnoise(p: vec2f) -> f32 {
	let i = floor(p);
	let f = fract(p);
	let w = f * f * (3.0 - 2.0 * f);
	let a = h21(i);
	let b = h21(i + vec2f(1.0, 0.0));
	let c = h21(i + vec2f(0.0, 1.0));
	let d = h21(i + vec2f(1.0, 1.0));
	return mix(mix(a, b, w.x), mix(c, d, w.x), w.y);
}
fn curl(p: vec2f) -> vec2f {
	let e = 0.07;
	let dy = vnoise(p + vec2f(0.0, e)) - vnoise(p - vec2f(0.0, e));
	let dx = vnoise(p + vec2f(e, 0.0)) - vnoise(p - vec2f(e, 0.0));
	return vec2f(dy, -dx) / (2.0 * e);
}

// How each organ's particles arrive when its answer lands.
struct Arrive { p: vec2f, v: vec2f, h: f32 };
fn arrive(tg: vec2f, seed: f32, u: f32) -> Arrive {
	let halfW = tu.u2.y;
	let riverY = tu.u0.w;
	let h1 = h11(seed * 917.0 + u * 31.0);
	let h2 = h11(seed * 53.0 + u * 7.0);
	let h3 = h11(seed * 13.0 + u * 91.0);
	var a: Arrive;
	let st = story();
	if (st == LEDGER) {
		// The record writes itself: the ink waits at its first entry.
		a.p = vec2f(-halfW * 1.08 - h1 * 0.12, riverY + (h2 - 0.5) * 0.06);
		a.v = vec2f(0.1 + h3 * 0.3, (h1 - 0.5) * 0.2);
		a.h = 0.9;
		return a;
	}
	if (st == GUARD) {
		// Everything the agents tried falls toward the gate.
		a.p = vec2f(tg.x + (h1 - 0.5) * 0.25, 1.1 + h2 * 0.6);
		a.v = vec2f(0.0, -1.2 - h3 * 1.6);
		a.h = 0.35;
		return a;
	}
	if (st == HEALING) {
		// Torn apart above and below, waiting to be knit.
		let side = select(-1.0, 1.0, h1 > 0.5);
		a.p = tg + vec2f((h3 - 0.5) * 0.08, side * (0.22 + h2 * 0.38));
		a.v = vec2f(0.0, side * 0.15);
		a.h = 0.1;
		return a;
	}
	if (st == CAUSAL) {
		// Everything starts at now and walks back.
		a.p = vec2f(halfW * 1.08 + h1 * 0.12, riverY + (h2 - 0.5) * 0.08);
		a.v = vec2f(-0.1 - h3 * 0.3, (h1 - 0.5) * 0.2);
		a.h = 1.1;
		return a;
	}
	if (st == WITNESS) {
		// The witnesses come in from both sides, each along its own lane.
		let mid = (tu.u19.z + tu.u19.w) * 0.5;
		let side = select(-1.0, 1.0, tg.x > mid);
		a.p = vec2f(side * (halfW * 1.1 + h1 * 0.12), tg.y + (h2 - 0.5) * 0.004);
		a.v = vec2f(-side * (0.2 + h3 * 0.4), 0.0);
		a.h = 0.6;
		return a;
	}
	if (st == SYSTEM) {
		// The store rises like a tide.
		a.p = vec2f(tg.x + (h1 - 0.5) * 0.06, -1.1 - h2 * 0.45);
		a.v = vec2f(0.0, 0.3 + h3 * 0.5);
		a.h = 0.2;
		return a;
	}
	// Trace: the answer erupts from the question.
	let ang = h1 * 6.2832;
	let burst = vec2f(cos(ang), sin(ang));
	let toward = normalize(tg - tu.u3.zw + vec2f(1e-4, 0.0));
	a.p = tu.u3.zw + burst * 0.015 * h11(seed * 3.1 + u);
	a.v = normalize(mix(burst, toward, 0.45)) * (0.5 + 2.9 * pow(h2, 1.6));
	a.h = 1.2;
	return a;
}

// When each particle is let go toward its place, per organ.
fn arriveRelease(tx: f32, seed: f32, age: f32) -> f32 {
	let halfW = max(tu.u2.y, 0.01);
	let xn = clamp((tx + halfW) / (2.0 * halfW), 0.0, 1.0);
	let st = story();
	if (st == LEDGER) { return ease((age - 0.1 - xn * 2.4 - seed * 0.15) / 0.45); }
	if (st == GUARD) { return ease((age - 0.05 - seed * 0.35) / 0.6); }
	if (st == HEALING) { return ease((age - 0.5 - xn * 1.7 - seed * 0.2) / 0.8); }
	if (st == CAUSAL) { return ease((age - 0.1 - (1.0 - xn) * 2.2 - seed * 0.15) / 0.5); }
	if (st == SYSTEM) { return ease((age - 0.15 - seed * 1.4) / 1.2); }
	// Witness: the outermost land first and the testimony closes in on the middle.
	if (st == WITNESS) { return ease((age - 0.15 - (1.0 - abs(xn - 0.5) * 2.0) * 1.6 - seed * 0.15) / 0.5); }
	return ease((age - 0.2 - seed * 0.55) / 0.9);
}

@compute @workgroup_size(256)
fn cs(@builtin(global_invocation_id) gid: vec3u) {
	let i = gid.x;
	if (i >= arrayLength(&ps)) { return; }
	let st = ps[i].v;
	let kind = u32(st.x + 0.5);
	let owner = select(u32(st.y + 0.5), galaxyOwner(u32(st.y + 0.5), tu.u7.z), kind == 1u);
	let seed = st.z;
	let u = st.w;
	let t = ep.time;
	let dt = tu.u2.w;
	let halfW = tu.u2.y;

	var s = state[i];
	var pos = s.pv.xy;
	var vel = s.pv.zw;
	var heat = s.m.z;

	var live = true;
	var isWalk = false;
	if (kind == 1u) {
		live = f32(owner) < tu.u0.y;
		let flags = u32(mems[min(owner, arrayLength(&mems) - 1u)].b.w + 0.5);
		live = live && flags != 0u;
		isWalk = (flags & 2u) != 0u;
	} else if (kind == 2u) {
		live = f32(owner) < tu.u0.z;
		isWalk = edges[owner].c.w < 0.5;
	}
	let tg = targetOf(kind, owner, seed, u, t);

	// A new answer: every particle arrives the way this organ tells it, or,
	// straight after a Palace click, starts on the very pixel it was in the
	// Palace's last frame (or simply stays where it is).
	let mode = u32(tu.u9.y + 0.5);
	let nHand = u32(tu.u9.w + 0.5);
	if (s.m.x != tu.u6.x) {
		s.m.x = tu.u6.x;
		if (mode == 1u && nHand > 0u) {
			let j = (i * 7919u + 13u) % nHand;
			let hp = handoff[j];
			pos = hp.a.xy + vec2f(h11(seed * 31.0 + u) - 0.5, h11(seed * 17.0 + u * 3.0) - 0.5) * 0.004;
			// The second explosion: the Palace's light goes off again on this page,
			// out from the middle and turning, hot.
			let r0 = length(pos) + 1e-4;
			let out0 = pos / r0;
			let sp = 0.9 + 2.6 * h11(seed * 53.0 + u * 7.0);
			vel = out0 * sp + vec2f(-out0.y, out0.x) * (0.4 + 1.2 * h11(seed * 91.0 + u));
			heat = 0.8 + 0.9 * h11(seed * 13.0 + u * 5.0);
			s.m.w = f32(j) + 1.0;
		} else if (mode != 2u) {
			let a = arrive(tg.xy, seed, u);
			pos = a.p;
			vel = a.v;
			heat = a.h;
			s.m.w = 0.0;
		}
	}
	// Holding the Palace's picture while the organ's data arrives: the
	// particles shimmer in place, they do not move yet.
	let holding = tu.u9.z > 0.5 && s.m.w > 0.5;
	// A new walk: the causes unfold out of the picked memory.
	if (s.m.y != tu.u6.y) {
		s.m.y = tu.u6.y;
		if (isWalk) {
			let a = h11(seed * 413.0 + u * 17.0) * 6.2832;
			let toward = normalize(tg.xy - tu.u3.xy + vec2f(0.0, 1e-4));
			let sp = 0.6 + 2.2 * h11(seed * 29.0 + u * 5.0);
			pos = tu.u3.xy + vec2f(cos(a), sin(a)) * 0.01;
			vel = normalize(mix(vec2f(cos(a), sin(a)), toward, 0.6)) * sp;
			heat = 1.4;
		}
	}

	if (holding && dt > 0.0) {
		// Until the page's own layout takes it, the light keeps exploding: it
		// flies out, slows, and swirls round the middle, never still.
		let r = length(pos) + 1e-4;
		let o = pos / r;
		vel = vel * exp(-dt * 1.1) + (o * 0.3 + vec2f(-o.y, o.x) * (0.8 + 0.8 * seed)) * dt;
		pos += vel * dt;
		heat = heat * exp(-dt * 0.7);
	}
	if (live && !holding && dt > 0.0 && ep.capture_mode < 0.5) {
		let relAge = select(tu.u1.w, tu.u1.z, isWalk);
		let release = select(arriveRelease(tg.x, seed, relAge), ease((relAge - 0.2 - seed * 0.55) / 0.9), isWalk);
		let y0 = pos.y;
		let k = tg.z * release;
		let c = 2.0 * sqrt(max(k, 1e-4)) * 0.72 + 0.45;
		var acc = (tg.xy - pos) * k - vel * c;

		// Swirl only while a particle is still arriving; a settled field is
		// straight, whatever heats it later (the write-head, a pick).
		let amp = select(0.0, min(heat, 1.0) * 0.8, release < 0.98);
		if (amp > 0.01) { acc += curl(pos * 2.4 + vec2f(t * 0.06, -t * 0.045)) * amp; }

		// The pick: a ring of light runs out through the field from the memory.
		// It lights what it passes and moves nothing, so every line stays straight.
		if (tu.u1.x >= 0.0) {
			let age = tu.u1.y;
			let r = length(pos - tu.u3.xy);
			let q = (r - (0.04 + age * 1.25)) / 0.06;
			heat += exp(-q * q) * exp(-age * 1.5) * dt * 9.0;
		}
		// The receipt: a hot front keeps rolling out from the memory while
		// its chain is re-verified...
		let vAge = tu.u5.x;
		if (vAge >= 0.0 && kind == 0u) {
			let r = length(pos - tu.u3.xy);
			let q = (r - fract(vAge / 1.15) * 0.7) / 0.04;
			heat += exp(-q * q) * dt * 8.0;
		}
		// ...and a second, tighter wave seals it when the chain checks out.
		let sAge = tu.u5.y;
		if (sAge >= 0.0) {
			let r = length(pos - tu.u3.xy);
			let q = (r - (0.02 + sAge * 0.75)) / 0.035;
			heat += exp(-q * q) * exp(-sAge * 2.0) * dt * 10.0;
		}
		// The cursor drags a wake through the field, the field turns around
		// it like water around a finger, and fast strokes heat what they touch.
		if (tu.u6.w > 0.5 && kind == 0u) {
			let d = pos - tu.u4.xy;
			let r2 = dot(d, d);
			let f = exp(-r2 / 0.005) * 0.3;
			let spd = min(tu.u8.z, 2.5);
			// A gentle wake: the field parts for the pointer and follows its
			// stroke a little. No vortex, little heat: it should never compete
			// with the memories themselves.
			acc += (tu.u4.zw * 1.6 + d * inverseSqrt(r2 + 1e-5) * 0.5) * f * 4.0;
			heat += f * dt * 0.5 * spd;
		}

		// Ledger: each clock's hand goes round and the halo it passes glows.
		if (story() == LEDGER && kind == 0u && islandCount() > 0u) {
			let I = island(haloOf(seed));
			let d = pos - I.c;
			let behind = fract((handAngle(t) - atan2(d.x, d.y)) / 6.2831853);
			heat += exp(-behind * 40.0) * dt * 4.0;
		}

		heat *= exp(-dt * 1.25);
		vel += acc * dt;
		let spd = length(vel);
		if (spd > 5.0) { vel *= 5.0 / spd; }
		pos += vel * dt;

		// Guard: whatever crosses the gate going down flashes as it passes.
		if (story() == GUARD && kind != 2u && y0 > gateY() && pos.y <= gateY()) {
			heat += 0.9;
		}
	}
	state[i] = S(vec4f(pos, vel), vec4f(s.m.x, s.m.y, clamp(heat, 0.0, 3.0), s.m.w));
}
`;

export const TRACE_PARTICLES_WGSL = /* wgsl */ `
${COMMON}
@group(0) @binding(0) var<uniform> ep: EP;
@group(0) @binding(1) var<uniform> tu: TU;
@group(0) @binding(2) var<storage, read> mems: array<Mem>;
@group(0) @binding(3) var<storage, read> edges: array<Edge>;
@group(0) @binding(4) var<storage, read> ps: array<P>;
@group(0) @binding(5) var<storage, read> state: array<S>;
@group(0) @binding(6) var<storage, read> density: array<f32>;
@group(0) @binding(7) var<storage, read> handoff: array<H>;
${ISLANDS}

const QUAD = array<vec2f, 6>(
	vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
	vec2f(-1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, 1.0)
);

struct VO {
	@builtin(position) clip: vec4f,
	@location(0) uv: vec2f,
	@location(1) col: vec3f,
};

fn hidden() -> VO {
	var o: VO;
	o.clip = vec4f(3.0, 3.0, 0.0, 1.0);
	return o;
}

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VO {
	let st = ps[ii].v;
	let kind = u32(st.x + 0.5);
	let owner = select(u32(st.y + 0.5), galaxyOwner(u32(st.y + 0.5), tu.u7.z), kind == 1u);
	let seed = st.z;
	let u = st.w;
	let s = state[ii];
	let pos = s.pv.xy;
	let vel = s.pv.zw;
	let heat = s.m.z;
	let focus = tu.u6.z > 0.5;
	var col = vec3f(0.0);
	var alpha = 0.0;
	var size = 0.0;

	// A particle that came from the Palace: while the picture holds it is
	// drawn exactly as it was there; once released it keeps the Palace's
	// colour for a moment and turns into its own.
	let handIdx = s.m.w;
	var handCol = vec3f(0.0);
	var handMix = 0.0;
	if (handIdx > 0.5) {
		let hp = handoff[u32(handIdx - 1.0)];
		handCol = hp.b.rgb * (0.9 + 1.4 * hp.a.z);
		// The second explosion runs through every colour as it flies.
		handCol = mix(handCol, rainbow(length(pos) * 0.8 - ep.time * 1.3 + seed * 0.3) * 1.6, 0.4);
		handMix = select(1.0 - ease(tu.u1.w / 1.6), 1.0, tu.u9.z > 0.5);
		let gone = (kind == 1u && f32(owner) >= tu.u0.y) || (kind == 2u && f32(owner) >= tu.u0.z);
		if (tu.u9.z > 0.5 || (gone && handMix > 0.0)) {
			// Holding, or a particle the organ has no place for: it stays the
			// Palace's light and fades out as the organ takes shape.
			let q = QUAD[vi];
			let sz = 0.0034;
			// Streaked along its flight, like the Palace's burst.
			let hs = length(vel);
			let hd = select(vec2f(1.0, 0.0), vel / max(hs, 1e-5), hs > 1e-4);
			let hst = 1.0 + min(hs * 0.04 / sz, 9.0);
			var o: VO;
			o.clip = toClip(pos + hd * q.x * sz * hst + vec2f(-hd.y, hd.x) * q.y * sz);
			o.uv = q;
			// Bright while it explodes, burning hot where it is fastest.
			o.col = (handCol * 2.4 + tu.u12.rgb * min(hs * 0.9, 1.6)) * select(handMix, 1.0, tu.u9.z > 0.5) / pow(hst, 0.3);
			if (handMix <= 0.003 && tu.u9.z < 0.5) { return hidden(); }
			return o;
		}
	}

	if (kind == 0u) {
		// The halo wears its island's colour, brighter while the island is lit.
		var hue = tu.u11.rgb;
		var lit = 0.0;
		if (islandCount() > 0u) {
			let I = island(haloOf(seed));
			hue = mix(I.col, tu.u11.rgb, 0.35);
			lit = I.lit;
		}
		col = mix(tu.u10.rgb, hue, 0.55 + 0.45 * u);
		alpha = (0.11 + 0.16 * lit) * select(1.0, 0.55, focus);
		if (story() == SYSTEM) { alpha *= 0.8 + 0.5 * heartbeat(ep.time); }
		size = 0.0021;
	} else if (kind == 1u) {
		if (f32(owner) >= tu.u0.y) { return hidden(); }
		let m = mems[owner];
		let flags = u32(m.b.w + 0.5);
		if (flags == 0u) { return hidden(); }
		let picked = (flags & 4u) != 0u;
		let isWalk = (flags & 2u) != 0u;
		let hovered = f32(owner) == tu.u2.x;
		let isCause = (flags & 8u) != 0u;
		var bright = 0.4 + 0.6 * m.a.w;
		if (focus && !picked && !isWalk && !isCause) { bright *= 0.4; }
		if (isCause) { bright *= 1.2 + 1.2 * ease(tu.u1.z / 0.8) * exp(-max(tu.u1.z - 0.8, 0.0) * 0.6) + 0.35; }
		if (hovered) { bright *= 1.8; }
		if (tu.u6.w > 0.5) {
			let dc = m.a.xy - tu.u4.xy;
			bright *= 1.0 + 0.3 * exp(-dot(dc, dc) / 0.008);
		}
		if (picked) { bright *= 1.5 + 2.2 * exp(-tu.u1.y * 1.3); }
		if ((flags & 16u) != 0u && !picked) { bright *= 0.42 + 0.22 * sin(ep.time * 1.3 + f32(owner)); }
		// More particles per memory, each one dimmer: the total light stays even.
		let share = clamp(sqrt(tu.u7.w / max(tu.u7.z, 1.0)), 0.3, 1.6);
		col = m.b.rgb * bright * share;
		// The arrival brings each memory in as light; once it has landed the
		// marker takes over and the light fades, unless the memory is hovered or
		// picked (then it is the beam to the time axis).
		let landed = select(tu.u1.w - landAt(m.a.x), tu.u1.z - 1.2, isWalk);
		let rest = 1.0 - ease((landed - 0.1) / 0.6);
		alpha = select(rest, 0.85, hovered || picked) * select(1.0, ease(tu.u1.z / 0.5), isWalk);
		size = select(0.0026, 0.0022, hovered || picked);
	} else {
		if (f32(owner) >= tu.u0.z) { return hidden(); }
		let e = edges[owner];
		let causal = e.b.x > 0.5;
		let grow = ease((select(tu.u1.z, tu.u1.w, e.c.w > 0.5) - e.b.z) / 1.1);
		let flow = fract(u + ep.time * select(0.09, 0.26, causal) + seed * 0.13);
		let head = smoothstep(0.6, 1.0, flow);
		let tint = e.c.rgb;
		col = select(tint * 0.4, mix(tint, mix(tint, vec3f(1.0), 0.65), head) * 1.1, causal);
		alpha = grow * select(0.5, 1.0, causal) * (0.35 + 0.65 * e.b.y);
		size = select(0.0021, 0.0033, causal);
	}

	// Heat: freshly struck particles burn white-gold, then cool to their colour.
	let h = clamp(heat, 0.0, 1.6);
	col += tu.u12.rgb * h * 1.5;
	if (handMix > 0.0) {
		col = mix(col, handCol, handMix);
		alpha = max(alpha, handMix);
	}

	// Stretch each sprite along its velocity (motion blur for free), keeping
	// its energy roughly constant so fast streaks don't blow out.
	let spd = length(vel);
	let dir = select(vec2f(1.0, 0.0), vel / max(spd, 1e-5), spd > 1e-4);
	let stretch = 1.0 + min(spd * 0.04 / size, 11.0);
	let q = QUAD[vi];
	let w = pos + dir * q.x * size * stretch + vec2f(-dir.y, dir.x) * q.y * size;

	var o: VO;
	o.clip = toClip(w);
	if (alpha <= 0.003) { return hidden(); }
	o.uv = q;
	// Behind the page's words the river whispers and everything else dims.
	let qz = quietAt(o.clip.xy);
	o.col = col * alpha / sqrt(stretch) * mix(1.0, select(0.32, 0.14, kind == 0u), qz);
	return o;
}

@fragment
fn fs(i: VO) -> @location(0) vec4f {
	let d = length(i.uv);
	let rn = clamp(d / 0.94, 0.0, 1.0);
	let disc = 1.0 - smoothstep(0.0, 1.0, rn);
	let core = (1.0 - rn) * (1.0 - rn) * (1.0 - rn);
	let halo = (1.0 - rn) * 0.4;
	return vec4f(i.col * (core + halo) * disc, 1.0);
}
`;

export const TRACE_BACKDROP_WGSL = /* wgsl */ `
${COMMON}
@group(0) @binding(0) var<uniform> ep: EP;
@group(0) @binding(1) var<uniform> tu: TU;
@group(0) @binding(2) var<storage, read> density: array<f32>;
${ISLANDS}
${UNIVERSE}

struct VO { @builtin(position) clip: vec4f };
const TRI = array<vec2f, 3>(vec2f(-1.0, -3.0), vec2f(3.0, 1.0), vec2f(-1.0, 1.0));

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VO {
	var o: VO;
	o.clip = vec4f(TRI[vi], 0.0, 1.0);
	return o;
}

// A hexagon (Inigo Quilez), for Guard's shields.
fn sdHex(p0: vec2f, r: f32) -> f32 {
	let k = vec3f(-0.866025404, 0.5, 0.577350269);
	var p = abs(p0.yx);
	p -= 2.0 * min(dot(k.xy, p), 0.0) * k.xy;
	p -= vec2f(clamp(p.x, -k.z * r, k.z * r), r);
	return length(p) * sign(p.y);
}

// Every island over the organ's world: its inside quieter so each orb on it
// reads, its rim lit in its colour with a glow round it, brighter while the
// pointer is on it. Each organ draws its islands its own way.
fn islandsOver(base: vec3f, w: vec2f, t: f32, px: f32) -> vec3f {
	var c = base;
	let n = islandCount();
	let st = story();
	let opened = tu.u18.z > 0.5;
	let marks = tu.u18.w;
	for (var k = 0u; k < min(n, 24u); k++) {
		let I = island(k);
		let rad = min(I.rx, I.ry);
		let rel = w - I.c;
		let q = rel / vec2f(I.rx, I.ry);
		let rq = length(q);
		// 0 at the top, turning clockwise.
		let ang = atan2(q.x, q.y);
		var d = (rq - 1.0) * rad;
		if (d > rad * 0.9 + 0.12) { continue; }
		if (st == GUARD && !opened) {
			// A shield.
			d = sdHex(rel, rad * 0.98);
		} else if (st == HEALING) {
			// A living cell: its wall breathes and ripples.
			d -= rad * 0.03 * sin(ang * 7.0 + t * 1.1 + f32(k) * 1.7) * select(1.0, 0.35, opened);
		}
		let lit = I.lit;
		let inside = 1.0 - smoothstep(-px * 1.5, px * 1.5, d);
		var rimK = 0.55 + 0.85 * lit;
		if (st == CAUSAL) { rimK *= 0.55 + 1.0 * vnoise(vec2f(ang * 2.5 + f32(k) * 7.0, t * 1.4)); }
		if (st == SYSTEM) { rimK *= 0.75 + 0.9 * heartbeat(t + f32(k) * 0.13); }
		c = mix(c, c * 0.38 + I.col * (0.03 + 0.035 * lit), inside);
		let rim = exp(-pow(d / (px * 2.0), 2.0));
		let outGlow = exp(-max(d, 0.0) / (0.02 + 0.12 * rad)) * (1.0 - inside);
		let inGlow = exp(-max(-d, 0.0) / (0.02 + 0.1 * rad)) * inside;
		c += I.col * (rim * rimK + outGlow * (0.1 + 0.18 * lit) + inGlow * (0.06 + 0.06 * lit));
		if (st == TRACE) {
			// A sun: a corona breathing round it.
			c += I.col * exp(-max(d, 0.0) / (0.07 * rad + 0.01)) * (1.0 - inside) * (0.05 + 0.035 * sin(t * 1.3 + f32(k)));
		} else if (st == WITNESS) {
			// A lens: a second, fainter rim just inside the first.
			let d2 = d + rad * 0.09;
			c += I.col * exp(-pow(d2 / (px * 1.6), 2.0)) * 0.24;
		} else if (st == SYSTEM) {
			// A beat leaves the rim as a ring.
			let b = fract(t * 0.85 + f32(k) * 0.13);
			let dr = (d - b * rad * 0.5) / (px * 2.5);
			c += I.col * exp(-dr * dr) * (1.0 - b) * 0.5 * (1.0 - inside);
		}
		if (marks > 0.5) {
			// A clock: a dot at every hour (or day), the quarters bigger, and a
			// soft sweep behind the hand that keeps going round.
			let hk = round(ang / 6.2831853 * marks);
			let ha = hk / marks * 6.2831853;
			let hp = I.c + vec2f(sin(ha), cos(ha)) * (rad - 0.62 * tu.u19.x);
			let big = marks > 12.0 && (i32(hk) % 6) == 0;
			let dotR = px * select(1.8, 3.2, big);
			c += I.col * smoothstep(dotR + px, dotR - px, distance(w, hp)) * select(0.5, 1.0, big);
			let behind = fract((handAngle(t) - ang) / 6.2831853);
			c += I.col * exp(-behind * 14.0) * inside * 0.12;
		}
	}
	return c;
}

@fragment
fn fs(i: VO) -> @location(0) vec4f {
	let A = tu.u0.x;
	let res = vec2f(max(ep.viewport_w, 1.0), max(ep.viewport_h, 1.0));
	let ndc = vec2f(i.clip.x / res.x * 2.0 - 1.0, 1.0 - i.clip.y / res.y * 2.0);
	let zoom = max(tu.u13.z, 0.1);
	let w = vec2f(ndc.x * A, ndc.y) / zoom + tu.u13.xy;
	let t = ep.time;
	// One pixel in world units, so every rim is crisp at any zoom.
	let px = 2.0 / res.y / zoom;
	// The organ's own world first, then its islands over it.
	var c = islandsOver(universe(w, ndc, t), w, t, px);

	// The receipt check: a light runs round the picked memory while its chain is re-verified.
	let vAge = tu.u5.x;
	if (vAge >= 0.0) {
		let rel = w - tu.u3.xy;
		let ring = exp(-pow((length(rel) - tu.u19.x * 2.3) / (px * 2.0), 2.0));
		let head = fract((fract(vAge * 0.9) * 6.2831853 - atan2(rel.x, rel.y)) / 6.2831853);
		c += vec3f(1.0, 0.72, 0.36) * ring * (0.25 + 1.4 * exp(-head * 6.0));
	}

	// The cursor carries a faint light.
	if (tu.u6.w > 0.5) {
		let dc = distance(w, tu.u4.xy);
		c += tu.u11.rgb * exp(-dc * dc * 90.0) * 0.02;
	}

	// The seal: a ring from the picked memory when its chain checks out.
	let sAge = tu.u5.y;
	if (sAge >= 0.0) {
		let r = distance(w, tu.u3.xy);
		let q = (r - (0.02 + sAge * 0.75)) / 0.01;
		let ok = tu.u5.z > 0.5;
		let tint = select(vec3f(1.0, 0.35, 0.3), vec3f(1.0, 0.85, 0.45), ok);
		c += tint * exp(-q * q) * exp(-sAge * 1.8) * 0.5;
	}
	// Behind the page's words the field goes quiet.
	c *= mix(1.0, 0.3, quietAt(ndc));
	return vec4f(c, 1.0);
}
`;

export const TRACE_MARKERS_WGSL = /* wgsl */ `
${COMMON}
@group(0) @binding(0) var<uniform> ep: EP;
@group(0) @binding(1) var<uniform> tu: TU;
@group(0) @binding(2) var<storage, read> mems: array<Mem>;

const QUAD = array<vec2f, 6>(
	vec2f(-1.0, -1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
	vec2f(-1.0, -1.0), vec2f(1.0, 1.0), vec2f(-1.0, 1.0)
);

struct VO {
	@builtin(position) clip: vec4f,
	// Position inside the marker, in pixels from its centre.
	@location(0) p: vec2f,
	// radius (px), shape, hover (0..1), picked (0/1)
	@location(1) @interpolate(flat) info: vec4f,
	// type rgb, retention
	@location(2) @interpolate(flat) col: vec4f,
	// visibility, device pixel ratio, press (0..1), dim
	@location(3) @interpolate(flat) st: vec4f,
};

fn hidden() -> VO {
	var o: VO;
	o.clip = vec4f(3.0, 3.0, 0.0, 1.0);
	return o;
}

// Distance functions (Inigo Quilez), in pixels.
fn sdSegment(p: vec2f, a: vec2f, b: vec2f) -> f32 {
	let pa = p - a;
	let ba = b - a;
	let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
	return length(pa - ba * h);
}
fn sdHexagon(q: vec2f, r: f32) -> f32 {
	let k = vec3f(-0.866025404, 0.5, 0.577350269);
	var p = abs(q);
	p -= 2.0 * min(dot(k.xy, p), 0.0) * k.xy;
	p -= vec2f(clamp(p.x, -k.z * r, k.z * r), r);
	return length(p) * sign(p.y);
}
fn sdRoundedX(q: vec2f, w: f32, r: f32) -> f32 {
	let p = abs(q);
	return length(p - min(p.x + p.y, w) * 0.5) - r;
}
fn sdTriangle(p0: vec2f, r: f32) -> f32 {
	let k = 1.7320508;
	var p = vec2f(abs(p0.x) - r, p0.y + r / k);
	if (p.x + k * p.y > 0.0) { p = vec2f(p.x - k * p.y, -k * p.x - p.y) / 2.0; }
	p.x -= clamp(p.x, -2.0 * r, 0.0);
	return -length(p) * sign(p.y);
}
fn shapeDist(p: vec2f, r: f32, shape: u32) -> f32 {
	switch shape {
		case 1u: { return (abs(p.x) + abs(p.y) - r * 1.25) * 0.7071; }
		case 2u: { return sdRoundedX(p, r * 1.7, r * 0.34); }
		case 3u: { return sdTriangle(p + vec2f(0.0, r * 0.12), r * 1.12) - r * 0.12; }
		case 4u: { return sdHexagon(p, r * 0.9); }
		case 5u: {
			let q = abs(p) - vec2f(r * 0.7);
			return length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0) - r * 0.2;
		}
		case 6u: { return abs(length(p) - r * 0.78) - r * 0.24; }
		case 7u: { return sdSegment(p, vec2f(0.0, -r * 1.7), vec2f(0.0, r * 1.7)) - r * 0.78; }
		default: { return length(p) - r; }
	}
}

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VO {
	if (f32(ii) >= tu.u0.y) { return hidden(); }
	let m = mems[ii];
	let flags = u32(m.b.w + 0.5);
	if ((flags & 3u) == 0u) { return hidden(); }
	let shape = (flags >> 8u) & 15u;
	let isWalk = (flags & 2u) != 0u;
	let picked = (flags & 4u) != 0u;
	let hovered = f32(ii) == tu.u2.x;
	let dpr = max(tu.u19.y, 1.0);
	let zoom = max(tu.u13.z, 0.1);
	// The marker's radius in pixels, whatever the camera does.
	let rpx = m.a.z * zoom * ep.viewport_h * 0.5;
	// Hover: a critically damped grow over about 0.18 s; picked holds large.
	let hv = select(0.0, ease(tu.u2.z / 0.18), hovered);
	let pk = select(0.0, 1.0, picked);
	let grow = 1.0 + 0.35 * max(hv, pk * 0.85);
	let r = rpx * grow;
	// Room for the hover ring and the glow round the shape.
	let pad = 22.0 * dpr;
	let tall = select(1.0, 2.2, shape == 7u);
	let half = vec2f(r + pad, r * tall + pad);
	let c = toClip(m.a.xy);
	let q = QUAD[vi];
	var o: VO;
	o.clip = vec4f(c.xy + q * half * 2.0 / vec2f(max(ep.viewport_w, 1.0), max(ep.viewport_h, 1.0)), 0.0, 1.0);
	o.p = q * half;
	o.info = vec4f(r, f32(shape), hv, pk);
	o.col = vec4f(m.b.rgb, m.a.w);
	// Each marker lights up as its arriving particles land.
	let appear = select(ease((tu.u1.w - landAt(m.a.x) + 0.2) / 0.4), ease((tu.u1.z - 0.9) / 0.4), isWalk);
	// A pick presses the marker for a moment.
	let press = select(0.0, exp(-tu.u1.y * 6.0) * step(0.0, tu.u1.y), picked);
	// While something is hovered or picked the rest step back.
	let others = (tu.u2.x >= 0.0 || tu.u6.z > 0.5) && !hovered && !picked;
	let dim = select(1.0, 0.55, others) * mix(1.0, 0.07, quietAt(c.xy));
	o.st = vec4f(appear, dpr, press, dim);
	if (appear <= 0.002) { return hidden(); }
	return o;
}

@fragment
fn fs(i: VO) -> @location(0) vec4f {
	let r = i.info.x;
	let shape = u32(i.info.y + 0.5);
	let hv = i.info.z;
	let pk = i.info.w;
	let dpr = i.st.y;
	let press = i.st.z;
	// A press squeezes the shape a little.
	let d = shapeDist(i.p / (1.0 - 0.12 * press), r, shape);
	// Exact one-pixel anti-aliased edges, measured in pixels.
	let fill = clamp(0.5 - d, 0.0, 1.0);
	let inner = clamp(0.5 - (d + 1.3 * dpr), 0.0, 1.0);
	let tc = i.col.rgb;
	let ret = clamp(i.col.a, 0.0, 1.0);
	// Retention is how bright the memory is held, never its size. The orb
	// lights from its heart: near white in the middle, its colour at the edge.
	let body = tc * (0.55 + 0.45 * ret);
	let rn = clamp(length(i.p) / max(r * 1.2, 1.0), 0.0, 1.0);
	let heart = mix(mix(body, vec3f(1.0), 0.45) * 1.1, body * 0.8, rn * rn);
	var rgb = mix(body * 0.35, heart, inner) * fill;
	// A bright edge just inside the shape.
	let edge = exp(-pow((d + 1.6 * dpr) / (1.1 * dpr), 2.0)) * fill;
	rgb += mix(tc, vec3f(1.0), 0.5) * edge * 0.45;
	if (shape == 7u) {
		// A pill holds many memories: drawn as a stack, one rung per few pixels.
		let rung = abs(fract(i.p.y / (3.0 * dpr)) - 0.5) * 2.0;
		rgb *= mix(0.45, 1.0, smoothstep(0.35, 0.65, rung));
	}
	var a = fill;
	// A faint ring at rest says "this can be clicked".
	let ringD = abs(d - 3.5 * dpr) - 0.5 * dpr;
	let ring = clamp(0.5 - ringD, 0.0, 1.0) * (1.0 - fill);
	rgb += vec3f(0.82, 0.95, 0.95) * ring * 0.16;
	a = max(a, ring * 0.16);
	// Hover and pick: a bright ring outside the shape, and its glow.
	let lit = max(hv, pk);
	let hrD = abs(d - 5.5 * dpr) - 1.0 * dpr;
	let hring = clamp(0.5 - hrD, 0.0, 1.0) * lit;
	rgb += mix(tc, vec3f(1.0), 0.45) * hring;
	a = max(a, hring);
	// Every orb glows, more under the pointer.
	let glow = exp(-max(d, 0.0) / (6.0 * dpr)) * (1.0 - fill) * (0.3 + 0.45 * lit);
	rgb += tc * glow;
	// The press ripple.
	let rip = abs(d - (4.0 + 26.0 * (1.0 - press)) * dpr) - 1.0 * dpr;
	rgb += vec3f(1.0, 0.95, 0.85) * clamp(0.5 - rip, 0.0, 1.0) * press * 0.8;
	let k = i.st.x * i.st.w;
	return vec4f(rgb * k, a * k);
}
`;
