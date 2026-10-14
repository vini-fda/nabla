//! The ship's textured 3D model, and an OpenGL renderer that draws it into the canvas,
//! seen from straight above with an orthographic projection that matches the 2D
//! [`Camera`], so the model lines up with everything the canvas paints around it.
//!
//! The model is `assets/nabla_starship.glb`: one mesh with positions, normals and
//! texture coordinates, a base color texture (2048 px JPEG) and a metallic-roughness
//! texture (1024 px JPEG). In model space the ship's nose points along +y.

#![expect(unsafe_code, reason = "OpenGL calls through glow are unsafe")]

use std::sync::Arc;

use eframe::glow::{self, HasContext as _};
use eframe::{egui_glow, glow::Context};

use crate::camera::Camera;

/// The model, embedded in the binary.
#[expect(
    clippy::large_include_file,
    reason = "the ship model, about 1.9 MB, textures included"
)]
pub const SHIP_GLB: &[u8] = include_bytes!("../assets/nabla_starship.glb");

/// Which way of the model faces the camera (up, out of the screen): +z is the side with
/// both bays open onto the machinery inside. The nose is +y.
const MODEL_TOP: [f32; 3] = [0.0, 0.0, 1.0];

/// An RGBA8 image.
struct Image {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

/// The ship's mesh and textures, as read from the glTF file.
pub struct ShipModel {
    /// Interleaved position (3), normal (3), texture coordinates (2) per vertex.
    vertices: Vec<f32>,
    indices: Vec<u32>,
    base_color: Image,
    metallic_roughness: Image,
    /// Distance from the model's origin to its nose, along +y.
    half_length: f32,
}

/// Bytes per vertex in the vertex buffer: eight four-byte floats.
const VERTEX_STRIDE: i32 = 8 * 4;

impl ShipModel {
    /// Reads the first mesh primitive of a binary glTF file, and its material's textures.
    pub fn load(glb: &[u8]) -> Result<Self, String> {
        let gltf = gltf::Gltf::from_slice(glb).map_err(|e| e.to_string())?;
        let blob = gltf.blob.as_deref().ok_or("no binary chunk")?;
        let primitive = gltf
            .meshes()
            .next()
            .and_then(|mesh| mesh.primitives().next())
            .ok_or("no mesh")?;
        let reader = primitive.reader(|buffer| (buffer.index() == 0).then_some(blob));

        let positions: Vec<[f32; 3]> = reader.read_positions().ok_or("no positions")?.collect();
        let normals: Vec<[f32; 3]> = reader.read_normals().ok_or("no normals")?.collect();
        let uvs: Vec<[f32; 2]> = reader
            .read_tex_coords(0)
            .ok_or("no texture coordinates")?
            .into_f32()
            .collect();
        let indices: Vec<u32> = reader
            .read_indices()
            .ok_or("no indices")?
            .into_u32()
            .collect();
        let vertices = positions
            .iter()
            .zip(&normals)
            .zip(&uvs)
            .flat_map(|((p, n), uv)| [p[0], p[1], p[2], n[0], n[1], n[2], uv[0], uv[1]])
            .collect();
        let half_length = positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max);

        let pbr = primitive.material().pbr_metallic_roughness();
        let texture_image = |texture: Option<gltf::texture::Texture<'_>>| -> Result<Image, String> {
            let texture = texture.ok_or("missing texture")?;
            let gltf::image::Source::View { view, .. } = texture.source().source() else {
                return Err("texture not embedded".to_owned());
            };
            let bytes = &blob[view.offset()..view.offset() + view.length()];
            let image = image::load_from_memory(bytes)
                .map_err(|e| e.to_string())?
                .to_rgba8();
            Ok(Image {
                width: image.width(),
                height: image.height(),
                pixels: image.into_raw(),
            })
        };
        Ok(Self {
            vertices,
            indices,
            base_color: texture_image(pbr.base_color_texture().map(|info| info.texture()))?,
            metallic_roughness: texture_image(
                pbr.metallic_roughness_texture().map(|info| info.texture()),
            )?,
            half_length,
        })
    }
}

const VERTEX_SHADER: &str = r"
in vec3 a_position;
in vec3 a_normal;
in vec2 a_uv;
uniform mat4 u_clip_from_model;
uniform mat3 u_world_from_model;
out vec3 v_normal;
out vec2 v_uv;

void main() {
    gl_Position = u_clip_from_model * vec4(a_position, 1.0);
    v_normal = u_world_from_model * a_normal;
    v_uv = a_uv;
}
";

/// Lit in linear color, written out gamma-encoded like the rest of egui. World axes: x right,
/// y up the screen, z out of the screen toward the viewer.
const FRAGMENT_SHADER: &str = r"
#ifdef GL_ES
precision highp float;
#endif
in vec3 v_normal;
in vec2 v_uv;
uniform sampler2D u_base_color;
uniform sampler2D u_metallic_roughness;
out vec4 out_color;

// Key light from the upper left of the screen and above it, and a dimmer fill light from
// the lower right, so no side of the hull goes black.
const vec3 KEY = vec3(-0.45, 0.55, 0.70);
const vec3 FILL = vec3(0.6, -0.5, 0.6);
const vec3 VIEW = vec3(0.0, 0.0, 1.0);

vec3 to_linear(vec3 c) { return pow(c, vec3(2.2)); }
vec3 to_gamma(vec3 c) { return pow(c, vec3(1.0 / 2.2)); }

void main() {
    vec3 n = normalize(gl_FrontFacing ? v_normal : -v_normal);
    vec3 albedo = to_linear(texture(u_base_color, v_uv).rgb);
    vec3 mr = texture(u_metallic_roughness, v_uv).rgb;
    float roughness = clamp(mr.g, 0.05, 1.0);
    float metallic = mr.b;

    vec3 key = normalize(KEY);
    float key_diffuse = max(dot(n, key), 0.0);
    float fill_diffuse = max(dot(n, normalize(FILL)), 0.0);
    // Sky above, darker ground below.
    float ambient = mix(0.08, 0.25, 0.5 + 0.5 * n.z);

    // Blinn-Phong highlight, sharper and brighter the smoother the surface.
    float shininess = 2.0 / (roughness * roughness * roughness * roughness) - 2.0;
    float highlight = pow(max(dot(n, normalize(key + VIEW)), 0.0), shininess)
        * (shininess + 8.0) / 25.0 * key_diffuse;
    vec3 f0 = mix(vec3(0.04), albedo, metallic);

    vec3 diffuse = albedo * (1.0 - metallic) * (ambient + 1.1 * key_diffuse + 0.35 * fill_diffuse);
    vec3 reflected = f0 * (ambient + highlight);
    out_color = vec4(to_gamma(diffuse + reflected), 1.0);
}
";

/// The model's GPU resources.
pub struct ShipRenderer {
    program: glow::Program,
    vertex_array: glow::VertexArray,
    vertex_buffer: glow::Buffer,
    index_buffer: glow::Buffer,
    index_count: i32,
    base_color: glow::Texture,
    metallic_roughness: glow::Texture,
    half_length: f32,
}

impl ShipRenderer {
    /// Uploads `model`. Fails where the GL lacks GLSL 1.40 / ES 3.00 shaders.
    pub fn new(gl: &Context, model: &ShipModel) -> Result<Self, String> {
        let version = egui_glow::ShaderVersion::get(gl);
        if !version.is_new_shader_interface() {
            return Err(format!("unsupported shader version {version:?}"));
        }
        // SAFETY: plain object creation and uploads on the current context.
        unsafe {
            let program = compile_program(gl, version.version_declaration())?;

            let vertex_array = gl.create_vertex_array()?;
            gl.bind_vertex_array(Some(vertex_array));
            let vertex_buffer = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vertex_buffer));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                &to_bytes(&model.vertices),
                glow::STATIC_DRAW,
            );
            for (location, size, offset) in [(0, 3, 0), (1, 3, 3), (2, 2, 6)] {
                gl.enable_vertex_attrib_array(location);
                gl.vertex_attrib_pointer_f32(
                    location,
                    size,
                    glow::FLOAT,
                    false,
                    VERTEX_STRIDE,
                    4 * offset,
                );
            }
            let index_buffer = gl.create_buffer()?;
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(index_buffer));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                &model
                    .indices
                    .iter()
                    .flat_map(|i| i.to_ne_bytes())
                    .collect::<Vec<_>>(),
                glow::STATIC_DRAW,
            );
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);

            Ok(Self {
                program,
                vertex_array,
                vertex_buffer,
                index_buffer,
                index_count: i32::try_from(model.indices.len()).map_err(|e| e.to_string())?,
                base_color: upload_texture(gl, &model.base_color)?,
                metallic_roughness: upload_texture(gl, &model.metallic_roughness)?,
                half_length: model.half_length,
            })
        }
    }

    pub fn destroy(&self, gl: &Context) {
        // SAFETY: deletes only objects this renderer created.
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vertex_array);
            gl.delete_buffer(self.vertex_buffer);
            gl.delete_buffer(self.index_buffer);
            gl.delete_texture(self.base_color);
            gl.delete_texture(self.metallic_roughness);
        }
    }

    fn paint(&self, gl: &Context, clip_from_model: &[f32; 16], world_from_model: &[f32; 9]) {
        // SAFETY: draws with this renderer's objects; egui restores its own state afterwards.
        unsafe {
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LESS);
            gl.depth_mask(true);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::DEPTH_BUFFER_BIT);
            gl.disable(glow::BLEND);

            gl.use_program(Some(self.program));
            let uniform = |name| gl.get_uniform_location(self.program, name);
            gl.uniform_matrix_4_f32_slice(
                uniform("u_clip_from_model").as_ref(),
                false,
                clip_from_model,
            );
            gl.uniform_matrix_3_f32_slice(
                uniform("u_world_from_model").as_ref(),
                false,
                world_from_model,
            );
            for (unit, name, texture) in [
                (0, "u_base_color", self.base_color),
                (1, "u_metallic_roughness", self.metallic_roughness),
            ] {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.uniform_1_i32(uniform(name).as_ref(), unit.cast_signed());
            }

            gl.bind_vertex_array(Some(self.vertex_array));
            gl.draw_elements(glow::TRIANGLES, self.index_count, glow::UNSIGNED_INT, 0);
            gl.bind_vertex_array(None);
            gl.active_texture(glow::TEXTURE0);
            gl.disable(glow::DEPTH_TEST);
        }
    }
}

/// Paints the ship (the model, scaled so the nose is `half_length` world units from its
/// center) at `position`, heading `theta` radians counterclockwise from +x, into the canvas
/// `canvas` seen through `camera`. Returns `None` when the ship is outside the canvas.
pub fn paint_callback(
    renderer: &Arc<ShipRenderer>,
    camera: &Camera,
    canvas: egui::Rect,
    position: egui::Vec2,
    theta: f32,
    half_length: f32,
) -> Option<egui::PaintCallback> {
    // Only the ship's surroundings are drawn into.
    let center = camera.world_to_screen(position, canvas);
    let radius = half_length * camera.scale * 1.05;
    let rect =
        egui::Rect::from_center_size(center, egui::Vec2::splat(2.0 * radius)).intersect(canvas);
    if !rect.is_positive() {
        return None;
    }

    let scale = half_length / renderer.half_length;
    let world_from_model = rotation(theta);

    // Clip coordinates of world point (x, y, z), x and y through the camera onto the screen,
    // then into `rect`; z (toward the viewer) into depth, nearer being smaller.
    let k = camera.scale;
    let canvas_center = canvas.center();
    let (sx, sy) = (2.0 * k / rect.width(), 2.0 * k / rect.height());
    let tx = 2.0 * (canvas_center.x - camera.center.x * k - rect.center().x) / rect.width();
    let ty = -2.0 * (canvas_center.y + camera.center.y * k - rect.center().y) / rect.height();
    let sz = -1.0 / (2.0 * half_length);
    let clip_from_world = [
        [sx, 0.0, 0.0, 0.0],
        [0.0, sy, 0.0, 0.0],
        [0.0, 0.0, sz, 0.0],
        [tx, ty, 0.0, 1.0],
    ];
    let r = &world_from_model;
    let world_from_model4 = [
        [scale * r[0], scale * r[1], scale * r[2], 0.0],
        [scale * r[3], scale * r[4], scale * r[5], 0.0],
        [scale * r[6], scale * r[7], scale * r[8], 0.0],
        [position.x, position.y, 0.0, 1.0],
    ];
    let clip_from_model = mul4(&clip_from_world, &world_from_model4);

    let renderer = Arc::clone(renderer);
    Some(egui::PaintCallback {
        rect,
        callback: Arc::new(egui_glow::CallbackFn::new(move |_info, painter| {
            renderer.paint(painter.gl(), &clip_from_model, &world_from_model);
        })),
    })
}

/// The rotation (column-major 3x3) that turns the model's nose (+y) to heading `theta` in
/// the plane and its [`MODEL_TOP`] toward the viewer (+z).
fn rotation(theta: f32) -> [f32; 9] {
    let (sin, cos) = theta.sin_cos();
    let nose = [0.0, 1.0, 0.0];
    let side = cross(nose, MODEL_TOP);
    // Where the model's nose, top and side go.
    let heading = [cos, sin, 0.0];
    let up = [0.0, 0.0, 1.0];
    let world_side = cross(heading, up);
    // R = W M^T, with M = [nose top side] and W = [heading up world_side] as columns.
    let mut r = [0.0; 9];
    for col in 0..3 {
        for row in 0..3 {
            r[col * 3 + row] =
                heading[row] * nose[col] + up[row] * MODEL_TOP[col] + world_side[row] * side[col];
        }
    }
    r
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Product of column-major 4x4 matrices (`[column][row]`), flattened column-major.
fn mul4(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            out[col * 4 + row] = (0..4).map(|i| a[i][row] * b[col][i]).sum();
        }
    }
    out
}

fn to_bytes(floats: &[f32]) -> Vec<u8> {
    floats.iter().flat_map(|f| f.to_ne_bytes()).collect()
}

/// # Safety
/// The caller must have `gl` current.
unsafe fn compile_program(gl: &Context, version: &str) -> Result<glow::Program, String> {
    // SAFETY: forwarded from the caller.
    unsafe {
        let program = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, source) in [
            (glow::VERTEX_SHADER, VERTEX_SHADER),
            (glow::FRAGMENT_SHADER, FRAGMENT_SHADER),
        ] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, &format!("{version}{source}"));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(gl.get_shader_info_log(shader));
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        for (location, name) in [(0, "a_position"), (1, "a_normal"), (2, "a_uv")] {
            gl.bind_attrib_location(program, location, name);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(gl.get_program_info_log(program));
        }
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        Ok(program)
    }
}

/// # Safety
/// The caller must have `gl` current.
unsafe fn upload_texture(gl: &Context, image: &Image) -> Result<glow::Texture, String> {
    // SAFETY: forwarded from the caller.
    unsafe {
        let texture = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8.cast_signed(),
            image.width.cast_signed(),
            image.height.cast_signed(),
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&image.pixels)),
        );
        gl.generate_mipmap(glow::TEXTURE_2D);
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR_MIPMAP_LINEAR.cast_signed(),
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR.cast_signed(),
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
        Ok(texture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(r: &[f32; 9], v: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|row| (0..3).map(|col| r[col * 3 + row] * v[col]).sum())
    }

    fn assert_near(a: [f32; 3], b: [f32; 3]) {
        assert!(
            a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5),
            "{a:?} != {b:?}"
        );
    }

    #[test]
    fn rotation_points_nose_along_heading_and_top_at_viewer() {
        for theta in [0.0, 0.7, 2.0, -2.5] {
            let r = rotation(theta);
            assert_near(apply(&r, [0.0, 1.0, 0.0]), [theta.cos(), theta.sin(), 0.0]);
            assert_near(apply(&r, MODEL_TOP), [0.0, 0.0, 1.0]);
            // A proper rotation: no mirroring.
            let x = apply(&r, [1.0, 0.0, 0.0]);
            let y = apply(&r, [0.0, 1.0, 0.0]);
            assert_near(cross(x, y), apply(&r, [0.0, 0.0, 1.0]));
        }
    }

    #[test]
    fn loads_the_embedded_model() {
        let model = ShipModel::load(SHIP_GLB).expect("the embedded model loads");
        assert_eq!(model.vertices.len() % 8, 0);
        assert_eq!(model.indices.len() % 3, 0);
        assert!((model.half_length - 1.0).abs() < 0.01);
        assert_eq!(model.base_color.pixels.len(), 2048 * 2048 * 4);
    }
}
