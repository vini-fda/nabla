// Soft bands of the gravitational potential on the plane z = 0, drawn under everything else
// on the canvas. The crisp equipotential contours are drawn separately, in Rust, by marching
// squares (field.rs): per-pixel bands alias where the field changes within one pixel (near a
// body, zoomed out), while world-space lines stay sub-cell accurate at any zoom. In the 3D
// view the lines are the depth-tested ribbons of the marks' pass (`overlays.rs`).

struct Uniforms {
    clip_from_world: mat4x4<f32>,
    // x: the world's gravity constant G, y: the body count, z: the facing fade, w: in
    // perspective, how far from the eye the plane is drawn (its fog's reach), 0 in
    // orthographic.
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> uniforms: Uniforms;
// x, y, radius and gravitational mass of each body.
@group(0) @binding(1) var<storage, read> bodies: array<vec4<f32>>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec2<f32>,
    // In perspective, the distance from the eye along the view: the clip w.
    @location(1) distance: f32,
};

@vertex fn vs_main(@location(0) world: vec3<f32>) -> VertexOut {
    var out: VertexOut;
    out.position = uniforms.clip_from_world * vec4<f32>(world, 1.0);
    out.world = world.xy;
    out.distance = out.position.w;
    return out;
}

// Where the plane's fog begins, as a part of its reach: `PLANE_FOG_START` in camera.rs.
const FOG_START: f32 = 0.25;

/// How much of the bands shows at `distance` from the eye along the view, as `Camera::fog`
/// has it for the rest of the plane's drawings: fading out smoothly toward the plane's
/// reach, so the plane melts into the horizon instead of ending at the edge of the quad.
fn fog(distance: f32) -> f32 {
    let reach = uniforms.params.w;
    if reach <= 0.0 {
        return 1.0;
    }
    return 1.0 - smoothstep(FOG_START * reach, reach, distance);
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

/// The deepest of the bodies' surface potentials, `G M / R`, as a magnitude: the scale the
/// glow toward the bodies is measured against, so it reads the same at any zoom.
fn deepest_surface() -> f32 {
    var deepest = 1e-12;
    let count = u32(uniforms.params.y);
    for (var i = 0u; i < count; i = i + 1u) {
        let body = bodies[i];
        deepest = max(deepest, uniforms.params.x * body.w / max(body.z, 1e-9));
    }
    return deepest;
}

// ln of the potential ratio between adjacent bands, matching the contour levels in field.rs.
const LN_BAND: f32 = 0.3465735902799726; // ln(sqrt(2))
// Band colors, gamma encoded like egui colors: the floor of a band, its crest along the
// contour line, and the glow rising toward the bodies, which adds its light to the bands.
const DEEP: vec3<f32> = vec3<f32>(0.022, 0.040, 0.085);
const CREST: vec3<f32> = vec3<f32>(0.075, 0.170, 0.350);
const GLOW: vec3<f32> = vec3<f32>(0.080, 0.270, 0.700);
const ALPHA: f32 = 0.55;
// How fast the crest falls away inside its contour line, as a part of a band, and how
// sharply it rises toward it from outside.
const FALL: f32 = 0.10;
const RISE: f32 = 3.0;
// The crest's average over a band, which it gives way to where a band is finer than the
// pixels can show: `1 / (RISE + 1)`, and about half of `FALL`.
const CREST_MEAN: f32 = 0.30;
// How steeply the glow fades with the potential away from the bodies: as `(phi / phi_s)^k`
// of the deepest surface potential, so `(R / r)^k` around a lone body.
const GLOW_FALLOFF: f32 = 1.0;

/// How bright a band's crest is at the band coordinate `x` (bands of the log-potential,
/// growing toward the bodies), 0 to 1: rising toward each contour line from outside, the
/// way the field lines fall, and dying away just within it, continuous across the line.
/// `grain` is how much `x` changes over a pixel: where that is a good part of a band, the
/// crests are finer than the pixels can show, and give way to their average.
fn crest_at(x: f32, grain: f32) -> f32 {
    let w = fract(x);
    let shaped = max(pow(w, RISE), 1.0 - smoothstep(0.0, FALL, w));
    return mix(shaped, CREST_MEAN, smoothstep(0.15, 0.45, grain));
}

fn band(world: vec2<f32>, distance: f32) -> vec4<f32> {
    let depth = max(-potential(world), 1e-12);
    let x = log(depth) / LN_BAND;
    let crest = crest_at(x, fwidth(x));
    let glow = pow(clamp(depth / deepest_surface(), 0.0, 1.0), GLOW_FALLOFF);
    let alpha = ALPHA * uniforms.params.z * fog(distance);
    let color = mix(DEEP, CREST, crest * (0.55 + 0.45 * glow)) + GLOW * glow;
    return vec4<f32>(color * alpha, alpha);
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
    return band(in.world, in.distance);
}

@fragment
fn fs_linear_framebuffer(in: VertexOut) -> @location(0) vec4<f32> {
    let out = band(in.world, in.distance);
    return vec4<f32>(linear_from_gamma_rgb(out.rgb), out.a);
}
