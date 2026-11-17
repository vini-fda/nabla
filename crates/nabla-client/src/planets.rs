//! White circles in Debug and lit sphere meshes in 3D, sharing the ship's depth mapping.

use crate::camera::{Camera, Vec3};
use crate::ship3d::{DepthMap, clip_from_world, to_bytes};
use crate::world::CelestialBody;
use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use eframe::wgpu::{self, util::DeviceExt as _};

/// The sphere mesh's rings, pole to pole, and segments around: a UV sphere with poles along
/// the world's z axis. The outline a body leaves in the plane, where ships fly and land, is
/// its equator, a ring of `SEGMENTS` chords, which come within `1 - cos(PI / SEGMENTS)`,
/// about `2e-5`, of its radius: `0.007` units for a planet `350` across, a hair against a
/// ship's hull. Even, so that the equator is a ring of the mesh.
const RINGS: u32 = 256;
const SEGMENTS: u32 = 512;

struct Resources {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    /// The unit sphere's points, which are also its normals.
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    /// Each body's center in the plane and radius.
    instances: wgpu::Buffer,
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
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![1 => Float32x3],
                }),
            ],
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
    let (vertices, indices, index_count) = sphere_buffers(device);
    let instances = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("no planets"),
        size: 12,
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
            indices,
            index_count,
            instances,
            count: 0,
            bodies: Vec::new(),
        });
}

/// The unit sphere's vertex and index buffers, and how many indices it draws.
fn sphere_buffers(device: &wgpu::Device) -> (wgpu::Buffer, wgpu::Buffer, u32) {
    let (points, triangles) = unit_sphere();
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("planet mesh"),
        contents: &to_bytes(&points),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("planet mesh indices"),
        contents: &triangles
            .iter()
            .flat_map(|i| i.to_ne_bytes())
            .collect::<Vec<_>>(),
        usage: wgpu::BufferUsages::INDEX,
    });
    (vertices, indices, triangles.len() as u32)
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
    let matrix = clip_from_world(camera, rect, depth);
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

/// The unit sphere: its points, which are also its normals, as `x, y, z` runs, and its
/// triangles, as runs of three indices into them. Uploaded once; each body draws it scaled
/// by its radius about its center.
fn unit_sphere() -> (Vec<f32>, Vec<u32>) {
    let mut points = Vec::with_capacity(((RINGS + 1) * (SEGMENTS + 1) * 3) as usize);
    for ring in 0..=RINGS {
        let (sin_lat, cos_lat) = (std::f32::consts::PI * ring as f32 / RINGS as f32).sin_cos();
        for segment in 0..=SEGMENTS {
            let (sin_lon, cos_lon) =
                (std::f32::consts::TAU * segment as f32 / SEGMENTS as f32).sin_cos();
            points.extend([sin_lat * cos_lon, sin_lat * sin_lon, cos_lat]);
        }
    }
    let index = |ring: u32, segment: u32| ring * (SEGMENTS + 1) + segment;
    let mut triangles = Vec::with_capacity((RINGS * SEGMENTS * 6) as usize);
    for ring in 0..RINGS {
        for segment in 0..SEGMENTS {
            triangles.extend([
                index(ring, segment),
                index(ring + 1, segment),
                index(ring, segment + 1),
                index(ring, segment + 1),
                index(ring + 1, segment),
                index(ring + 1, segment + 1),
            ]);
        }
    }
    (points, triangles)
}

/// Each body's center in the plane and radius, as `x, y, radius` runs.
fn instances(bodies: &[CelestialBody]) -> Vec<f32> {
    bodies
        .iter()
        .flat_map(|body| [body.position[0], body.position[1], body.radius])
        .collect()
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
                let instances = instances(&self.bodies);
                res.count = self.bodies.len() as u32;
                if !instances.is_empty() {
                    res.instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("planets"),
                        contents: &to_bytes(&instances),
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
            pass.set_vertex_buffer(1, res.instances.slice(..));
            pass.set_index_buffer(res.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..res.index_count, 0, 0..res.count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mesh is a unit sphere whose every edge, the cells' diagonals among them, stays
    /// within `4e-5` of it, and whose outline in the plane stays within `2e-5`: the 32 by 64
    /// mesh before let it come within `1.2e-3`, half a ship's size for a planet `350` across,
    /// so its outline showed as straight runs.
    #[test]
    fn the_sphere_mesh_hugs_the_sphere() {
        let (points, triangles) = unit_sphere();
        let point = |i: u32| {
            let i = i as usize * 3;
            Vec3::new(points[i], points[i + 1], points[i + 2])
        };
        for i in 0..(points.len() / 3) as u32 {
            assert!((point(i).length() - 1.0).abs() < 1e-6);
        }
        assert_eq!(triangles.len() as u32, RINGS * SEGMENTS * 6);
        let mut sag = 0.0_f32;
        for triangle in triangles.as_chunks::<3>().0 {
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                let middle = (point(triangle[a]) + point(triangle[b])) * 0.5;
                sag = sag.max(1.0 - middle.length());
            }
        }
        assert!(sag < 4e-5, "sag {sag}");
        // The equator is a ring of the mesh, in the plane, its chords within `2e-5`.
        let equator = (RINGS / 2) * (SEGMENTS + 1);
        for segment in 0..SEGMENTS {
            let (a, b) = (point(equator + segment), point(equator + segment + 1));
            assert!(a.z.abs() < 1e-6);
            assert!(1.0 - ((a + b) * 0.5).length() < 2e-5);
        }
    }
}
