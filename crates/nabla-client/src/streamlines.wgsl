// The field lines of the bodies' gravity on the plane z = 0: glowing red streamlines with
// arrowheads, and pulses of light falling along them the way the field points.
//
// The geometry comes from streamlines.rs, already in the viewport's device pixels: each line a
// ribbon with miter joins, wide enough for its glow, each vertex carrying its signed distance
// across the ribbon; each arrowhead a triangle, each vertex carrying its distance to the
// triangle's three edges. Every fragment's alpha is a fence on those distances, so the edges
// are anti-aliased analytically at any width, with or without multisampling.
//
// The pulses are wavefronts on the equipotentials: their phase is the log-potential, the same
// world-anchored coordinate the contour lines of field.wgsl are levels of, evaluated per
// fragment. So they neither move with the view nor depend on how the lines were tessellated,
// they cross from one contour line to the next in the same time anywhere, and neighbouring
// lines pulse together.

struct Uniforms {
    // x, y: the callback viewport's size in device pixels; z: half the core line's width,
    // w: the anti-aliasing feather, both in device pixels.
    view: vec4<f32>,
    // x: the world's gravity constant G; y: the body count; z: the pulses' phase from the
    // clock, in periods, 0 to 1; w: how far the glow reaches past the core, in device pixels.
    params: vec4<f32>,
    // x: the facing fade; the rest unused.
    fade: vec4<f32>,
};
@group(0) @binding(0) var<uniform> uniforms: Uniforms;
// x, y, radius and gravitational mass of each body.
@group(0) @binding(1) var<storage, read> bodies: array<vec4<f32>>;

struct VertexIn {
    // x, y: device pixels from the viewport's corner; z: the depth the plane lies at, 0 to 1.
    @location(0) pos: vec3<f32>,
    // w: 0 for a line's ribbon, 1 for an arrowhead. A ribbon's x: the signed distance across
    // it, in device pixels. An arrowhead's x, y, z: the distances to its three edges.
    @location(1) shape: vec4<f32>,
    // The world point `(x, y) * q` and `q`, the perspective's scale there (1 in
    // orthographic): interpolated across the screen and divided again, they give each
    // fragment's own world point exactly, however long the segment.
    @location(2) world: vec3<f32>,
    // The primitive's own opacity: an arrowhead fading in as the zoom makes room for it.
    @location(3) opacity: f32,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) shape: vec4<f32>,
    @location(1) world: vec3<f32>,
    @location(2) opacity: f32,
};

@vertex
fn vs_main(vertex: VertexIn) -> VertexOut {
    var out: VertexOut;
    // From the viewport's top-left pixels to clip space, egui's y down flipped as its own
    // shader flips it.
    let centered = vec2<f32>(
        2.0 * vertex.pos.x / uniforms.view.x - 1.0,
        1.0 - 2.0 * vertex.pos.y / uniforms.view.y,
    );
    out.position = vec4<f32>(centered, vertex.pos.z, 1.0);
    out.shape = vertex.shape;
    out.world = vertex.world;
    out.opacity = vertex.opacity;
    return out;
}

/// The gravitational potential at `p` on the plane, as in field.wgsl: `-G M / r` outside a
/// body, and the uniform solid sphere's inside it.
fn potential(p: vec2<f32>) -> f32 {
    var phi = 0.0;
    let count = u32(uniforms.params.y);
    for (var i = 0u; i < count; i = i + 1u) {
        let body = bodies[i];
        let r = length(body.xy - p);
        let gm = uniforms.params.x * body.w;
        if r >= body.z {
            phi -= gm / max(r, 1e-9);
        } else {
            let R = body.z;
            phi -= gm * (3.0 * R * R - r * r) / (2.0 * R * R * R);
        }
    }
    return phi;
}

// ln of the potential ratio between adjacent contour lines, as in field.wgsl and field.rs.
const LN_BAND: f32 = 0.3465735902799726; // ln(sqrt(2))
// How many of those bands lie between one pulse and the next.
const PULSE_BANDS: f32 = 2.0;
// How fast a pulse's tail dies away behind its head, per period.
const TAIL: f32 = 4.0;

// Colors, gamma encoded like egui's: the line at rest, the line under a pulse's head, and the
// glow around it.
const RED: vec3<f32> = vec3<f32>(0.92, 0.16, 0.24);
const HOT: vec3<f32> = vec3<f32>(1.00, 0.80, 0.68);
const GLOW: vec3<f32> = vec3<f32>(1.00, 0.14, 0.20);

/// How bright the pulses are at the pulse coordinate `x` (periods of the log-potential, less
/// the clock's phase), 0 to 1: a head with a soft front edge, and a tail dying away behind
/// it. As `x` grows toward the bodies and the clock's phase is subtracted, the pulses fall
/// inward. `grain` is how much `x` changes over a pixel: where that is a good part of a
/// period, the pulses are finer than the pixels can show, and give way to their average.
fn pulse_at(x: f32, grain: f32) -> f32 {
    let phase = fract(x);
    // The head's front, at least a pixel and a half wide.
    let front = clamp(1.5 * grain, 0.04, 0.5);
    let shaped = exp(-TAIL * (1.0 - phase)) * (1.0 - smoothstep(1.0 - front, 1.0, phase));
    return mix(shaped, 1.0 / TAIL, smoothstep(0.2, 0.5, grain));
}

// 0-1 linear from 0-1 sRGB gamma, as in egui's shader.
fn linear_from_gamma_rgb(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let lower = srgb / vec3<f32>(12.92);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

/// The fragment's color, premultiplied by its alpha, its channels gamma encoded, or linear
/// for a `linear` framebuffer.
fn shade(in: VertexOut, linear: bool) -> vec4<f32> {
    let world = in.world.xy / in.world.z;
    let x = log(max(-potential(world), 1e-12)) / (LN_BAND * PULSE_BANDS);
    let pulse = pulse_at(x - uniforms.params.z, fwidth(x));
    let half = uniforms.view.z;
    let feather = uniforms.view.w;

    var lit = mix(RED, HOT, pulse);
    var glow = GLOW;
    if linear {
        lit = linear_from_gamma_rgb(lit);
        glow = linear_from_gamma_rgb(glow);
    }

    var color: vec4<f32>;
    if in.shape.w > 0.5 {
        // An arrowhead: inside all three of its edges, each pushed out by half the feather.
        let inside = min(in.shape.x, min(in.shape.y, in.shape.z));
        let covered = clamp(inside / feather, 0.0, 1.0);
        color = vec4<f32>(lit, 1.0) * (covered * (0.75 + 0.25 * pulse));
    } else {
        let d = abs(in.shape.x);
        // The core: a box filter's worth of feather, centered on its edge.
        let core = clamp((half - d) / feather + 0.5, 0.0, 1.0) * (0.6 + 0.4 * pulse);
        // The glow: falling off from the core's edge to nothing at its reach.
        let g = clamp(max(d - half, 0.0) / uniforms.params.w, 0.0, 1.0);
        let halo = exp(-3.5 * g) * (1.0 - g) * (1.0 - g) * (0.30 + 0.70 * pulse);
        // The core over the glow, which adds its light to what is under it more than it
        // covers it.
        let under = (1.0 - core) * halo;
        color = vec4<f32>(lit * core + glow * under, core + 0.4 * under);
    }
    return color * (in.opacity * uniforms.fade.x);
}

@fragment
fn fs_gamma_framebuffer(in: VertexOut) -> @location(0) vec4<f32> {
    return shade(in, false);
}

@fragment
fn fs_linear_framebuffer(in: VertexOut) -> @location(0) vec4<f32> {
    return shade(in, true);
}
