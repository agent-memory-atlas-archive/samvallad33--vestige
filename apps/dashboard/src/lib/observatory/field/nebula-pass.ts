/**
 * The Field's universe: a nebula behind the mind.
 *
 * Clouds of colour in the kinds of memory the stars wear (decision amber,
 * correction red, fact cyan, event green, pattern violet), folding slowly
 * through each other, densest round the sphere and brightening where the
 * pointer moves. Additive and quiet, so every star stays the brightest
 * thing on screen.
 */
import type { FramePass, ObservatoryEngine } from '../engine';

const WGSL = /* wgsl */ `
struct Params {
	frame: f32, loop_phase: f32, node_count: f32, edge_count: f32,
	path_count: f32, pulse: f32, viewport_w: f32, viewport_h: f32,
	brightness: f32, demo_id: f32, time: f32, capture_mode: f32,
	live_kind: f32, live_frame: f32, live_energy: f32, projection_days: f32,
	cursor_x: f32, cursor_y: f32, cursor_vx: f32, cursor_vy: f32,
};
struct U { centre: vec4f }; // sphere centre xy (NDC), its radius, 0
@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<uniform> u: U;

struct VO { @builtin(position) clip: vec4f };
const TRI = array<vec2f, 3>(vec2f(-1.0, -3.0), vec2f(3.0, 1.0), vec2f(-1.0, 1.0));
@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VO {
	var o: VO;
	o.clip = vec4f(TRI[vi], 0.0, 1.0);
	return o;
}
fn h21(q: vec2f) -> f32 {
	var p3 = fract(vec3f(q.x, q.y, q.x) * 0.1031);
	p3 += dot(p3, p3.yzx + 33.33);
	return fract((p3.x + p3.y) * p3.z);
}
fn vnoise(q: vec2f) -> f32 {
	let i = floor(q);
	let f = fract(q);
	let w = f * f * (3.0 - 2.0 * f);
	return mix(mix(h21(i), h21(i + vec2f(1.0, 0.0)), w.x), mix(h21(i + vec2f(0.0, 1.0)), h21(i + vec2f(1.0, 1.0)), w.x), w.y);
}
fn fbm(q0: vec2f) -> f32 {
	var v = 0.0;
	var a = 0.5;
	var q = q0;
	for (var k = 0; k < 5; k++) {
		v += a * vnoise(q);
		q = q * 2.02 + vec2f(3.1, 1.7);
		a *= 0.5;
	}
	return v;
}
@fragment
fn fs(i: VO) -> @location(0) vec4f {
	let res = vec2f(max(p.viewport_w, 1.0), max(p.viewport_h, 1.0));
	let ndc = vec2f(i.clip.x / res.x * 2.0 - 1.0, 1.0 - i.clip.y / res.y * 2.0);
	let A = res.x / res.y;
	// The same space the stage writes the cursor in (wide: x stretched, tall: y stretched).
	let q = vec2f(ndc.x * max(A, 1.0), ndc.y / min(A, 1.0));
	let t = p.time;
	// Domain-warped clouds, folding slowly.
	let warp = vec2f(fbm(q * 1.1 + vec2f(t * 0.02, 0.0)), fbm(q * 1.1 + vec2f(5.2, t * 0.018)));
	let n = fbm(q * 1.4 + warp * 1.6 + vec2f(0.0, t * 0.01));
	let hue = fbm(q * 0.6 + warp * 0.8 + 9.0);
	// The kinds of memory, blended by where the cloud is.
	let amber = vec3f(1.0, 0.74, 0.3);
	let red = vec3f(1.0, 0.42, 0.33);
	let cyan = vec3f(0.35, 0.88, 1.0);
	let green = vec3f(0.3, 1.0, 0.68);
	let violet = vec3f(0.72, 0.62, 1.0);
	let k = hue * 5.0;
	var col = mix(violet, cyan, clamp(k - 1.0, 0.0, 1.0));
	col = mix(col, green, clamp(k - 2.0, 0.0, 1.0));
	col = mix(col, amber, clamp(k - 2.8, 0.0, 1.0));
	col = mix(col, red, clamp(k - 3.6, 0.0, 1.0));
	let cloud = smoothstep(0.42, 0.85, n);
	// Densest round the sphere, fading toward the words' column.
	let c = vec2f(u.centre.x * max(A, 1.0), u.centre.y / min(A, 1.0));
	let dc = length(q - c) / max(u.centre.z, 0.1);
	let around = exp(-dc * dc * 0.35);
	let cur = vec2f(p.cursor_x, p.cursor_y);
	let near = exp(-dot(q - cur, q - cur) * 3.0);
	let glow = col * cloud * (0.05 + 0.13 * around + 0.22 * near);
	// A few far stars of its own.
	let sp = floor(q * 90.0);
	let star = step(0.9975, h21(sp)) * (0.5 + 0.5 * sin(t * 2.0 + h21(sp + 7.0) * 30.0));
	return vec4f(glow + vec3f(0.85, 0.9, 1.0) * star * 0.35, 1.0);
}
`;

export class NebulaPass implements FramePass {
	private engine: ObservatoryEngine;
	private pipe: GPURenderPipeline | null = null;
	private bind: GPUBindGroup | null = null;
	private ubuf: GPUBuffer | null = null;
	private centre = new Float32Array([0, 0, 0.8, 0]);

	constructor(engine: ObservatoryEngine) {
		this.engine = engine;
		const device = engine.gpuDevice;
		const params = engine.paramsBuffer;
		if (!device || !params) return;
		this.ubuf = device.createBuffer({ size: 16, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST });
		const module = device.createShaderModule({ label: 'field-nebula', code: WGSL });
		this.pipe = device.createRenderPipeline({
			label: 'field-nebula',
			layout: 'auto',
			vertex: { module, entryPoint: 'vs' },
			fragment: {
				module,
				entryPoint: 'fs',
				targets: [
					{
						format: engine.sceneFormat,
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
			layout: this.pipe.getBindGroupLayout(0),
			entries: [
				{ binding: 0, resource: { buffer: params } },
				{ binding: 1, resource: { buffer: this.ubuf } }
			]
		});
	}

	/** Where the sphere sits (NDC) and how big it is, so the clouds gather round it. */
	setCentre(x: number, y: number, r: number): void {
		this.centre.set([x, y, r, 0]);
	}

	compute(): void {
		if (this.ubuf) this.engine.gpuDevice?.queue.writeBuffer(this.ubuf, 0, this.centre);
	}

	render(pass: GPURenderPassEncoder): void {
		if (!this.pipe || !this.bind) return;
		pass.setPipeline(this.pipe);
		pass.setBindGroup(0, this.bind);
		pass.draw(3);
	}

	dispose(): void {
		this.ubuf?.destroy();
		this.ubuf = null;
	}
}
