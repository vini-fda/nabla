// The ship's model, lit and textured. World axes: x right, y up the screen, z out of the
// screen toward the viewer. See ship3d.rs.

struct Uniforms {
    clip_from_model: mat4x4<f32>,
    // Rotation part only (the model is scaled uniformly), for normals.
    world_from_model: mat4x4<f32>,
    // x: 1 to gamma-encode the output (the target is not sRGB), 0 to write linear color.
    output: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;
// sRGB texture: sampling returns linear color.
@group(0) @binding(1) var base_color: texture_2d<f32>;
@group(0) @binding(2) var metallic_roughness: texture_2d<f32>;
@group(0) @binding(3) var texture_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
) -> VertexOut {
    var out: VertexOut;
    out.position = u.clip_from_model * vec4<f32>(position, 1.0);
    out.normal = (u.world_from_model * vec4<f32>(normal, 0.0)).xyz;
    out.uv = uv;
    return out;
}

// Key light from the upper left of the screen and above it, and a dimmer fill light from
// the lower right, so no side of the hull goes black.
const KEY: vec3<f32> = vec3<f32>(-0.45, 0.55, 0.70);
const FILL: vec3<f32> = vec3<f32>(0.6, -0.5, 0.6);
const VIEW: vec3<f32> = vec3<f32>(0.0, 0.0, 1.0);

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    var n = normalize(in.normal);
    if !front_facing {
        n = -n;
    }
    let albedo = textureSample(base_color, texture_sampler, in.uv).rgb;
    let mr = textureSample(metallic_roughness, texture_sampler, in.uv).rgb;
    let roughness = clamp(mr.g, 0.05, 1.0);
    let metallic = mr.b;

    let key = normalize(KEY);
    let key_diffuse = max(dot(n, key), 0.0);
    let fill_diffuse = max(dot(n, normalize(FILL)), 0.0);
    // Sky above, darker ground below.
    let ambient = mix(0.08, 0.25, 0.5 + 0.5 * n.z);

    // Blinn-Phong highlight, sharper and brighter the smoother the surface.
    let shininess = 2.0 / (roughness * roughness * roughness * roughness) - 2.0;
    let highlight = pow(max(dot(n, normalize(key + VIEW)), 0.0), shininess)
        * (shininess + 8.0) / 25.0 * key_diffuse;
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);

    let diffuse = albedo * (1.0 - metallic) * (ambient + 1.1 * key_diffuse + 0.35 * fill_diffuse);
    let reflected = f0 * (ambient + highlight);
    var color = diffuse + reflected;
    if u.output.x > 0.5 {
        color = pow(color, vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(color, 1.0);
}
