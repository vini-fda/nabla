//! The gravity field of the world's bodies, as two overlays on the plane `z = 0`:
//!
//! - soft bands of the log-potential, drawn by a wgpu pass (`field.wgsl`) under everything
//!   else on the canvas, and
//! - crisp equipotential contour lines, extracted in world space by marching squares and
//!   chained into polylines: per-pixel bands alias where the field changes within one
//!   pixel (near a body, zoomed out), while world-space lines stay sub-cell accurate at any
//!   zoom. In the flat 2D view they are drawn as plain egui strokes; in the 3D view, as
//!   the same stroke by a wgpu pass that tests them against the depth of the planets and
//!   ships drawn before them, so what stands in front of the plane hides them.
//!
//! Both use the potential whose negative gradient is the simulation's acceleration
//! (`nabla_sim::physics::gravity_acceleration`): `-G M / r` outside a body, and the uniform
//! solid sphere's `-G M (3 R^2 - r^2) / (2 R^3)` inside it, continuous at the surface and
//! zero at the center.

use std::collections::HashMap;

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use eframe::wgpu::{self, util::DeviceExt as _};
use egui::{Painter, Rect, Shape, Stroke, Vec2};

use crate::camera::{Camera, Orbit, Vec3};
use crate::ship3d::{DepthMap, clip_from_world, to_bytes};
use crate::world::{CelestialBody, World};

/// The ratio between adjacent contour lines' potentials (and the shader's bands): the
/// potential magnitude of each line is `1 / sqrt(2)` of the one inside it. The lines sit at
/// the world-fixed potentials `-LEVEL_BASE^n`, exactly where `field.wgsl`'s band phase
/// wraps, so lines and bands share the world itself as their anchor: neither moves as the
/// view pans, zooms or recomputes.
pub const LEVEL_BASE: f64 = std::f64::consts::SQRT_2;

/// How squarely the plane must face the viewer for the field to be drawn at all, and at full
/// strength, as for the grid. Below it the plane is edge on and the field unreadable.
pub(crate) const MIN_FACING: f32 = 0.05;
pub(crate) const FULL_FACING: f32 = 0.30;

/// How many cells span the view's longer side, roughly, and the bounds on the grid's side.
const TARGET_CELLS: f64 = 256.0;
const MIN_CELLS: usize = 16;
const MAX_CELLS: usize = 800;

// ---------------------------------------------------------------------------------------------
// The wgpu pass: soft log-potential bands over the plane.

struct Resources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    bodies_buf: wgpu::Buffer,
    bodies_key: Vec<CelestialBody>,
    uniforms: wgpu::Buffer,
    quad: wgpu::Buffer,
}

pub fn init(render: &egui_wgpu::RenderState, samples: u32, depth: Option<wgpu::TextureFormat>) {
    let device = &render.device;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gravity field"),
        source: wgpu::ShaderSource::Wgsl(include_str!("field.wgsl").into()),
    });
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gravity field uniforms"),
        size: 80,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let quad = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gravity field quad"),
        size: 6 * 12,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bodies_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gravity field bodies"),
        size: 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("gravity field"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(80),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let bind_group = bind_group(device, &layout, &uniforms, &bodies_buf);
    let pipeline = pipeline(device, &shader, &layout, render, samples, depth);
    let lines = LinesResources::new(device, &shader, render, samples, depth);
    render
        .renderer
        .write()
        .callback_resources
        .insert(Resources {
            pipeline,
            layout,
            bind_group,
            bodies_buf,
            bodies_key: Vec::new(),
            uniforms,
            quad,
        });
    render.renderer.write().callback_resources.insert(lines);
}

/// The banding pass's pipeline: a world-space quad on the plane `z = 0`, blended as egui
/// blends its own shapes, depth-tested but not written, so the field is under everything
/// drawn after it.
fn pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    render: &egui_wgpu::RenderState,
    samples: u32,
    depth: Option<wgpu::TextureFormat>,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("gravity field"),
        bind_group_layouts: &[Some(layout)],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("gravity field"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 12,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3],
            })],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: depth.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            // As egui's own shader picks its output.
            entry_point: Some(if render.target_format.is_srgb() {
                "fs_linear_framebuffer"
            } else {
                "fs_gamma_framebuffer"
            }),
            compilation_options: Default::default(),
            // egui's blending, with premultiplied alpha as the shader outputs.
            targets: &[Some(wgpu::ColorTargetState {
                format: render.target_format,
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
    })
}

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    bodies: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gravity field"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: bodies.as_entire_binding(),
            },
        ],
    })
}

struct Callback {
    bodies: Vec<CelestialBody>,
    uniforms: Vec<u8>,
    quad: Vec<f32>,
}

/// The paint callback of the potential bands over the part of the plane `z = 0` in view, or
/// `None` when there is nothing to show: no gravity, or the plane edge on.
pub fn paint(
    camera: &Camera,
    rect: Rect,
    bodies: &[CelestialBody],
    gravity: f64,
) -> Option<egui::PaintCallback> {
    if bodies.is_empty() || gravity == 0.0 {
        return None;
    }
    let facing = camera.orbit.toward_viewer().z.abs();
    if facing < MIN_FACING {
        return None;
    }
    let (min, max) = region(camera, rect)?;
    let span = (max[0] - min[0]).max(max[1] - min[1]);
    let depth = DepthMap::for_camera(camera, rect, (0.75 * span) as f32);
    let matrix = clip_from_world(camera, rect, &depth);
    let fade = ((facing - MIN_FACING) / (FULL_FACING - MIN_FACING)).min(1.0);
    let mut uniforms = to_bytes(&matrix.into_iter().flatten().collect::<Vec<_>>());
    uniforms.extend(to_bytes(&[gravity as f32, bodies.len() as f32, fade, 0.0]));
    // Two triangles covering the region square on the plane.
    let (cx, cy) = (
        f64::midpoint(min[0], max[0]) as f32,
        f64::midpoint(min[1], max[1]) as f32,
    );
    let (hx, hy) = (
        ((max[0] - min[0]) / 2.0) as f32,
        ((max[1] - min[1]) / 2.0) as f32,
    );
    let quad = [
        [cx - hx, cy - hy, 0.0],
        [cx + hx, cy - hy, 0.0],
        [cx + hx, cy + hy, 0.0],
        [cx - hx, cy - hy, 0.0],
        [cx + hx, cy + hy, 0.0],
        [cx - hx, cy + hy, 0.0],
    ]
    .into_iter()
    .flatten()
    .collect();
    Some(egui_wgpu::Callback::new_paint_callback(
        rect,
        Callback {
            bodies: bodies.to_vec(),
            uniforms,
            quad,
        },
    ))
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
            // Upload the bodies only when the world's change.
            if res.bodies_key != self.bodies {
                let floats: Vec<f32> = self
                    .bodies
                    .iter()
                    .flat_map(|b| [b.position[0], b.position[1], b.radius, b.mass as f32])
                    .collect();
                res.bodies_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("gravity field bodies"),
                    contents: &to_bytes(&floats),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                });
                res.bind_group = bind_group(device, &res.layout, &res.uniforms, &res.bodies_buf);
                res.bodies_key.clone_from(&self.bodies);
            }
            queue.write_buffer(&res.uniforms, 0, &self.uniforms);
            queue.write_buffer(&res.quad, 0, &to_bytes(&self.quad));
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
            pass.set_vertex_buffer(0, res.quad.slice(..));
            pass.draw(0..6, 0..1);
        }
    }
}

// -----------------------------------------------------------------------------------------
// The wgpu pass for the contour lines, drawn over the bands and depth-tested against the
// planets and ships that drew before them.

/// The contour lines pass's pipeline and buffers.
struct LinesResources {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    /// The segments' quads, regrown whenever a frame no longer fits.
    vertices: wgpu::Buffer,
    capacity: u64,
    /// The last frame's bound vertices and their bytes.
    count: u32,
    bound: u64,
}

impl LinesResources {
    fn new(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        render: &egui_wgpu::RenderState,
        samples: u32,
        depth: Option<wgpu::TextureFormat>,
    ) -> Self {
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gravity field line uniforms"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gravity field lines"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(32),
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gravity field lines"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        Self {
            pipeline: lines_pipeline(device, shader, &layout, render, samples, depth),
            bind_group,
            uniforms,
            vertices: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gravity field line vertices"),
                size: 4,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            capacity: 4,
            count: 0,
            bound: 4,
        }
    }
}

/// The lines pass's pipeline: like the bands', but each vertex one of a ribbon's
/// (`crate::ribbon`), in device pixels with the plane's depth, carrying its distance across
/// the ribbon, which the fragment shader fences itself in by.
fn lines_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    render: &egui_wgpu::RenderState,
    samples: u32,
    depth: Option<wgpu::TextureFormat>,
) -> wgpu::RenderPipeline {
    // Group 0: the bands' uniform and bodies; group 2: the lines' own uniform.
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("gravity field lines"),
        bind_group_layouts: &[None, None, Some(layout)],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("gravity field lines"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("lines_vs"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: LINE_VERTEX_BYTES,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32],
            })],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: depth.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            // As egui's own shader picks its output, and the bands' above.
            entry_point: Some(if render.target_format.is_srgb() {
                "lines_fs_linear_framebuffer"
            } else {
                "lines_fs_gamma_framebuffer"
            }),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: render.target_format,
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
    })
}

/// Floats and bytes per vertex of the lines pass: `LinesVertexIn` of `field.wgsl`.
const LINE_VERTEX_FLOATS: usize = 4;
const LINE_VERTEX_BYTES: u64 = 4 * LINE_VERTEX_FLOATS as u64;

/// The paint callback of one frame's contour lines in the 3D view: the chains' ribbons in the
/// canvas's device pixels, each vertex with the depth the plane lies at there and its
/// distance across the ribbon, for the fragment distance fence.
struct LinesCallback {
    uniforms: Vec<u8>,
    vertices: Vec<f32>,
}

impl CallbackTrait for LinesCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(res) = resources.get_mut::<LinesResources>() {
            queue.write_buffer(&res.uniforms, 0, &self.uniforms);
            let bytes = to_bytes(&self.vertices);
            res.count = (bytes.len() as u64 / LINE_VERTEX_BYTES) as u32;
            res.bound = bytes.len() as u64;
            if res.bound > res.capacity {
                res.capacity = res.bound.next_power_of_two();
                res.vertices = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gravity field line vertices"),
                    size: res.capacity,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            queue.write_buffer(&res.vertices, 0, &bytes);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        if let Some(res) = resources.get::<LinesResources>() {
            pass.set_pipeline(&res.pipeline);
            pass.set_bind_group(2, &res.bind_group, &[]);
            pass.set_vertex_buffer(0, res.vertices.slice(0..res.bound));
            pass.draw(0..res.count, 0..1);
        }
    }
}

/// The contour lines' stroke in either view, `width` points thick, fading out as the view
/// turns edge on to the plane; `None` once it has.
fn stroke(camera: &Camera, width: f32, visuals: &egui::Visuals) -> Option<Stroke> {
    let facing = camera.orbit.toward_viewer().z.abs();
    if facing < MIN_FACING {
        return None;
    }
    let fade = ((facing - MIN_FACING) / (FULL_FACING - MIN_FACING)).min(1.0);
    Some(Stroke::new(
        width,
        visuals.text_color().gamma_multiply(0.4 * fade),
    ))
}

/// A chain's points, and whether it is a ring: one that closes back on its first point, which
/// is then left off its end.
fn ring(chain: &[[f32; 2]]) -> (bool, &[[f32; 2]]) {
    match (chain.first(), chain.last()) {
        (Some(first), Some(last)) if chain.len() > 2 && first == last => {
            (true, &chain[..chain.len() - 1])
        }
        _ => (false, chain),
    }
}

/// The contour lines on the canvas in the 3D view, from `cache` (`contours`, warmed by the
/// caller) as a paint callback to add after the planets and ships: drawn over the bands,
/// depth-tested against the 3D content through the scene's own `depth` map, so what stands
/// in front of the plane hides them. They are the stroke `draw_contours` gives egui in the
/// flat view, the same color and `width` points thick, with the same pixel of anti-aliasing.
/// The callback is recreated every frame, as the view's device pixels move.
pub fn paint_lines(
    camera: &Camera,
    rect: Rect,
    cache: &Option<Contours>,
    width: f32,
    depth: &DepthMap,
    pixels_per_point: f32,
    visuals: &egui::Visuals,
) -> Option<egui::PaintCallback> {
    let stroke = stroke(camera, width, visuals)?;
    let (vertices, uniforms) = line_vertices(
        camera,
        rect,
        cache.as_ref()?,
        stroke,
        depth,
        pixels_per_point,
    )?;
    Some(egui_wgpu::Callback::new_paint_callback(
        rect,
        LinesCallback {
            uniforms: to_bytes(&uniforms),
            vertices,
        },
    ))
}

/// What `paint_lines` draws: the vertices of the chains' ribbons, and the floats of the
/// shader's uniforms. Each vertex is its place in the canvas's device pixels, the depth the
/// plane lies at there, and its distance across the ribbon.
fn line_vertices(
    camera: &Camera,
    rect: Rect,
    contours: &Contours,
    stroke: Stroke,
    depth: &DepthMap,
    pixels_per_point: f32,
) -> Option<(Vec<f32>, [f32; 8])> {
    // In device pixels: half the stroke, and one pixel of feathering centered on its edge,
    // as egui feathers its own strokes. The ribbon covers both.
    let half = 0.5 * stroke.width * pixels_per_point;
    let feather = 1.0;
    let reach = half + 0.5 * feather;
    let on_screen = rect.expand(stroke.width + 8.0);
    let mut vertices: Vec<f32> = Vec::new();
    for chain in &contours.chains {
        let (closed, points) = ring(chain);
        let count = points.len();
        let points = points.iter().map(|p| Vec3::plane(Vec2::from(*p)));
        let runs = camera.project_polyline_with_world(points, rect);
        // A ring is still one when the near plane has cut nothing from it.
        let closed = closed && runs.len() == 1 && runs[0].len() == count;
        for run in runs {
            if run.len() < 2 || !run.iter().any(|(_, p)| on_screen.contains(*p)) {
                continue;
            }
            // From the canvas rect's corner, in device pixels like the shader wants, with
            // each point's depth and whether it is in sight.
            let points = crate::ribbon::distinct(
                run.iter().map(|&(world, screen)| {
                    (
                        (screen - rect.min) * pixels_per_point,
                        (depth.depth(world, 0.0), on_screen.contains(screen)),
                    )
                }),
                closed,
            );
            let places: Vec<Vec2> = points.iter().map(|(p, _)| *p).collect();
            let offsets = crate::ribbon::offsets(&places, reach, closed);
            for (i, j) in crate::ribbon::segments(points.len(), closed) {
                let ((a, (depth_a, seen_a)), (b, (depth_b, seen_b))) = (points[i], points[j]);
                if !seen_a && !seen_b {
                    continue;
                }
                let (a_plus, a_minus) = (a + offsets[i], a - offsets[i]);
                let (b_plus, b_minus) = (b + offsets[j], b - offsets[j]);
                for (at, depth, across) in [
                    (a_plus, depth_a, reach),
                    (a_minus, depth_a, -reach),
                    (b_minus, depth_b, -reach),
                    (a_plus, depth_a, reach),
                    (b_minus, depth_b, -reach),
                    (b_plus, depth_b, reach),
                ] {
                    vertices.extend([at.x, at.y, depth, across]);
                }
            }
        }
    }
    if vertices.is_empty() {
        return None;
    }
    // The color as the shader and egui's own have it: gamma channels from 0 to 1,
    // premultiplied by their alpha.
    let [r, g, b, a] = stroke.color.to_normalized_gamma_f32();
    let uniforms = [
        rect.width() * pixels_per_point,
        rect.height() * pixels_per_point,
        half,
        feather,
        r,
        g,
        b,
        a,
    ];
    Some((vertices, uniforms))
}

// ---------------------------------------------------------------------------------------------
// The contour lines: marching squares over the potential on a world-space grid.

/// The gravitational potential at `p`, matching the simulation's acceleration field.
pub(crate) fn potential(gravity: f64, bodies: &[CelestialBody], p: [f64; 2]) -> f64 {
    let mut phi = 0.0;
    for body in bodies {
        let d = [
            f64::from(body.position[0]) - p[0],
            f64::from(body.position[1]) - p[1],
        ];
        let r = d[0].hypot(d[1]);
        let gm = gravity * body.mass;
        let radius = f64::from(body.radius);
        if r >= radius {
            phi -= gm / r.max(1e-12);
        } else {
            phi -= gm * (3.0 * radius * radius - r * r) / (2.0 * radius * radius * radius);
        }
    }
    phi
}

/// The part of the plane `z = 0` to cover, as a square's `(min, max)` corners, padded: in
/// orthographic, what is on screen; in perspective, a square around the view's focus, as the
/// plane's far side may be beyond the horizon. `None` when the view is edge on.
pub(crate) fn region(camera: &Camera, rect: Rect) -> Option<([f64; 2], [f64; 2])> {
    if camera.eye_distance(rect).is_none() {
        let corners = [
            rect.left_top(),
            rect.right_top(),
            rect.left_bottom(),
            rect.right_bottom(),
        ]
        .map(|c| camera.screen_to_world(c, rect));
        let corners: Vec<Vec2> = corners.into_iter().collect::<Option<_>>()?;
        let min = corners.iter().copied().fold(Vec2::INFINITY, Vec2::min);
        let max = corners.iter().copied().fold(-Vec2::INFINITY, Vec2::max);
        Some(pad(min, max))
    } else {
        let d = camera.eye_distance(rect)?;
        let focus = camera
            .screen_to_world(rect.center(), rect)
            .unwrap_or_else(|| Vec2::new(camera.center.x, camera.center.y));
        let reach = 3.0 * d;
        Some(pad(focus - Vec2::splat(reach), focus + Vec2::splat(reach)))
    }
}

/// The square around `min..max`, padded by 30% of its half side.
fn pad(min: Vec2, max: Vec2) -> ([f64; 2], [f64; 2]) {
    let center = (min + max) / 2.0;
    let half = ((max - min) / 2.0).max_elem() * 1.3;
    (
        [f64::from(center.x - half), f64::from(center.y - half)],
        [f64::from(center.x + half), f64::from(center.y + half)],
    )
}

/// The potential on a square grid covering `min..max`, with the sampling lattice snapped to
/// the world.
struct ScalarGrid {
    min: [f64; 2],
    cell: f64,
    /// Cells per side; the values are `(n + 1)^2`, row-major.
    n: usize,
    values: Vec<f64>,
    /// The least and greatest values on the grid, to skip levels outside them.
    lo: f64,
    hi: f64,
}

fn scalar_grid(min: [f64; 2], max: [f64; 2], gravity: f64, bodies: &[CelestialBody]) -> ScalarGrid {
    let span = (max[0] - min[0]).max(max[1] - min[1]);
    let mut cell = span / TARGET_CELLS;
    // At least a few cells across the smallest body, so its own contours resolve.
    if let Some(radius) = bodies.iter().map(|b| f64::from(b.radius)).reduce(f64::min) {
        cell = cell.min(radius / 6.0);
    }
    let n = ((span / cell).ceil() as usize).clamp(MIN_CELLS, MAX_CELLS);
    let cell = span / n as f64;
    // Snap the lattice's origin to a multiple of the cell, so the grid always samples the
    // same world points: a pan re-uses the samples it already had (plus new ones at the
    // edge it reaches) instead of re-interpolating over a shifted lattice, and the lines
    // don't wobble between recomputes. One row and column more covers what the snapped
    // origin leaves of `max` uncovered.
    let min = [
        (min[0] / cell).floor() * cell,
        (min[1] / cell).floor() * cell,
    ];
    let n = n + 1;
    let n1 = n + 1;
    let mut values = vec![0.0; n1 * n1];
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for i in 0..n1 {
        for j in 0..n1 {
            let p = [min[0] + j as f64 * cell, min[1] + i as f64 * cell];
            let v = potential(gravity, bodies, p);
            values[i * n1 + j] = v;
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    ScalarGrid {
        min,
        cell,
        n,
        values,
        lo,
        hi,
    }
}

/// The iso-levels to contour: the band boundaries `-LEVEL_BASE.powi(n)` (where `field.wgsl`'s
/// `LN_BAND` phase wraps) whose indices fall strictly inside the grid's range of potential.
/// They are fixed in the world, not derived from the view, so no pan, zoom or recompute can
/// move or pop the lines while a body slides in and out of the padded region. If the range
/// spans more than the cap, the deepest levels win: the weakest are the ones the view can no
/// longer resolve anyway.
fn levels(grid: &ScalarGrid) -> Vec<f64> {
    const MAX_LEVELS: usize = 26;
    let mut out = Vec::new();
    // The potential is a sum of negatives, so `lo <= phi <= hi < 0`: as magnitudes, the
    // weakest and deepest sampled potentials outline which band indices cross the grid.
    let (weak, deep) = (-grid.hi, -grid.lo);
    if !(weak > 0.0 && deep > weak) {
        return out;
    }
    let index = |magnitude: f64| magnitude.ln() / LEVEL_BASE.ln();
    for n in (index(weak).floor() as i32 + 1)..=(index(deep).ceil() as i32 - 1) {
        let level = -LEVEL_BASE.powi(n);
        // Strictly inside the sampled range: at an extremum the contour would pass through
        // the extreme sample itself and degenerate.
        if grid.lo < level && level < grid.hi {
            out.push(level);
        }
    }
    if out.len() > MAX_LEVELS {
        out.drain(..out.len() - MAX_LEVELS);
    }
    out
}

/// The contour segments of one cell, as pairs of edge indices (0 bottom, 1 right, 2 top,
/// 3 left), for the case whose bits say which corners are above `level`. The saddle cases 5
/// and 10 are decided by the cell's center value (the asymptotic decider), which gives the
/// right topology at the saddle point between two bodies.
fn cell_segments(case: u8, center: f64, level: f64) -> &'static [(u8, u8)] {
    match case {
        1 | 14 => &[(3, 0)],
        2 | 13 => &[(0, 1)],
        3 | 12 => &[(3, 1)],
        4 | 11 => &[(1, 2)],
        5 => {
            if center > level {
                &[(0, 1), (2, 3)]
            } else {
                &[(3, 0), (1, 2)]
            }
        }
        6 | 9 => &[(0, 2)],
        7 | 8 => &[(2, 3)],
        10 => {
            if center > level {
                &[(3, 0), (1, 2)]
            } else {
                &[(0, 1), (2, 3)]
            }
        }
        _ => &[],
    }
}

/// The contour chains of `levels` over the whole grid, in world coordinates: marching squares
/// piece by piece, then chained together where neighbouring cells meet, so each line is one
/// smooth polyline instead of a stub per cell.
fn extract(grid: &ScalarGrid, levels: &[f64]) -> Vec<Vec<[f32; 2]>> {
    let n1 = grid.n + 1;
    let mut out = Vec::new();
    // Where which grid edge is crossed, filled in per level: horizontal edges between
    // samples `(i, j)` and `(i, j + 1)`, vertical ones between `(i, j)` and `(i + 1, j)`.
    // Every edge is written the one time, so the two cells it belongs to agree bit for bit
    // on where their pieces meet: the chain through them has no kink at the join.
    let mut horizontal = vec![[f32::NAN, f32::NAN]; n1 * grid.n];
    let mut vertical = vec![[f32::NAN, f32::NAN]; grid.n * n1];
    for &level in levels {
        if level < grid.lo || level > grid.hi {
            continue;
        }
        for i in 0..n1 {
            let row = i * n1;
            for j in 0..grid.n {
                let (a, b) = (grid.values[row + j], grid.values[row + j + 1]);
                if (a > level) != (b > level) {
                    let t = (level - a) / (b - a);
                    let y = (grid.min[1] + i as f64 * grid.cell) as f32;
                    horizontal[i * grid.n + j] =
                        [(grid.min[0] + (j as f64 + t) * grid.cell) as f32, y];
                }
            }
        }
        for i in 0..grid.n {
            let (row, next) = (i * n1, (i + 1) * n1);
            for j in 0..n1 {
                let (a, b) = (grid.values[row + j], grid.values[next + j]);
                if (a > level) != (b > level) {
                    let t = (level - a) / (b - a);
                    let x = (grid.min[0] + j as f64 * grid.cell) as f32;
                    vertical[i * n1 + j] = [x, (grid.min[1] + (i as f64 + t) * grid.cell) as f32];
                }
            }
        }
        // The pieces the saddle-decided cases cut each crossing cell into.
        let mut pieces: Vec<([f32; 2], [f32; 2])> = Vec::new();
        for i in 0..grid.n {
            let (row, next) = (i * n1, (i + 1) * n1);
            for j in 0..grid.n {
                let v = [
                    grid.values[row + j],
                    grid.values[row + j + 1],
                    grid.values[next + j + 1],
                    grid.values[next + j],
                ];
                let case = ((v[0] > level) as u8)
                    | (((v[1] > level) as u8) << 1)
                    | (((v[2] > level) as u8) << 2)
                    | (((v[3] > level) as u8) << 3);
                if case == 0 || case == 15 {
                    continue;
                }
                let center = (v[0] + v[1] + v[2] + v[3]) * 0.25;
                let crossing = |edge: u8| match edge {
                    // Bottom, right, top, left, of the cell `(i, j)`.
                    0 => horizontal[i * grid.n + j],
                    1 => vertical[i * n1 + j + 1],
                    2 => horizontal[(i + 1) * grid.n + j],
                    _ => vertical[i * n1 + j],
                };
                for &(a, b) in cell_segments(case, center, level) {
                    pieces.push((crossing(a), crossing(b)));
                }
            }
        }
        out.extend(chained(&pieces));
    }
    out
}

/// The unused piece, if any, that has an end at `p`, and its far endpoint.
fn next_free(
    at: &HashMap<(u32, u32), Vec<(u32, bool)>>,
    pieces: &[([f32; 2], [f32; 2])],
    used: &[bool],
    p: &[f32; 2],
) -> Option<(u32, [f32; 2])> {
    at.get(&(p[0].to_bits(), p[1].to_bits()))?
        .iter()
        .copied()
        .find_map(|(pi, back)| {
            (!used[pi as usize]).then(|| {
                let (a, b) = pieces[pi as usize];
                (pi, if back { a } else { b })
            })
        })
}

/// The pieces joined into polylines through their shared endpoints. An endpoint meets at
/// most one other unused piece (it sits on a grid edge shared by at most two cells), so
/// the chains come out as simple paths, one walk per contour. A chain that closes back on
/// its first point is a ring: its duplicate end point tells the drawing to close it.
fn chained(pieces: &[([f32; 2], [f32; 2])]) -> Vec<Vec<[f32; 2]>> {
    // Which pieces have an end at (the bits of) each point.
    let mut at: HashMap<(u32, u32), Vec<(u32, bool)>> = HashMap::with_capacity(2 * pieces.len());
    for (pi, (a, b)) in pieces.iter().enumerate() {
        at.entry((a[0].to_bits(), a[1].to_bits()))
            .or_default()
            .push((pi as u32, false));
        at.entry((b[0].to_bits(), b[1].to_bits()))
            .or_default()
            .push((pi as u32, true));
    }
    let mut used = vec![false; pieces.len()];
    let mut out = Vec::new();
    for pi in 0..pieces.len() {
        if used[pi] {
            continue;
        }
        used[pi] = true;
        let (a, b) = pieces[pi];
        let bits = |p: [f32; 2]| (p[0].to_bits(), p[1].to_bits());
        if bits(a) == bits(b) {
            continue; // A chain of one point: nothing to draw.
        }
        let mut chain = vec![a, b];
        while let Some(tail) = chain.last().copied()
            && let Some((pj, p)) = next_free(&at, pieces, &used, &tail)
        {
            used[pj as usize] = true;
            chain.push(p);
        }
        let mut head = Vec::new();
        while let Some(tip) = chain.first().copied()
            && let Some((pj, p)) = next_free(&at, pieces, &used, &tip)
        {
            used[pj as usize] = true;
            head.push(p);
        }
        head.reverse();
        head.extend(chain);
        out.push(head);
    }
    out
}

/// Cached contour chains in world coordinates, with the camera and world they were computed
/// for, so they are recomputed only when the view or the world changes enough.
pub struct Contours {
    chains: Vec<Vec<[f32; 2]>>,
    bodies: Vec<CelestialBody>,
    gravity: f64,
    center: Vec3,
    scale: f32,
    orbit: Orbit,
    rect: Rect,
    cell: f64,
}

impl Contours {
    fn fresh(&self, camera: &Camera, rect: Rect, bodies: &[CelestialBody], gravity: f64) -> bool {
        self.bodies == bodies
            && self.gravity == gravity
            && self.rect == rect
            && (self.scale - camera.scale).abs() < 0.1 * camera.scale
            && (self.center - camera.center).length() < (8.0 * self.cell) as f32
            && self.orbit.is_near(camera.orbit, 0.02)
    }
}

/// The contour chains for the current view, from `cache`, recomputing them when the view has
/// panned, zoomed or orbited enough, or the world changed.
pub fn contours<'a>(
    camera: &Camera,
    rect: Rect,
    bodies: &[CelestialBody],
    gravity: f64,
    cache: &'a mut Option<Contours>,
) -> &'a [Vec<[f32; 2]>] {
    if !cache
        .as_ref()
        .is_some_and(|c| c.fresh(camera, rect, bodies, gravity))
    {
        recompute(camera, rect, bodies, gravity, cache);
    }
    &cache.as_ref().expect("set").chains
}

/// Recompute the cache's contour chains for the current view and world.
fn recompute(
    camera: &Camera,
    rect: Rect,
    bodies: &[CelestialBody],
    gravity: f64,
    cache: &mut Option<Contours>,
) {
    let mut cell = 1.0;
    let chains = if bodies.is_empty() || gravity == 0.0 {
        Vec::new()
    } else if let Some((min, max)) = region(camera, rect) {
        let grid = scalar_grid(min, max, gravity, bodies);
        cell = grid.cell;
        extract(&grid, &levels(&grid))
    } else {
        Vec::new()
    };
    *cache = Some(Contours {
        chains,
        bodies: bodies.to_vec(),
        gravity,
        center: camera.center,
        scale: camera.scale,
        orbit: camera.orbit,
        rect,
        cell,
    });
}

/// The contour lines on the canvas, projected from world space (with near-plane clipping in
/// perspective), stroked `width` points thick as one line per chain: egui feathers the
/// whole polyline's edges, so the joins between cells come out as smooth as the rest, and
/// fades them out as the view turns edge on to the plane.
pub fn draw_contours(
    painter: &Painter,
    camera: &Camera,
    rect: Rect,
    world: &World,
    width: f32,
    cache: &mut Option<Contours>,
    visuals: &egui::Visuals,
) {
    let Some(stroke) = stroke(camera, width, visuals) else {
        return;
    };
    let on_screen = rect.expand(width + 8.0);
    let chains = contours(camera, rect, &world.bodies, world.gravity_constant, cache);
    for chain in chains {
        // A ring the near plane cuts in perspective comes out open, run by run.
        let (closed, points) = ring(chain);
        let points: Vec<Vec3> = points.iter().map(|p| Vec3::plane(Vec2::from(*p))).collect();
        let count = points.len();
        let runs = camera.project_polyline(points, rect);
        let close_runs = closed && runs.len() == 1 && runs[0].len() == count;
        for run in runs {
            if run.len() < 2 || !run.iter().any(|p| on_screen.contains(*p)) {
                continue;
            }
            if close_runs {
                painter.add(Shape::closed_line(run, stroke));
            } else {
                painter.line(run, stroke);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nabla_sim::physics::gravity_acceleration;

    fn body(x: f32, y: f32, r: f32, m: f64) -> CelestialBody {
        CelestialBody {
            name: "body".to_owned(),
            position: [x, y],
            radius: r,
            mass: m,
        }
    }

    #[test]
    fn potential_gradient_is_the_sims_acceleration() {
        let bodies = [body(0.0, -12.0, 7.0, 343.0), body(12.0, 4.0, 2.5, 15.625)];
        let g = 1.0;
        for p in [
            [5.0, 5.0],
            [0.0, -12.0],
            [3.0, -9.0],
            [9.89, 1.19],
            [-30.0, 25.0],
            [12.0, 4.0],
        ] {
            let h = 1e-5;
            let gx = (potential(g, &bodies, [p[0] + h, p[1]])
                - potential(g, &bodies, [p[0] - h, p[1]]))
                / (2.0 * h);
            let gy = (potential(g, &bodies, [p[0], p[1] + h])
                - potential(g, &bodies, [p[0], p[1] - h]))
                / (2.0 * h);
            let a = gravity_acceleration(&bodies, g, p);
            assert!(
                (gx + a[0]).abs() < 1e-4 && (gy + a[1]).abs() < 1e-4,
                "at {p:?}: gradient ({gx}, {gy}) but acceleration {a:?}"
            );
        }
    }

    #[test]
    fn single_body_contours_are_circles_at_the_level_radii() {
        let bodies = [body(0.0, 0.0, 2.0, 8.0)];
        let g = 1.0; // G M = 8, surface potential -4
        let grid = scalar_grid([-20.0, -20.0], [20.0, 20.0], g, &bodies);
        for level in levels(&grid) {
            // The ring's radius: G M / |level| outside the body, and inside it
            // r^2 = 3 R^2 + 2 R^3 level / (G M).
            let r = if level <= -4.0 {
                let r2 = 12.0 + 2.0 * level;
                if r2 <= 0.0 {
                    continue;
                }
                r2.sqrt()
            } else {
                8.0 / -level
            };
            if r > 19.5 {
                continue;
            }
            let chains = extract(&grid, &[level]);
            // About one point per cell the ring crosses, and every one on it.
            let points: usize = chains.iter().map(Vec::len).sum();
            let crossed = (2.0 * std::f64::consts::PI * r / grid.cell) as usize;
            assert!(
                points > crossed.max(8) / 2,
                "level {level}: only {points} points over a {crossed}-cell ring"
            );
            for chain in &chains {
                for p in chain {
                    let d = f64::from(p[0]).hypot(f64::from(p[1]));
                    assert!(
                        (d - r).abs() < grid.cell * 1.5,
                        "level {level}: point {p:?} at distance {d}, want {r}"
                    );
                }
            }
        }
    }

    #[test]
    fn two_body_contours_pass_through_the_saddle() {
        // Two bodies whose saddle point lands on a contour level exactly: the saddle's place
        // depends only on the masses' ratio and its potential only on their scale, so both
        // masses get the one factor that puts that potential on the (world-fixed) level.
        let saddle: [f64; 2] = [9.889_292, 1.185_723];
        let (rp, rm) = (
            saddle[0].hypot(saddle[1] + 12.0),
            (saddle[0] - 12.0).hypot(saddle[1] - 4.0),
        );
        let scale = LEVEL_BASE.powi(9) / (343.0 / rp + 15.625 / rm);
        let bodies = [
            body(0.0, -12.0, 7.0, 343.0 * scale),
            body(12.0, 4.0, 2.5, 15.625 * scale),
        ];
        let g = 1.0;
        let saddle_phi = potential(g, &bodies, saddle);
        // A region around the saddle, so the grid is fine there.
        let grid = scalar_grid([0.0, -10.0], [20.0, 10.0], g, &bodies);
        let level = levels(&grid)
            .into_iter()
            .min_by(|a, b| (a - saddle_phi).abs().total_cmp(&(b - saddle_phi).abs()))
            .expect("levels");
        assert!(
            (level - saddle_phi).abs() < 1e-3,
            "level {level} should be the saddle's {saddle_phi}"
        );
        let chains = extract(&grid, &[level]);
        let closest = chains
            .iter()
            .flat_map(|chain| chain.windows(2))
            .map(|w| {
                let mid = [
                    f32::midpoint(w[0][0], w[1][0]),
                    f32::midpoint(w[0][1], w[1][1]),
                ];
                f64::from(mid[0] - saddle[0] as f32).hypot(f64::from(mid[1] - saddle[1] as f32))
            })
            .fold(f64::INFINITY, f64::min);
        assert!(
            closest < 1.0,
            "level {level} near phi {saddle_phi}: closest line stretch {closest} from the saddle"
        );
    }

    #[test]
    fn chains_close_into_rings() {
        let bodies = [body(0.0, 0.0, 2.0, 8.0)];
        let grid = scalar_grid([-10.0, -10.0], [10.0, 10.0], 1.0, &bodies);
        for level in levels(&grid) {
            let r = if level <= -4.0 {
                (12.0 + 2.0 * level).sqrt()
            } else {
                8.0 / -level
            };
            if !r.is_finite() || r > 9.9 {
                continue;
            }
            let chains = extract(&grid, &[level]);
            // Wholly in view, the level set is one closed ring, not a stub soup.
            assert_eq!(
                chains.len(),
                1,
                "level {level}: one ring, got {}",
                chains.len()
            );
            let chain = &chains[0];
            assert!(
                chain.len() > 3,
                "level {level}: ring of only {} points",
                chain.len()
            );
            assert_eq!(
                chain[0],
                chain[chain.len() - 1],
                "level {level}: ring not closed"
            );
            for pair in chain.windows(2) {
                assert_ne!(
                    pair[0], pair[1],
                    "level {level}: chain doubles back on itself"
                );
            }
        }
    }

    #[test]
    fn the_3d_views_lines_are_the_flat_views_stroke() {
        let bodies = [body(0.0, 0.0, 2.0, 8.0)];
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let camera = Camera {
            scale: 20.0,
            ..Camera::default()
        };
        let mut cache = None;
        contours(&camera, rect, &bodies, 1.0, &mut cache);
        let contours = cache.as_ref().expect("warmed");
        let visuals = egui::Visuals::dark();
        let depth = DepthMap::for_camera(&camera, rect, 100.0);
        let stroke = stroke(&camera, 1.5, &visuals).expect("facing the plane");
        let (vertices, uniforms) =
            line_vertices(&camera, rect, contours, stroke, &depth, 2.0).expect("lines");
        // The shader's color is egui's: channels from 0 to 1, premultiplied, so the lines
        // are as translucent as the stroke, not opaque white.
        let [r, g, b, a] = stroke.color.to_array().map(|c| f32::from(c) / 255.0);
        assert_eq!(uniforms[4..], [r, g, b, a]);
        assert!(a > 0.0 && a < 0.5 && r <= a && g <= a && b <= a);
        // Half the stroke's width in device pixels, and a pixel of feathering.
        assert_eq!(uniforms[..4], [1600.0, 1200.0, 1.5, 1.0]);
        // Every vertex is the ribbon's reach across: the half width and half the feather.
        let vertices = vertices.as_chunks::<LINE_VERTEX_FLOATS>().0;
        assert!(vertices.len() > 600 && vertices.len().is_multiple_of(6));
        assert!(vertices.iter().all(|v| v[3].abs() == 2.0));
        // Consecutive segments' quads share their vertices where they join, so no pixel
        // of a line is blended twice: the second triangle's last vertex, and the first
        // triangle's last, are the next quad's first two.
        let place = |v: &[f32; LINE_VERTEX_FLOATS]| [v[0], v[1]];
        let joined = vertices
            .chunks(6)
            .zip(vertices.chunks(6).skip(1))
            .filter(|(quad, next)| {
                place(&quad[5]) == place(&next[0]) && place(&quad[2]) == place(&next[1])
            })
            .count();
        // All but where one ring's quads end and the next ring's begin.
        let rings = contours.chains.len();
        assert!(
            joined + rings >= vertices.len() / 6,
            "{joined} joins among {} quads of {rings} rings",
            vertices.len() / 6
        );
    }

    #[test]
    fn a_ring_closes_on_itself() {
        // The ring's last quad ends where its first began.
        let ring_chain = vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0], [0.0, 0.0]];
        assert_eq!(ring(&ring_chain), (true, &ring_chain[..4]));
        let open = vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0]];
        assert_eq!(ring(&open), (false, &open[..]));
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let camera = Camera {
            scale: 20.0,
            ..Camera::default()
        };
        let contours = Contours {
            chains: vec![ring_chain],
            bodies: Vec::new(),
            gravity: 1.0,
            center: camera.center,
            scale: camera.scale,
            orbit: camera.orbit,
            rect,
            cell: 1.0,
        };
        let depth = DepthMap::for_camera(&camera, rect, 100.0);
        let stroke = Stroke::new(2.0, egui::Color32::WHITE);
        let (vertices, _) =
            line_vertices(&camera, rect, &contours, stroke, &depth, 1.0).expect("lines");
        let vertices = vertices.as_chunks::<LINE_VERTEX_FLOATS>().0;
        // Four sides, four quads, the last back to the first.
        assert_eq!(vertices.len(), 24);
        assert_eq!(vertices[23][..2], vertices[0][..2]);
        assert_eq!(vertices[20][..2], vertices[1][..2]);
    }

    #[test]
    fn saddle_cells_split_by_the_center() {
        assert_eq!(cell_segments(5, 1.0, 0.0), &[(0, 1), (2, 3)]);
        assert_eq!(cell_segments(5, -1.0, 0.0), &[(3, 0), (1, 2)]);
        assert_eq!(cell_segments(10, 1.0, 0.0), &[(3, 0), (1, 2)]);
        assert_eq!(cell_segments(10, -1.0, 0.0), &[(0, 1), (2, 3)]);
        assert_eq!(cell_segments(1, 0.0, 0.0), &[(3, 0)]);
        assert_eq!(cell_segments(14, 0.0, 0.0), &[(3, 0)]);
        assert!(cell_segments(0, 0.0, 0.0).is_empty());
        assert!(cell_segments(15, 0.0, 0.0).is_empty());
    }

    #[test]
    fn cache_recomputes_only_when_the_view_moves_enough() {
        let bodies = [body(0.0, 0.0, 7.0, 343.0)];
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let camera = Camera {
            scale: 10.0,
            ..Camera::default()
        };
        let mut cache = None;
        let first = contours(&camera, rect, &bodies, 1.0, &mut cache);
        let ptr = first.as_ptr();
        // A small pan reuses the cache.
        let mut panned = camera;
        panned.center.x += 0.001;
        assert_eq!(
            contours(&panned, rect, &bodies, 1.0, &mut cache).as_ptr(),
            ptr,
            "small pan reuses the cache"
        );
        // A zoom out recomputes it.
        let zoomed = Camera {
            scale: 100.0,
            ..camera
        };
        assert_ne!(
            contours(&zoomed, rect, &bodies, 1.0, &mut cache).as_ptr(),
            ptr,
            "zoom recomputes"
        );
    }

    #[test]
    fn lattice_and_levels_are_pinned_to_the_world() {
        let bodies = [body(0.0, 0.0, 2.0, 8.0)];
        let home = scalar_grid([-12.0, -12.0], [12.0, 12.0], 1.0, &bodies);
        // The same view, panned somewhere it hasn't been.
        let panned = scalar_grid([-2.37, 3.11], [21.63, 17.11], 1.0, &bodies);
        // The cell depends on the span only, and both lattices' origins sit on the same
        // world grid of cells, so they sample the very same points.
        assert_eq!(panned.cell, home.cell);
        assert_eq!(panned.n, home.n);
        for a in 0..2 {
            let shift = (panned.min[a] - home.min[a]) / panned.cell;
            assert!(
                (shift - shift.round()).abs() < 1e-9,
                "axis {a}: origins off the world lattice by {shift}"
            );
        }
        // A lattice point both grids cover, from opposite sides: the same sample value.
        let n1 = home.n + 1;
        let at = |g: &ScalarGrid, x: f64, y: f64| {
            let (col, row) = (
                ((x - g.min[0]) / g.cell).round() as usize,
                ((y - g.min[1]) / g.cell).round() as usize,
            );
            g.values[row * n1 + col]
        };
        assert_eq!(at(&home, 0.0, 3.09375), at(&panned, 0.0, 3.09375));
        // The levels are the shader's band boundaries, whatever the view: -LEVEL_BASE^n.
        let panned_levels = levels(&panned);
        assert!(!panned_levels.is_empty());
        for level in panned_levels {
            let n = (-level).ln() / LEVEL_BASE.ln();
            assert_eq!(
                level,
                -LEVEL_BASE.powi(n.round() as i32),
                "off a band boundary"
            );
        }
    }
}
