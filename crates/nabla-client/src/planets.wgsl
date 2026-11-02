struct Uniforms {
    clip_from_world: mat4x4<f32>,
    output: vec4<f32>,
};
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
};

@vertex fn vs_main(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>) -> Vertex {
    var out: Vertex;
    out.position = uniforms.clip_from_world * vec4<f32>(position, 1.0);
    out.normal = normal;
    return out;
}

@fragment fn fs_main(in: Vertex) -> @location(0) vec4<f32> {
    let light = normalize(vec3<f32>(-0.5, 0.6, 1.0));
    let diffuse = max(dot(normalize(in.normal), light), 0.0);
    var color = vec3<f32>(0.72, 0.77, 0.84) * (0.12 + 0.88 * diffuse);
    if uniforms.output.x > 0.5 {
        color = pow(color, vec3<f32>(1.0 / 2.2));
    }
    return vec4<f32>(color, 1.0);
}
