//! Soft additive sprites (the thrusters' plumes), drawn with wgpu into egui's render pass
//! after the 3D ship and depth tested against it. A sprite then shows in front of the hull
//! where it is nearer the viewer and behind it where it is farther, from any side; egui's
//! own shapes have no depth, so painted before the ship they would always be under it, and
//! painted after, always over it. The shader is `sprites.wgsl`.
//!
//! Sprites blend as egui blends its shapes (premultiplied alpha; a color with zero alpha
//! adds onto what is below), so they look the same as they would as an egui mesh.

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use eframe::wgpu;

use crate::ship3d::{Image, to_bytes, upload_texture};

/// Floats per vertex: position (clip x, y and depth), texture coordinates, color (RGBA, 0 to
/// 1, gamma encoded as in egui).
const VERTEX_FLOATS: usize = 3 + 2 + 4;

/// Sprites to draw, built on the CPU each frame.
#[derive(Default)]
pub struct SpriteBatch {
    /// Vertices in screen points (x, y), then depth, texture coordinates and color.
    vertices: Vec<[f32; VERTEX_FLOATS]>,
    indices: Vec<u32>,
}

impl SpriteBatch {
    /// A sprite centered on `center` (screen points) with half axes `a` and `b`, at `depth`
    /// (0 nearest to 1 farthest, as the ship's depth).
    pub fn add(
        &mut self,
        center: egui::Pos2,
        a: egui::Vec2,
        b: egui::Vec2,
        depth: f32,
        color: egui::Color32,
    ) {
        let base = self.vertices.len() as u32;
        let [r, g, bl, al] = color.to_array().map(|c| f32::from(c) / 255.0);
        for (corner, [u, v]) in [
            (-a - b, [0.0, 0.0]),
            (a - b, [1.0, 0.0]),
            (a + b, [1.0, 1.0]),
            (-a + b, [0.0, 1.0]),
        ] {
            let p = center + corner;
            self.vertices.push([p.x, p.y, depth, u, v, r, g, bl, al]);
        }
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// The sprites' GPU resources, kept in egui-wgpu's callback resources.
struct SpriteResources {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    /// Grown as needed.
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
}

/// Draws sprite batches; its GPU resources live in the egui renderer's callback resources.
pub struct SpriteRenderer;

impl SpriteRenderer {
    /// Uploads `texture`, the one image every sprite shows. `msaa_samples` and
    /// `depth_format` must be those of egui's render pass, as for the ship.
    pub fn new(
        render_state: &egui_wgpu::RenderState,
        texture: &Image,
        msaa_samples: u32,
        depth_format: Option<wgpu::TextureFormat>,
    ) -> Self {
        let device = &render_state.device;
        let (pipeline, bind_group_layout) = create_pipeline(
            device,
            render_state.target_format,
            msaa_samples,
            depth_format,
        );

        // Not sRGB: sampled as is, gamma encoded, as egui samples its textures.
        let view = upload_texture(
            device,
            &render_state.queue,
            texture,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sprites"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sprites"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        render_state
            .renderer
            .write()
            .callback_resources
            .insert(SpriteResources {
                pipeline,
                bind_group,
                vertices: vertex_buffer(device, 0),
                indices: index_buffer(device, 0),
            });
        Self
    }

    /// Paints `batch` into `canvas`, after (over) what is already painted there, the ship
    /// included. Returns `None` when there is nothing to paint.
    #[expect(
        clippy::unused_self,
        reason = "a renderer proves `new` has set up the GPU resources the callback draws with"
    )]
    pub fn paint_callback(
        &self,
        canvas: egui::Rect,
        batch: SpriteBatch,
    ) -> Option<egui::PaintCallback> {
        if batch.is_empty() || !canvas.is_positive() {
            return None;
        }
        // Screen points to clip coordinates in `canvas`, the callback's viewport.
        let vertices: Vec<f32> = batch
            .vertices
            .into_iter()
            .flat_map(|mut v| {
                v[0] = 2.0 * (v[0] - canvas.left()) / canvas.width() - 1.0;
                v[1] = 1.0 - 2.0 * (v[1] - canvas.top()) / canvas.height();
                v
            })
            .collect();
        Some(egui_wgpu::Callback::new_paint_callback(
            canvas,
            SpriteCallback {
                vertices: to_bytes(&vertices),
                indices: batch.indices.iter().flat_map(|i| i.to_ne_bytes()).collect(),
                index_count: batch.indices.len() as u32,
            },
        ))
    }
}

/// The render pipeline of `sprites.wgsl` for egui's render pass (`target_format`,
/// `msaa_samples` and `depth_format`), and the layout of its bind group.
fn create_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    msaa_samples: u32,
    depth_format: Option<wgpu::TextureFormat>,
) -> (wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("sprites"),
        source: wgpu::ShaderSource::Wgsl(include_str!("sprites.wgsl").into()),
    });
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sprites"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sprites"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        ..Default::default()
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sprites"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: (VERTEX_FLOATS * 4) as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x3, 1 => Float32x2, 2 => Float32x4
                ],
            })],
        },
        primitive: wgpu::PrimitiveState::default(),
        // Tested against the ship, but not written: sprites don't hide each other.
        // Equal passes, so sprites farther than the ship's reach (depth 1, as cleared)
        // show wherever the ship isn't.
        depth_stencil: depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: msaa_samples,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            // As egui's own shader picks its output.
            entry_point: Some(if target_format.is_srgb() {
                "fs_linear_framebuffer"
            } else {
                "fs_gamma_framebuffer"
            }),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            // egui's blending.
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    (pipeline, bind_group_layout)
}

fn vertex_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sprite vertices"),
        size: size.max(1024),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn index_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sprite indices"),
        size: size.max(1024),
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// One frame's sprites, uploaded before drawing.
struct SpriteCallback {
    vertices: Vec<u8>,
    indices: Vec<u8>,
    index_count: u32,
}

impl CallbackTrait for SpriteCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(sprites) = resources.get_mut::<SpriteResources>() {
            // Grow the buffers to the next power of two that holds this frame's sprites.
            let (vertex_size, index_size) = (self.vertices.len() as u64, self.indices.len() as u64);
            if sprites.vertices.size() < vertex_size {
                sprites.vertices = vertex_buffer(device, vertex_size.next_power_of_two());
            }
            if sprites.indices.size() < index_size {
                sprites.indices = index_buffer(device, index_size.next_power_of_two());
            }
            queue.write_buffer(&sprites.vertices, 0, &self.vertices);
            queue.write_buffer(&sprites.indices, 0, &self.indices);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(sprites) = resources.get::<SpriteResources>() else {
            return;
        };
        render_pass.set_pipeline(&sprites.pipeline);
        render_pass.set_bind_group(0, &sprites.bind_group, &[]);
        render_pass.set_vertex_buffer(0, sprites.vertices.slice(..));
        render_pass.set_index_buffer(sprites.indices.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}
