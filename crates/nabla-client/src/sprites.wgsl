// Soft additive sprites, colored and blended as egui colors and blends its own shapes. See
// sprites.rs.

@group(0) @binding(0) var sprite: texture_2d<f32>;
@group(0) @binding(1) var sprite_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // Gamma encoded, premultiplied.
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(
    // Clip x and y, and depth.
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(position, 1.0);
    out.uv = uv;
    out.color = color;
    return out;
}

// 0-1 linear from 0-1 sRGB gamma, as in egui's shader.
fn linear_from_gamma_rgb(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let lower = srgb / vec3<f32>(12.92);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

@fragment
fn fs_gamma_framebuffer(in: VertexOut) -> @location(0) vec4<f32> {
    return in.color * textureSample(sprite, sprite_sampler, in.uv);
}

@fragment
fn fs_linear_framebuffer(in: VertexOut) -> @location(0) vec4<f32> {
    let color = in.color * textureSample(sprite, sprite_sampler, in.uv);
    return vec4<f32>(linear_from_gamma_rgb(color.rgb), color.a);
}
