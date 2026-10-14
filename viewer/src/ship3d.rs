//! The ship's textured 3D model, and a wgpu renderer that draws it into the canvas (through
//! egui's own render pass, with its depth buffer), seen from straight above with an
//! orthographic projection that matches the 2D [`Camera`], so the model lines up with
//! everything the canvas paints around it. The shader is `ship3d.wgsl`.
//!
//! The model is `assets/nabla_starship.glb`: one mesh with positions, normals and
//! texture coordinates, a base color texture (2048 px JPEG) and a metallic-roughness
//! texture (1024 px JPEG). In model space the ship's nose points along +y.

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use eframe::wgpu::{self, util::DeviceExt as _};

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
#[derive(Clone)]
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
const VERTEX_STRIDE: u64 = 8 * 4;

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

/// The model's GPU resources, kept in egui-wgpu's callback resources.
struct ShipResources {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

/// Size of the uniform block in `ship3d.wgsl`: two 4x4 matrices and a vec4.
const UNIFORMS_SIZE: u64 = (16 + 16 + 4) * 4;

/// Draws the model; its GPU resources live in the egui renderer's callback resources.
pub struct ShipRenderer {
    half_length: f32,
    /// Gamma-encode the shader's output, for a target that is not sRGB.
    gamma_output: bool,
}

impl ShipRenderer {
    /// Uploads `model`. `msaa_samples` and `depth_format` must be those of egui's render
    /// pass, into which the ship is drawn.
    pub fn new(
        render_state: &egui_wgpu::RenderState,
        model: &ShipModel,
        msaa_samples: u32,
        depth_format: Option<wgpu::TextureFormat>,
    ) -> Self {
        let device = &render_state.device;
        let queue = &render_state.queue;

        let (pipeline, bind_group_layout) = create_pipeline(
            device,
            render_state.target_format,
            msaa_samples,
            depth_format,
        );

        let buffer = |label, contents: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
        };
        let vertices = buffer(
            "ship3d vertices",
            &to_bytes(&model.vertices),
            wgpu::BufferUsages::VERTEX,
        );
        let index_bytes: Vec<u8> = model.indices.iter().flat_map(|i| i.to_ne_bytes()).collect();
        let indices = buffer("ship3d indices", &index_bytes, wgpu::BufferUsages::INDEX);
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ship3d uniforms"),
            size: UNIFORMS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let base_color = upload_texture(
            device,
            queue,
            &model.base_color,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let metallic_roughness = upload_texture(
            device,
            queue,
            &model.metallic_roughness,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ship3d"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ship3d"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&base_color),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&metallic_roughness),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        render_state
            .renderer
            .write()
            .callback_resources
            .insert(ShipResources {
                pipeline,
                bind_group,
                uniforms,
                vertices,
                indices,
                index_count: model.indices.len() as u32,
            });
        Self {
            half_length: model.half_length,
            gamma_output: !render_state.target_format.is_srgb(),
        }
    }
}

/// The render pipeline of `ship3d.wgsl` for egui's render pass (`target_format`,
/// `msaa_samples` and `depth_format`), and the layout of its bind group.
fn create_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    msaa_samples: u32,
    depth_format: Option<wgpu::TextureFormat>,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ship3d"),
        source: wgpu::ShaderSource::Wgsl(include_str!("ship3d.wgsl").into()),
    });
    let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ship3d"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(UNIFORMS_SIZE),
                },
                count: None,
            },
            texture_entry(1),
            texture_entry(2),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ship3d"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        ..Default::default()
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("ship3d"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: VERTEX_STRIDE,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x3, 1 => Float32x3, 2 => Float32x2
                ],
            })],
        },
        // The model is double-sided.
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: msaa_samples,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(target_format.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    (pipeline, bind_group_layout)
}

/// One frame's drawing of the ship: the uniforms to upload before drawing.
struct ShipCallback {
    uniforms: Vec<u8>,
}

impl CallbackTrait for ShipCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(ship) = resources.get::<ShipResources>() {
            queue.write_buffer(&ship.uniforms, 0, &self.uniforms);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(ship) = resources.get::<ShipResources>() else {
            return;
        };
        render_pass.set_pipeline(&ship.pipeline);
        render_pass.set_bind_group(0, &ship.bind_group, &[]);
        render_pass.set_vertex_buffer(0, ship.vertices.slice(..));
        render_pass.set_index_buffer(ship.indices.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..ship.index_count, 0, 0..1);
    }
}

/// Paints the ship (the model, scaled so the nose is `half_length` world units from its
/// center) at `position`, heading `theta` radians counterclockwise from +x, into the canvas
/// `canvas` seen through `camera`. Returns `None` when the ship is outside the canvas.
pub fn paint_callback(
    renderer: &ShipRenderer,
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
    let r = rotation(theta);

    // Clip coordinates of world point (x, y, z), x and y through the camera onto the screen,
    // then into `rect`; z (toward the viewer) into depth (0 to 1), nearer being smaller.
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
        [tx, ty, 0.5, 1.0],
    ];
    let world_from_model = [
        [scale * r[0], scale * r[1], scale * r[2], 0.0],
        [scale * r[3], scale * r[4], scale * r[5], 0.0],
        [scale * r[6], scale * r[7], scale * r[8], 0.0],
        [position.x, position.y, 0.0, 1.0],
    ];
    let rotation4 = [
        r[0], r[1], r[2], 0.0, r[3], r[4], r[5], 0.0, r[6], r[7], r[8], 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let output = [f32::from(u8::from(renderer.gamma_output)), 0.0, 0.0, 0.0];

    let mut uniforms = to_bytes(&mul4(&clip_from_world, &world_from_model));
    uniforms.extend(to_bytes(&rotation4));
    uniforms.extend(to_bytes(&output));
    Some(egui_wgpu::Callback::new_paint_callback(
        rect,
        ShipCallback { uniforms },
    ))
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

/// Uploads `image` as a texture of `format` with a full chain of mipmaps (wgpu doesn't make
/// them), and returns its view.
fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &Image,
    format: wgpu::TextureFormat,
) -> wgpu::TextureView {
    let levels = mipmaps(image);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ship3d"),
        size: wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, mip) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &mip.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * mip.width),
                rows_per_image: Some(mip.height),
            },
            wgpu::Extent3d {
                width: mip.width,
                height: mip.height,
                depth_or_array_layers: 1,
            },
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// `image` and its successive halvings down to 1x1, each pixel the average of the 2x2 above.
fn mipmaps(image: &Image) -> Vec<Image> {
    let mut levels = vec![image.clone()];
    while let Some(last) = levels.last().filter(|i| i.width > 1 || i.height > 1) {
        let (width, height) = ((last.width / 2).max(1), (last.height / 2).max(1));
        let at = |x: u32, y: u32, c: usize| {
            let (x, y) = (x.min(last.width - 1), y.min(last.height - 1));
            u32::from(last.pixels[((y * last.width + x) * 4) as usize + c])
        };
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                for c in 0..4 {
                    let sum = at(2 * x, 2 * y, c)
                        + at(2 * x + 1, 2 * y, c)
                        + at(2 * x, 2 * y + 1, c)
                        + at(2 * x + 1, 2 * y + 1, c);
                    pixels.push(((sum + 2) / 4) as u8);
                }
            }
        }
        levels.push(Image {
            width,
            height,
            pixels,
        });
    }
    levels
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

    #[test]
    fn mipmaps_halve_down_to_one_pixel() {
        let image = Image {
            width: 4,
            height: 2,
            pixels: (0..4 * 2 * 4).map(|i| (i * 8) as u8).collect(),
        };
        let levels = mipmaps(&image);
        let sizes: Vec<_> = levels.iter().map(|l| (l.width, l.height)).collect();
        assert_eq!(sizes, [(4, 2), (2, 1), (1, 1)]);
        // Red of the first 2x2 block: pixels 0, 1, 4 and 5, red values 0, 32, 128, 160.
        assert_eq!(levels[1].pixels[0], 80);
    }
}
