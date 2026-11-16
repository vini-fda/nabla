// Soft bands of the gravitational potential on the plane z = 0, drawn under everything else
// on the canvas. The crisp equipotential contours are drawn separately, in Rust, by marching
// squares (field.rs): per-pixel bands alias where the field changes within one pixel (near a
// body, zoomed out), while world-space lines stay sub-cell accurate at any zoom. In the 3D
// view the lines are the depth-tested ribbons of the marks' pass (`overlays.rs`).

struct Uniforms {
    clip_from_world: mat4x4<f32>,
    // x: the world's gravity constant G, y: the body count, z: the facing fade, w: unused.
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> uniforms: Uniforms;
// x, y, radius and gravitational mass of each body.
@group(0) @binding(1) var<storage, read> bodies: array<vec4<f32>>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec2<f32>,
};

@vertex fn vs_main(@location(0) world: vec3<f32>) -> VertexOut {
    var out: VertexOut;
    out.position = uniforms.clip_from_world * vec4<f32>(world, 1.0);
    out.world = world.xy;
    return out;
}

/// The gravitational potential at `p` on the plane, whose negative gradient is the
/// simulation's acceleration: `-G M / r` outside a body, and the uniform solid sphere's
/// `-G M (3 R^2 - r^2) / (2 R^3)` inside it, continuous at the surface and zero at the center.
fn potential(p: vec2<f32>) -> f32 {
    var phi = 0.0;
    let count = u32(uniforms.params.y);
    for (var i = 0u; i < count; i = i + 1u) {
        let body = bodies[i];
        let d = body.xy - p;
        let r = length(d);
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

// ln of the potential ratio between adjacent bands, matching the contour levels in field.rs.
const LN_BAND: f32 = 0.3465735902799726; // ln(sqrt(2))
// Band colors, gamma encoded like egui colors; alpha is applied on top.
const DEEP: vec3<f32> = vec3<f32>(0.035, 0.060, 0.110);
const LIGHT: vec3<f32> = vec3<f32>(0.090, 0.150, 0.260);
const ALPHA: f32 = 0.5;

fn band(world: vec2<f32>) -> vec4<f32> {
    let wrapped = fract(log(max(-potential(world), 1e-12)) / LN_BAND);
    // A smooth periodic ramp, continuous across the band wrap, so per-pixel sampling
    // leaves no hard edge to alias. It peaks exactly where the contour lines sit (where
    // the phase wraps: -potential = LEVEL_BASE^n), so the bands crest along with them.
    let v = 0.5 + 0.5 * cos(6.283185307179586 * wrapped);
    let alpha = ALPHA * uniforms.params.z;
    return vec4<f32>(mix(DEEP, LIGHT, v) * alpha, alpha);
}

// 0-1 linear from 0-1 sRGB gamma, as in egui's shader.
fn linear_from_gamma_rgb(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let lower = srgb / vec3<f32>(12.92);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

// -------------------------------------------------------------------------------------------
// The contour lines are drawn by the marks' pass (`overlays.wgsl`), depth-tested against the
// planets and ships that drew before them.

@fragment
fn fs_gamma_framebuffer(in: VertexOut) -> @location(0) vec4<f32> {
    return band(in.world);
}

@fragment
fn fs_linear_framebuffer(in: VertexOut) -> @location(0) vec4<f32> {
    let out = band(in.world);
    return vec4<f32>(linear_from_gamma_rgb(out.rgb), out.a);
}
