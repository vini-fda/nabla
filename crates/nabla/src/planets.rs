//! White circles in Debug and lit sphere meshes in 3D, sharing the ship's depth mapping.

use crate::camera::{Camera, Vec3};
use crate::ship3d::{DepthMap, clip_from_world, to_bytes};
use crate::world::CelestialBody;
use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use eframe::wgpu::{self, util::DeviceExt as _};

struct Resources {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    vertices: wgpu::Buffer,
    count: u32,
    bodies: Vec<CelestialBody>,
}

pub fn init(render: &egui_wgpu::RenderState, samples: u32, depth: Option<wgpu::TextureFormat>) {
    let device = &render.device;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("planets"),
        source: wgpu::ShaderSource::Wgsl(include_str!("planets.wgsl").into()),
    });
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("planet uniforms"),
        size: 80,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("planets"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(80),
            },
            count: None,
        }],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("planets"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniforms.as_entire_binding(),
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("planets"),
        bind_group_layouts: &[Some(&layout)],
        ..Default::default()
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("planets"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 24,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
            })],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: depth.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(render.target_format.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    let vertices = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("empty planet mesh"),
        size: 24,
        usage: wgpu::BufferUsages::VERTEX,
        mapped_at_creation: false,
    });
    render
        .renderer
        .write()
        .callback_resources
        .insert(Resources {
            pipeline,
            bind_group,
            uniforms,
            vertices,
            count: 0,
            bodies: Vec::new(),
        });
}

pub fn draw_debug(
    painter: &egui::Painter,
    camera: &Camera,
    rect: egui::Rect,
    bodies: &[CelestialBody],
) {
    draw_circles(
        painter,
        camera,
        rect,
        bodies
            .iter()
            .map(|body| (egui::vec2(body.position[0], body.position[1]), body.radius)),
        egui::Stroke::new(2.0, egui::Color32::WHITE),
    );
}

/// Project circular shapes in the simulation plane, including camera near-plane clipping.
pub fn draw_circles(
    painter: &egui::Painter,
    camera: &Camera,
    rect: egui::Rect,
    circles: impl IntoIterator<Item = (egui::Vec2, f32)>,
    stroke: egui::Stroke,
) {
    for (center, radius) in circles {
        let points = (0..=128).map(|i| {
            let angle = i as f32 * std::f32::consts::TAU / 128.0;
            Vec3::plane(center + egui::Vec2::angled(angle) * radius)
        });
        for run in camera.project_polyline(points, rect) {
            painter.add(egui::Shape::line(run, stroke));
        }
    }
}

struct Callback {
    bodies: Vec<CelestialBody>,
    uniforms: Vec<u8>,
}

pub fn paint(
    camera: &Camera,
    rect: egui::Rect,
    depth: &DepthMap,
    bodies: &[CelestialBody],
    gamma: bool,
) -> egui::PaintCallback {
    let matrix = clip_from_world(camera, rect, rect, depth);
    let mut uniforms = to_bytes(&matrix.into_iter().flatten().collect::<Vec<_>>());
    uniforms.extend(to_bytes(&[f32::from(u8::from(gamma)), 0.0, 0.0, 0.0]));
    egui_wgpu::Callback::new_paint_callback(
        rect,
        Callback {
            bodies: bodies.to_vec(),
            uniforms,
        },
    )
}

/// Positions and radial normals; uploaded only when a world's bodies change.
fn mesh(bodies: &[CelestialBody]) -> Vec<f32> {
    const RINGS: usize = 32;
    const SEGMENTS: usize = 64;
    let mut vertices = Vec::with_capacity(bodies.len() * RINGS * SEGMENTS * 36);
    for body in bodies {
        for ring in 0..RINGS {
            for segment in 0..SEGMENTS {
                for (r, s) in [
                    (ring, segment),
                    (ring + 1, segment),
                    (ring, segment + 1),
                    (ring, segment + 1),
                    (ring + 1, segment),
                    (ring + 1, segment + 1),
                ] {
                    let latitude = std::f32::consts::PI * r as f32 / RINGS as f32;
                    let longitude = std::f32::consts::TAU * s as f32 / SEGMENTS as f32;
                    let n = [
                        latitude.sin() * longitude.cos(),
                        latitude.sin() * longitude.sin(),
                        latitude.cos(),
                    ];
                    vertices.extend([
                        body.position[0] + body.radius * n[0],
                        body.position[1] + body.radius * n[1],
                        body.radius * n[2],
                        n[0],
                        n[1],
                        n[2],
                    ]);
                }
            }
        }
    }
    vertices
}

impl CallbackTrait for Callback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(res) = resources.get_mut::<Resources>() {
            if res.bodies != self.bodies {
                let vertices = mesh(&self.bodies);
                res.count = (vertices.len() / 6) as u32;
                if !vertices.is_empty() {
                    res.vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("planet mesh"),
                        contents: &to_bytes(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                }
                res.bodies.clone_from(&self.bodies);
            }
            queue.write_buffer(&res.uniforms, 0, &self.uniforms);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        if let Some(res) = resources.get::<Resources>() {
            pass.set_pipeline(&res.pipeline);
            pass.set_bind_group(0, &res.bind_group, &[]);
            pass.set_vertex_buffer(0, res.vertices.slice(..));
            pass.draw(0..res.count, 0..1);
        }
    }
}
