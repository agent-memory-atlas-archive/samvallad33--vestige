/**
 * One memory, inside — WGSL.
 *
 * About 128k compute-simulated particles, every one of them standing for a
 * real property of the one memory on the page:
 *
 *   form     — the memory's own shape: a 3D superformula surface whose family
 *              comes from its type (a decision is crystalline, an event a
 *              burst, a pattern a many-lobed bloom) and whose exact form comes
 *              from a hash of its id, so no two memories look alike and a
 *              likeness between two shapes means nothing. Retention is how
 *              tightly the form holds: a fading memory lets particles drift
 *              off as dust. Stability sets the breath.
 *   ring     — the receipt. While the hash-chained log is re-verified a front
 *              runs around it; when the chain checks out it seals in light; a
 *              broken chain leaves it red with a gap.
 *   streams  — one per recorded edge, flowing out to (or in from) the
 *              neighbour it names, in the edge's colour; causal edges run
 *              faster and brighter than a shared handle.
 *   moons    — the neighbours themselves, small spheres in their own type
 *              colour; each one is a way to walk to that memory.
 *   dust     — a faint orbiting haze so the space around the memory breathes.
 *
 * A new arrival (a memory opened from a timeline, a neighbour walked to, or a
 * page opened cold) is a new generation: every particle starts where the
 * memory comes from (the picture handed over by the page that was clicked, a
 * moon's position, or the centre) and is released into the form on its own
 * beat, so the memory assembles in front of you.
 */

const COMMON = /* wgsl */ `
struct EP {
	frame: f32, loop_phase: f32, node_count: f32, edge_count: f32,
	path_count: f32, pulse: f32, viewport_w: f32, viewport_h: f32,
	brightness: f32, demo_id: f32, time: f32, capture_mode: f32,
	live_kind: f32, live_frame: f32, live_energy: f32, projection_days: f32,
};
// u0: aspect, age of this arrival (s), dt (s), time (s)
// u1: centre xy (world), form radius, orbit radius
// u2: type rgb, retention (0..1)
// u3: superformula m, n1, n2, n3 (around)
// u4: superformula m, n1, n2, n3 (pole to pole)
// u5: twist, normaliser, breath period (s), loose share (0..1)
// u6: cursor xy (world), cursor active, hovered (-1 none, 0.. a moon, 99 the form)
// u7: proof state (0 idle, 1 checking, 2 sealed, 3 broken), proof age (s), ring radius, 0
// u8: moon count, generation, arrival (0 birth, 1 handed-over picture, 2 from a point), handed-over count
// u9: arrival point xy (world), yaw, pitch
// u10: camera centre xy, zoom, 0
struct MU {
	u0: vec4f, u1: vec4f, u2: vec4f, u3: vec4f, u4: vec4f, u5: vec4f,
	u6: vec4f, u7: vec4f, u8: vec4f, u9: vec4f, u10: vec4f,
};
// a: x, y, size, outgoing (1) or incoming (0)
// b: moon rgb, causal (1) or shared handle (0)
// c: edge rgb, appears at (arrival age, s)
struct Moon { a: vec4f, b: vec4f, c: vec4f };
// pv: position xy, velocity xy   m: generation, handed-over index + 1, heat, depth
struct S { pv: vec4f, m: vec4f };
struct H { a: vec4f, b: vec4f };

const FORM_END: u32 = 72000u;
const RING_END: u32 = 84000u;
const PER_STREAM: u32 = 500u;
const STREAM_END: u32 = 96000u;
const PER_MOON: u32 = 360u;
const MOON_END: u32 = 104640u;
const TOTAL: u32 = 128000u;
const TAU: f32 = 6.28318530718;
const PI: f32 = 3.14159265359;

fn hash1(n: u32) -> f32 {
	var x = n * 747796405u + 2891336453u;
	x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
	x = (x >> 22u) ^ x;
	return f32(x) / 4294967295.0;
}
fn ease(t: f32) -> f32 {
	let c = clamp(t, 0.0, 1.0);
	return c * c * (3.0 - 2.0 * c);
}
fn toClip(w: vec2f) -> vec4f {
	let z = max(mu.u10.z, 0.1);
	return vec4f((w.x - mu.u10.x) * z / mu.u0.x, (w.y - mu.u10.y) * z, 0.0, 1.0);
}
fn sf(a: f32, m: f32, n1: f32, n2: f32, n3: f32) -> f32 {
	let t1 = pow(abs(cos(m * a * 0.25)), n2);
	let t2 = pow(abs(sin(m * a * 0.25)), n3);
	return pow(max(t1 + t2, 1e-5), -1.0 / n1);
}
fn rotY(p: vec3f, a: f32) -> vec3f {
	let c = cos(a);
	let s = sin(a);
	return vec3f(c * p.x + s * p.z, p.y, -s * p.x + c * p.z);
}
fn rotX(p: vec3f, a: f32) -> vec3f {
	let c = cos(a);
	let s = sin(a);
	return vec3f(p.x, c * p.y - s * p.z, s * p.y + c * p.z);
}
// The memory's own form: one point of its superformula surface (unit scale).
fn formPoint(i: u32) -> vec3f {
	var th = (hash1(i * 3u + 1u) * 2.0 - 1.0) * PI;
	var ph = asin(clamp(hash1(i * 3u + 2u) * 2.0 - 1.0, -1.0, 1.0));
	// Ribs: some particles sit on meridians and parallels, so the form reads
	// as a structure, not a solid blob.
	let rib = hash1(i * 3u + 9u);
	if (rib < 0.2) {
		th = round(th / (PI / 12.0)) * (PI / 12.0);
	} else if (rib < 0.36) {
		ph = round(ph / (PI / 10.0)) * (PI / 10.0);
	}
	let r1 = sf(th + mu.u5.x * ph, mu.u3.x, mu.u3.y, mu.u3.z, mu.u3.w);
	let r2 = sf(ph, mu.u4.x, mu.u4.y, mu.u4.z, mu.u4.w);
	var p = vec3f(r1 * cos(th) * r2 * cos(ph), r2 * sin(ph), r1 * sin(th) * r2 * cos(ph)) * mu.u5.y;
	let l = length(p);
	if (l > 1.0) { p = p / l; }
	return p;
}
fn isLoose(i: u32) -> bool {
	return hash1(i * 7u + 5u) < mu.u5.w;
}
`;

export const INSIDE_COMPUTE_WGSL = /* wgsl */ `
${COMMON}
@group(0) @binding(0) var<uniform> ep: EP;
@group(0) @binding(1) var<uniform> mu: MU;
@group(0) @binding(2) var<storage, read> moons: array<Moon>;
@group(0) @binding(3) var<storage, read_write> state: array<S>;
@group(0) @binding(4) var<storage, read> handoff: array<H>;

struct Target { p: vec2f, k: f32, depth: f32 };

fn targetOf(i: u32, t: f32, age: f32) -> Target {
	var o: Target;
	let c = mu.u1.xy;
	let R = mu.u1.z;
	let nMoon = u32(mu.u8.x + 0.5);
	o.k = 24.0;
	o.depth = 0.0;
	if (i < FORM_END) {
		var p = formPoint(i);
		let breath = 1.0 + 0.032 * sin(TAU * t / max(mu.u5.z, 0.5));
		if (isLoose(i)) {
			// Retention is the grip: what the memory no longer holds drifts out.
			let drift = fract(hash1(i * 11u + 3u) + t * 0.022);
			p = p * (1.06 + drift * 1.1);
			o.k = 5.0;
		}
		p = rotX(rotY(p * breath, t * 0.15 + mu.u9.z), 0.34 + mu.u9.w);
		o.p = c + p.xy * R * (1.0 + p.z * 0.22);
		o.depth = p.z;
		return o;
	}
	if (i < RING_END) {
		let j = i - FORM_END;
		let u = f32(j) / f32(RING_END - FORM_END);
		let a = PI * 0.5 - u * TAU;
		let r = mu.u7.z + (hash1(i * 5u) - 0.5) * 0.016;
		o.p = c + vec2f(cos(a), sin(a)) * r;
		o.k = 30.0;
		return o;
	}
	if (i < STREAM_END) {
		let j = i - RING_END;
		let e = j / PER_STREAM;
		if (e >= nMoon) {
			o.p = c;
			return o;
		}
		let m = moons[e];
		let u = f32(j % PER_STREAM) / f32(PER_STREAM);
		let causal = m.b.w > 0.5;
		let f = fract(u + t * select(0.11, 0.28, causal) + hash1(j) * 0.07);
		let dir = normalize(m.a.xy - c + vec2f(1e-5, 0.0));
		let near = c + dir * R * 0.92;
		let far = m.a.xy - dir * m.a.z * 1.4;
		var a = near;
		var b = far;
		if (m.a.w < 0.5) {
			a = far;
			b = near;
		}
		let side = vec2f(-dir.y, dir.x);
		let ctrl = (a + b) * 0.5 + side * length(b - a) * 0.2;
		let q = mix(mix(a, ctrl, f), mix(ctrl, b, f), f);
		o.p = q + side * (hash1(j * 13u + 7u) - 0.5) * 0.014;
		o.k = 0.0; // placed exactly (a flow, not a spring)
		return o;
	}
	if (i < MOON_END) {
		let j = i - STREAM_END;
		let e = j / PER_MOON;
		if (e >= nMoon) {
			o.p = c;
			return o;
		}
		let m = moons[e];
		let th = hash1(j * 3u + 1u) * TAU;
		let ph = asin(clamp(hash1(j * 3u + 2u) * 2.0 - 1.0, -1.0, 1.0));
		let p = rotY(vec3f(cos(th) * cos(ph), sin(ph), sin(th) * cos(ph)), t * 0.5 + f32(e) * 1.7);
		let hov = abs(mu.u6.w - f32(e)) < 0.5;
		let rad = m.a.z * select(1.0, 1.4, hov) * (1.0 + 0.06 * sin(t * 2.1 + f32(e)));
		o.p = select(c, m.a.xy + p.xy * rad, age >= m.c.w);
		o.depth = p.z;
		o.k = 34.0;
		return o;
	}
	// Dust: a slow haze orbiting the memory, inside its own region.
	let j = i - MOON_END;
	let s1 = hash1(j * 3u + 11u);
	let s2 = hash1(j * 5u + 13u);
	let rad = R * 1.2 + s1 * (mu.u1.w * 1.08 - R * 1.2);
	let ang = s2 * TAU + t * 0.03 / (0.35 + rad);
	o.p = c + vec2f(cos(ang), sin(ang) * 0.94) * rad;
	o.k = 2.5;
	return o;
}

@compute @workgroup_size(256)
fn cs(@builtin(global_invocation_id) gid: vec3u) {
	let i = gid.x;
	if (i >= TOTAL) { return; }
	let t = mu.u0.w;
	let age = mu.u0.y;
	let dt = clamp(mu.u0.z, 0.0, 0.05);
	var s = state[i];
	let gen = mu.u8.y;
	let mode = u32(mu.u8.z + 0.5);
	if (abs(s.m.x - gen) > 0.5) {
		// A new arrival: every particle starts where this memory comes from.
		var start = mu.u1.xy + (vec2f(hash1(i * 17u), hash1(i * 19u)) - 0.5) * 0.02;
		var v0 = vec2f(0.0);
		var hidx = 0.0;
		let nh = u32(mu.u8.w + 0.5);
		if (mode == 1u && nh > 0u) {
			let k = (i * 7919u + 13u) % nh;
			start = handoff[k].a.xy + (vec2f(hash1(i * 23u), hash1(i * 29u)) - 0.5) * 0.003;
			hidx = f32(k) + 1.0;
		} else if (mode == 2u) {
			start = mu.u9.xy + (vec2f(hash1(i * 23u), hash1(i * 29u)) - 0.5) * 0.05;
		} else {
			// Born from a point: a burst outward that the form then gathers in.
			let a = hash1(i * 31u) * TAU;
			v0 = vec2f(cos(a), sin(a)) * (0.4 + hash1(i * 37u) * 1.4);
		}
		s.pv = vec4f(start, v0);
		s.m = vec4f(gen, hidx, 1.0, 0.0);
	}
	let tg = targetOf(i, t, age);
	// Each particle is released into the form on its own beat, so the memory
	// assembles from its source instead of snapping into place.
	let release = select(hash1(i * 41u + 3u) * 0.35, 0.15 + hash1(i * 41u + 3u) * 0.95, mode != 0u);
	var pos = s.pv.xy;
	var vel = s.pv.zw;
	if (age > release) {
		if (tg.k <= 0.0) {
			// Streams flow along their edge exactly; the jump at the end of a
			// lap carries no velocity (no streak across the screen).
			let d = tg.p - pos;
			vel = select(vec2f(0.0), d / max(dt, 1e-3) * 0.5, length(d) < 0.12);
			pos = mix(pos, tg.p, select(0.18, 1.0, age > release + 0.9));
		} else {
			let k = tg.k;
			var acc = (tg.p - pos) * k - vel * (2.0 * sqrt(k) * 0.8);
			if (mu.u6.z > 0.5) {
				// The cursor stirs the light it passes.
				let d = pos - mu.u6.xy;
				let r2 = dot(d, d);
				acc += d / (r2 + 0.002) * 0.016 * exp(-r2 / 0.01);
			}
			vel += acc * dt;
			pos += vel * dt;
		}
	} else if (mode == 0u) {
		pos += vel * dt;
		vel *= 0.92;
	}
	s.m.z = max(0.0, s.m.z - dt * 0.7);
	s.m.w = tg.depth;
	s.pv = vec4f(pos, vel);
	state[i] = s;
}
`;

export const INSIDE_PARTICLES_WGSL = /* wgsl */ `
${COMMON}
@group(0) @binding(0) var<uniform> ep: EP;
@group(0) @binding(1) var<uniform> mu: MU;
@group(0) @binding(2) var<storage, read> moons: array<Moon>;
@group(0) @binding(3) var<storage, read> state: array<S>;
@group(0) @binding(4) var<storage, read> handoff: array<H>;

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
	let s = state[ii];
	let pos = s.pv.xy;
	let vel = s.pv.zw;
	let t = mu.u0.w;
	let age = mu.u0.y;
	let tc = mu.u2.rgb;
	let ret = mu.u2.w;
	let nMoon = u32(mu.u8.x + 0.5);
	let c = mu.u1.xy;
	let R = mu.u1.z;
	var col = vec3f(0.0);
	var size = 0.003;

	if (ii < FORM_END) {
		let depth = s.m.w * 0.5 + 0.5;
		let loose = isLoose(ii);
		// Lit from the front: the near side of the form brighter, the far side
		// a deeper shade of the same colour.
		col = tc * (0.32 + 0.9 * depth) * select(1.0, 0.34, loose);
		// A white-hot crest where the surface faces the viewer squarely.
		col += vec3f(1.0, 0.98, 0.92) * pow(depth, 9.0) * 0.55 * select(1.0, 0.0, loose);
		col *= 0.62 + 0.38 * ret;
		if (mu.u6.w > 98.0) { col *= 1.3; }
		// The seal: when the receipt checks out, a wave of light crosses the form.
		if (u32(mu.u7.x + 0.5) == 2u) {
			let dr = length(pos - c) / max(R, 1e-3) - mu.u7.y * 1.5;
			let wave = exp(-dr * dr * 22.0);
			col += vec3f(1.0, 0.9, 0.62) * wave * 1.8;
		}
		size = 0.0027 * (0.8 + 0.45 * depth);
	} else if (ii < RING_END) {
		let j = ii - FORM_END;
		let u = f32(j) / f32(RING_END - FORM_END);
		// The ring is a chain of links, one gap between each.
		let seg = fract(u * 96.0);
		let link = smoothstep(0.0, 0.1, seg) * (1.0 - smoothstep(0.82, 0.92, seg));
		let st = u32(mu.u7.x + 0.5);
		let pa = mu.u7.y;
		var b = 0.14;
		var rc = mix(vec3f(0.55, 0.78, 0.84), tc, 0.3);
		if (st == 1u) {
			// Checking: a front runs the chain, lighting each link it passes.
			let head = fract(pa / 1.1);
			let behind = fract(head - u);
			let lit = exp(-behind * 7.0);
			b = 0.14 + 1.5 * lit;
			rc = mix(rc, vec3f(1.0, 0.95, 0.8), lit);
		} else if (st == 2u) {
			// Sealed: the whole chain holds light, with a slow shimmer.
			let flash = exp(-pa * 1.3);
			b = 0.8 + 1.7 * flash + 0.14 * sin(u * TAU * 3.0 - t * 1.3);
			rc = mix(vec3f(1.0, 0.84, 0.5), vec3f(1.0, 0.98, 0.92), flash * 0.6);
		} else if (st == 3u) {
			// Broken: red, open at the top.
			let gap = smoothstep(0.02, 0.06, min(u, 1.0 - u));
			b = 0.95 * gap;
			rc = vec3f(1.0, 0.3, 0.28);
		}
		col = rc * b * link;
		size = 0.0024;
	} else if (ii < STREAM_END) {
		let j = ii - RING_END;
		let e = j / PER_STREAM;
		if (e >= nMoon) { return hidden(); }
		let m = moons[e];
		let appear = ease((age - m.c.w) / 0.7);
		if (appear <= 0.0) { return hidden(); }
		let u = f32(j % PER_STREAM) / f32(PER_STREAM);
		let causal = m.b.w > 0.5;
		let f = fract(u + t * select(0.11, 0.28, causal) + hash1(j) * 0.07);
		let head = smoothstep(0.55, 1.0, f);
		let hov = abs(mu.u6.w - f32(e)) < 0.5;
		col = mix(m.c.rgb * 0.42, mix(m.c.rgb, vec3f(1.0), 0.5), head) * select(0.55, 1.0, causal) * select(1.0, 2.3, hov) * appear;
		size = select(0.002, 0.0029, causal);
	} else if (ii < MOON_END) {
		let j = ii - STREAM_END;
		let e = j / PER_MOON;
		if (e >= nMoon) { return hidden(); }
		let m = moons[e];
		let appear = ease((age - m.c.w) / 0.6);
		if (appear <= 0.0) { return hidden(); }
		let hov = abs(mu.u6.w - f32(e)) < 0.5;
		let depth = s.m.w * 0.5 + 0.5;
		col = m.b.rgb * (0.4 + 0.75 * depth) * select(1.0, 2.2, hov) * appear;
		size = 0.0027;
	} else {
		let j = ii - MOON_END;
		let tw = 0.5 + 0.5 * sin(t * (0.5 + hash1(j) * 1.3) + hash1(j * 3u) * TAU);
		col = mix(vec3f(0.36, 0.5, 0.6), tc, 0.4) * (0.05 + 0.08 * tw);
		size = 0.002;
	}

	// A particle handed over by the page that was clicked keeps that page's
	// colour for a moment, then takes this memory's own.
	let hidx = s.m.y;
	if (hidx > 0.5) {
		let hp = handoff[u32(hidx - 1.0)];
		let k = 1.0 - ease((age - 0.25) / 1.5);
		col = mix(col, hp.b.rgb * (0.9 + 1.3 * hp.a.z), k);
	}
	// Arrival heat: freshly placed particles burn a little brighter.
	col += mix(tc, vec3f(1.0), 0.6) * s.m.z * 0.35 * select(1.0, 0.0, ii >= MOON_END);

	let spd = length(vel);
	let dir = select(vec2f(1.0, 0.0), vel / max(spd, 1e-5), spd > 1e-4);
	let stretch = 1.0 + min(spd * 0.035 / size, 9.0);
	let q = QUAD[vi];
	let w = pos + dir * q.x * size * stretch + vec2f(-dir.y, dir.x) * q.y * size;
	var o: VO;
	o.clip = toClip(w);
	o.uv = q;
	o.col = col / sqrt(stretch);
	return o;
}

@fragment
fn fs(i: VO) -> @location(0) vec4f {
	let rn = clamp(length(i.uv) / 0.94, 0.0, 1.0);
	let disc = 1.0 - smoothstep(0.0, 1.0, rn);
	let core = (1.0 - rn) * (1.0 - rn) * (1.0 - rn);
	return vec4f(i.col * (core + (1.0 - rn) * 0.4) * disc, 1.0);
}
`;

export const INSIDE_BACKDROP_WGSL = /* wgsl */ `
${COMMON}
@group(0) @binding(0) var<uniform> ep: EP;
@group(0) @binding(1) var<uniform> mu: MU;

struct VO { @builtin(position) clip: vec4f, @location(0) ndc: vec2f };

const TRI = array<vec2f, 3>(vec2f(-1.0, -3.0), vec2f(3.0, 1.0), vec2f(-1.0, 1.0));

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VO {
	let p = TRI[vi];
	var o: VO;
	o.clip = vec4f(p, 0.0, 1.0);
	o.ndc = p;
	return o;
}

@fragment
fn fs(i: VO) -> @location(0) vec4f {
	// A faint aura of the memory's own colour behind its form, breathing with it.
	let z = max(mu.u10.z, 0.1);
	let w = vec2f(i.ndc.x * mu.u0.x / z + mu.u10.x, i.ndc.y / z + mu.u10.y);
	let d = length(w - mu.u1.xy) / max(mu.u1.z, 1e-3);
	let breath = 0.85 + 0.15 * sin(TAU * mu.u0.w / max(mu.u5.z, 0.5));
	let aura = exp(-d * d * 0.55) * 0.05 * breath * (0.5 + 0.5 * mu.u2.w);
	return vec4f(mu.u2.rgb * aura, 1.0);
}
`;
