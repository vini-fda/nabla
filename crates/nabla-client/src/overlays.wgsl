// The marks over the canvas in the 3D view: the gravity field's contour lines (`field.rs`),
// the objective's orbit rings and its target with its tolerance, the path flown and the
// controller's prediction (`app.rs`). They are ribbons over the plane or billboards at a
// point of it, depth-tested against the planets, ships and plumes that drew before them.
//
// Each ribbon vertex carries the stroke's own shape and color, so one pass draws every
// mark of a frame whatever its width or color. As in the gravity shader (`field.wgsl`),
// each vertex also carries its signed distance across the ribbon, in device pixels, and
// each fragment's alpha is a fence on that distance: how much of the pixel the stroke
// covers. So the lines are anti-aliased at any width, with or without multisampling, and
// look as egui's own strokes of the same width and color do. A stroke may also glow: a soft
// halo of its own color past its edge, falling off to nothing at the glow's reach, as the
// field lines' glow does (`streamlines.wgsl`), which adds its light more than it covers.

struct Uniforms {
    // The canvas's size in device pixels.
    size: vec2<f32>,
};
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexIn {
    // x, y: device pixels from the canvas's corner; z: the mark's depth there, 0 to 1.
    @location(0) pos: vec3<f32>,
    // The signed distance across the ribbon, from the line along its middle, in device
    // pixels.
    @location(1) across: f32,
    // x: half the stroke's width, y: the anti-aliasing feather, z: how far its glow reaches
    // past its edge (0 for none), in device pixels.
    @location(2) shape: vec3<f32>,
    // The mark's color as egui's colors: sRGB gamma channels from 0 to 1, premultiplied by
    // their alpha.
    @location(3) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) across: f32,
    @location(1) shape: vec3<f32>,
    @location(2) color: vec4<f32>,
};

@vertex
fn vs_main(vertex: VertexIn) -> VertexOut {
    var out: VertexOut;
    // From the canvas's top-left device pixels to clip space, egui's y down flipped as its
    // own shader flips it.
    let centered = vec2<f32>(
        2.0 * vertex.pos.x / max(uniforms.size.x, 1.0) - 1.0,
        1.0 - 2.0 * vertex.pos.y / max(uniforms.size.y, 1.0),
    );
    out.position = vec4<f32>(centered, vertex.pos.z, 1.0);
    out.across = vertex.across;
    out.shape = vertex.shape;
    out.color = vertex.color;
    return out;
}

/// How much of a pixel `d` from the middle of a stroke of half-width `half` the stroke
/// covers: the overlap of the stroke with a box filter `feather` wide there. A stroke wider
/// than the feather is full inside and fades over the feather, centered on its edge; a
/// thinner one never fills a pixel, and is as much fainter, rather than wider than it is.
/// A vertex with `across` 0 and `half` at least half the feather marks a filled shape
/// instead: its fragments all cover their pixel fully.
fn coverage(d: f32, half: f32, feather: f32) -> f32 {
    let overlap = min(d + 0.5 * feather, half) - max(d - 0.5 * feather, -half);
    return clamp(overlap / feather, 0.0, 1.0);
}

// 0-1 linear from 0-1 sRGB gamma, as in egui's shader.
fn linear_from_gamma_rgb(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let lower = srgb / vec3<f32>(12.92);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

// How bright a glow is right at its stroke's edge, next to the stroke's own color.
const GLOW_STRENGTH: f32 = 0.45;

/// The fragment's color, premultiplied by its alpha: the stroke over its glow, if it has one.
/// `color` is the stroke's, gamma encoded or linear as the framebuffer wants.
fn shade(in: VertexOut, color: vec4<f32>) -> vec4<f32> {
    let d = abs(in.across);
    let covered = coverage(d, in.shape.x, in.shape.y);
    let reach = in.shape.z;
    if reach <= 0.0 {
        return color * covered;
    }
    // Falling off from the stroke's edge to nothing at its reach, as the field lines' glow.
    let g = clamp(max(d - in.shape.x, 0.0) / reach, 0.0, 1.0);
    let halo = GLOW_STRENGTH * exp(-3.5 * g) * (1.0 - g) * (1.0 - g);
    let under = (1.0 - covered) * halo;
    return color * covered + vec4<f32>(color.rgb * under, 0.4 * color.a * under);
}

@fragment
fn fs_gamma_framebuffer(in: VertexOut) -> @location(0) vec4<f32> {
    return shade(in, in.color);
}

@fragment
fn fs_linear_framebuffer(in: VertexOut) -> @location(0) vec4<f32> {
    return shade(in, vec4<f32>(linear_from_gamma_rgb(in.color.rgb), in.color.a));
}
