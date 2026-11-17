//! The gravity field of the world's bodies, as two overlays on the plane `z = 0`:
//!
//! - soft bands of the log-potential, drawn by a wgpu pass (`field.wgsl`) under everything
//!   else on the canvas, and
//! - crisp equipotential contour lines, extracted in world space by marching squares and
//!   chained into polylines: per-pixel bands alias where the field changes within one
//!   pixel (near a body, zoomed out), while world-space lines stay sub-cell accurate at any
//!   zoom. They are glowing blue strokes of the marks' pass (`crate::overlays`), to match
//!   the field lines' glowing red (`crate::streamlines`): in the flat 2D view a pass of
//!   their own; in the 3D view, with the other marks, tested against the depth of the
//!   planets and ships drawn before them, so what stands in front of the plane hides them.
//!
//! Both use the potential whose negative gradient is the simulation's acceleration
//! (`nabla_sim::physics::gravity_acceleration`): `-G M / r` outside a body, and the uniform
//! solid sphere's `-G M (3 R^2 - r^2) / (2 R^3)` inside it, continuous at the surface and
//! zero at the center.

use std::collections::HashMap;

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use eframe::wgpu::{self, util::DeviceExt as _};
use egui::{Painter, Rect, Stroke, Vec2};

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
/// In perspective, the finest cells are as many to the span of the plane `NEAR_REACH` eye
/// distances around the focus, and the cells grow with their distance from it beyond one
/// eye distance, as the view shrinks what is farther away.
const TARGET_CELLS: f64 = 256.0;
const MIN_CELLS: usize = 16;
const MAX_CELLS: usize = 800;
const NEAR_REACH: f64 = 3.0;

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
    let fog_reach = camera.plane_reach(rect).unwrap_or(0.0);
    uniforms.extend(to_bytes(&[
        gravity as f32,
        bodies.len() as f32,
        fade,
        fog_reach,
    ]));
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
// The contour lines: glowing blue strokes of the marks' pass (`crate::overlays`), matching the
// field lines' glowing red (`crate::streamlines`). In the 3D view they are drawn after the
// planets and ships, depth-tested against them through the scene's own depth map, so what
// stands in front of the plane hides them; in the flat view, at once, over the bands.

/// The contour lines' color at full strength, as egui's colors: sRGB gamma channels.
const CONTOUR_COLOR: [f32; 3] = [0.32, 0.62, 1.0];
/// How far the contour lines' glow reaches past their edge, in points.
const CONTOUR_GLOW: f32 = 6.0;

/// The contour lines on the canvas, from `cache` (`contours`, warmed by the caller) as
/// glowing strokes `width` points thick to add to the `overlays` pass, along with the other
/// marks of the frame.
pub fn contour_strokes(
    overlays: &mut crate::overlays::Overlays<'_>,
    camera: &Camera,
    cache: &Option<Contours>,
    width: f32,
) {
    let Some(stroke) = stroke(camera, width) else {
        return;
    };
    let Some(contours) = cache.as_ref() else {
        return;
    };
    for chain in &contours.chains {
        let (closed, points) = ring(chain);
        overlays.glowing_stroke(
            points.iter().map(|p| Vec3::plane(Vec2::from(*p))),
            closed,
            stroke.width,
            CONTOUR_GLOW,
            stroke.color,
        );
    }
}

/// The contour lines' stroke in either view, `width` points thick, fading out as the view
/// turns edge on to the plane; `None` once it has.
fn stroke(camera: &Camera, width: f32) -> Option<Stroke> {
    let facing = camera.orbit.toward_viewer().z.abs();
    if facing < MIN_FACING {
        return None;
    }
    let fade = ((facing - MIN_FACING) / (FULL_FACING - MIN_FACING)).min(1.0);
    let [r, g, b] = CONTOUR_COLOR.map(|c| (c * 255.0).round() as u8);
    Some(Stroke::new(
        width,
        egui::Color32::from_rgb(r, g, b).gamma_multiply(0.85 * fade),
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
/// orthographic, what is on screen; in perspective, a square around the view's focus out to
/// the plane's reach (`Camera::plane_reach`), where its fog has hidden it. `None` when the
/// view is edge on.
pub(crate) fn region(camera: &Camera, rect: Rect) -> Option<([f64; 2], [f64; 2])> {
    sampling(camera, rect).map(|s| (s.min, s.max))
}

/// How the view samples the plane for the contour lines: over `region`, with cells of
/// `cell_span / TARGET_CELLS` (or finer, for a small body), the same everywhere in
/// orthographic and, in perspective, growing past `spread`: one eye distance from the focus.
struct Sampling {
    min: [f64; 2],
    max: [f64; 2],
    cell_span: f64,
    spread: Option<([f64; 2], f64)>,
}

fn sampling(camera: &Camera, rect: Rect) -> Option<Sampling> {
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
        let (min, max) = pad(min, max);
        Some(Sampling {
            min,
            max,
            cell_span: (max[0] - min[0]).max(max[1] - min[1]),
            spread: None,
        })
    } else {
        let d = camera.eye_distance(rect)?;
        let reach = camera.plane_reach(rect)?;
        let focus = camera
            .screen_to_world(rect.center(), rect)
            .unwrap_or_else(|| Vec2::new(camera.center.x, camera.center.y));
        let (min, max) = pad(focus - Vec2::splat(reach), focus + Vec2::splat(reach));
        Some(Sampling {
            min,
            max,
            // As the padded square of `NEAR_REACH` eye distances around the focus.
            cell_span: 2.6 * NEAR_REACH * f64::from(d),
            spread: Some(([f64::from(focus.x), f64::from(focus.y)], f64::from(d))),
        })
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

/// The potential on a rectilinear grid covering `min..max`, its sample lines snapped to a
/// world lattice.
struct ScalarGrid {
    /// The sample lines' coordinates across and up the plane, increasing.
    xs: Vec<f64>,
    ys: Vec<f64>,
    /// The finest cell's side.
    cell: f64,
    /// The values at `(xs[j], ys[i])`, `xs.len()` to a row, row-major.
    values: Vec<f64>,
    /// The least and greatest values on the grid, to skip levels outside them.
    lo: f64,
    hi: f64,
}

/// The potential on a uniform grid over `min..max`, `TARGET_CELLS` to its longer side.
#[cfg(test)]
fn scalar_grid(min: [f64; 2], max: [f64; 2], gravity: f64, bodies: &[CelestialBody]) -> ScalarGrid {
    let span = (max[0] - min[0]).max(max[1] - min[1]);
    sampled(min, max, span, None, gravity, bodies)
}

/// The potential over `min..max`, cells of `cell_span / TARGET_CELLS` (finer for a small
/// body) out to `spread`'s distance from its focus, and beyond it as many times larger as
/// they are farther, in powers of two.
fn sampled(
    min: [f64; 2],
    max: [f64; 2],
    cell_span: f64,
    spread: Option<([f64; 2], f64)>,
    gravity: f64,
    bodies: &[CelestialBody],
) -> ScalarGrid {
    let mut cell = cell_span / TARGET_CELLS;
    // At least a few cells across the smallest body, so its own contours resolve.
    if let Some(radius) = bodies.iter().map(|b| f64::from(b.radius)).reduce(f64::min) {
        cell = cell.min(radius / 6.0);
    }
    // No fewer cells to a side than the least, nor more than the most, coarsening the
    // cell until there are not.
    let span = (max[0] - min[0]).max(max[1] - min[1]);
    cell = cell.min(span / MIN_CELLS as f64);
    let (xs, ys) = loop {
        let axis = |a: usize| sample_lines(min[a], max[a], cell, spread.map(|(f, d)| (f[a], d)));
        let (xs, ys) = (axis(0), axis(1));
        if xs.len().max(ys.len()) <= MAX_CELLS + 2 {
            break (xs, ys);
        }
        cell *= 2.0;
    };
    let mut values = Vec::with_capacity(xs.len() * ys.len());
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for &y in &ys {
        for &x in &xs {
            let v = potential(gravity, bodies, [x, y]);
            values.push(v);
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    ScalarGrid {
        xs,
        ys,
        cell,
        values,
        lo,
        hi,
    }
}

/// The coordinates of the sample lines along one axis, covering `lo..=hi`: multiples of
/// `cell`, so the grid always samples the same world points and a pan re-uses the samples
/// it had (the lines don't wobble between recomputes). With a `spread`, `(focus, near)`,
/// the lines are `cell` apart within `near` of the focus and, beyond, as many times farther
/// apart as twice their distance from it, in powers of two: each line `2^k` cells from the next
/// sits on a multiple of `2^k` cells, so those too stay on a world lattice.
fn sample_lines(lo: f64, hi: f64, cell: f64, spread: Option<(f64, f64)>) -> Vec<f64> {
    let (start, end) = ((lo / cell).floor() as i64, (hi / cell).ceil() as i64);
    let Some((focus, near)) = spread else {
        return (start..=end).map(|k| k as f64 * cell).collect();
    };
    let center = ((focus / cell).round() as i64).clamp(start, end);
    // The step outward from line `k`, in cells.
    // Beyond `near`, twice as far apart as the distance alone would make them: the far
    // field is under the fog, or small, and this halves the lines it takes.
    let step = |k: i64| -> i64 {
        let ratio = (k - center).unsigned_abs() as f64 * cell / near;
        let power = if ratio < 1.0 {
            0
        } else {
            ((2.0 * ratio).log2().floor() as u32)
                .min(k.trailing_zeros())
                .min(40)
        };
        1 << power
    };
    let mut below = Vec::new();
    let mut k = center;
    while k > start {
        k -= step(k);
        below.push(k);
    }
    below.reverse();
    let mut lines = below;
    k = center;
    lines.push(k);
    while k < end {
        k += step(k);
        lines.push(k);
    }
    lines.into_iter().map(|k| k as f64 * cell).collect()
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
    let (xs, ys) = (&grid.xs, &grid.ys);
    // Cells across and up, and samples to a row.
    let (nx, ny) = (xs.len() - 1, ys.len() - 1);
    let n1 = nx + 1;
    let mut out = Vec::new();
    // Where which grid edge is crossed, filled in per level: horizontal edges between
    // samples `(i, j)` and `(i, j + 1)`, vertical ones between `(i, j)` and `(i + 1, j)`.
    // Every edge is written the one time, so the two cells it belongs to agree bit for bit
    // on where their pieces meet: the chain through them has no kink at the join.
    let mut horizontal = vec![[f32::NAN, f32::NAN]; (ny + 1) * nx];
    let mut vertical = vec![[f32::NAN, f32::NAN]; ny * n1];
    for &level in levels {
        if level < grid.lo || level > grid.hi {
            continue;
        }
        for i in 0..=ny {
            let row = i * n1;
            for j in 0..nx {
                let (a, b) = (grid.values[row + j], grid.values[row + j + 1]);
                if (a > level) != (b > level) {
                    let t = (level - a) / (b - a);
                    horizontal[i * nx + j] =
                        [(xs[j] + t * (xs[j + 1] - xs[j])) as f32, ys[i] as f32];
                }
            }
        }
        for i in 0..ny {
            let (row, next) = (i * n1, (i + 1) * n1);
            for j in 0..n1 {
                let (a, b) = (grid.values[row + j], grid.values[next + j]);
                if (a > level) != (b > level) {
                    let t = (level - a) / (b - a);
                    vertical[i * n1 + j] = [xs[j] as f32, (ys[i] + t * (ys[i + 1] - ys[i])) as f32];
                }
            }
        }
        // The pieces the saddle-decided cases cut each crossing cell into.
        let mut pieces: Vec<([f32; 2], [f32; 2])> = Vec::new();
        for i in 0..ny {
            let (row, next) = (i * n1, (i + 1) * n1);
            for j in 0..nx {
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
                    0 => horizontal[i * nx + j],
                    1 => vertical[i * n1 + j + 1],
                    2 => horizontal[(i + 1) * nx + j],
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
    } else if let Some(view) = sampling(camera, rect) {
        let grid = sampled(
            view.min,
            view.max,
            view.cell_span,
            view.spread,
            gravity,
            bodies,
        );
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

/// The contour lines on the canvas in the flat view, where nothing hides them: the glowing
/// strokes of `contour_strokes`, `width` points thick, as a marks' pass of their own over the
/// plane in view.
pub fn draw_contours(
    painter: &Painter,
    camera: &Camera,
    rect: Rect,
    world: &World,
    width: f32,
    cache: &mut Option<Contours>,
    pixels_per_point: f32,
) {
    contours(camera, rect, &world.bodies, world.gravity_constant, cache);
    let Some((min, max)) = region(camera, rect) else {
        return;
    };
    // Without a scene to share the depth of, a map of the plane in view, as the bands'.
    let span = (max[0] - min[0]).max(max[1] - min[1]);
    let depth = DepthMap::for_camera(camera, rect, (0.75 * span) as f32);
    let mut overlays = crate::overlays::Overlays::new(camera, rect, &depth, pixels_per_point);
    contour_strokes(&mut overlays, camera, cache, width);
    if let Some(callback) = overlays.finish() {
        painter.add(callback);
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
    fn the_contour_lines_are_glowing_blue_strokes() {
        let bodies = [body(0.0, 0.0, 2.0, 8.0)];
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let camera = Camera {
            scale: 20.0,
            ..Camera::default()
        };
        let mut cache = None;
        contours(&camera, rect, &bodies, 1.0, &mut cache);
        let depth = DepthMap::for_camera(&camera, rect, 100.0);
        let stroke = stroke(&camera, 1.5).expect("facing the plane");
        let mut overlays = crate::overlays::Overlays::new(&camera, rect, &depth, 2.0);
        contour_strokes(&mut overlays, &camera, &cache, 1.5);
        // The shader's color is egui's: channels from 0 to 1, premultiplied, so the lines
        // are as translucent as the stroke, not opaque. Half the stroke's width in device
        // pixels, a pixel of feathering and the glow's reach: every vertex is the ribbon's
        // reach across, the half width, the feather and the glow.
        const FLOATS: usize = crate::overlays::VERTEX_FLOATS;
        let vertices: Vec<[f32; FLOATS]> = overlays.vertices().as_chunks::<FLOATS>().0.to_vec();
        let glow = CONTOUR_GLOW * 2.0;
        assert!(vertices.len() > 600 && vertices.len().is_multiple_of(6));
        assert!(vertices.iter().all(|v| v[3].abs() == 2.0 + glow));
        assert!(
            vertices
                .iter()
                .all(|v| v[4] == 1.5 && v[5] == 1.0 && v[6] == glow)
        );
        let [r, g, b, a] = stroke.color.to_normalized_gamma_f32();
        assert!(vertices.iter().all(|v| v[7..] == [r, g, b, a]));
        assert!(a > 0.5 && a < 1.0 && r <= a && g <= a && b <= a && b > 2.0 * r);
        // Consecutive segments' quads share their vertices where they join, so no pixel
        // of a line is blended twice: the second triangle's last vertex, and the first
        // triangle's last, are the next quad's first two.
        let place = |v: &[f32; FLOATS]| [v[0], v[1]];
        let joined = vertices
            .chunks(6)
            .zip(vertices.chunks(6).skip(1))
            .filter(|(quad, next)| {
                place(&quad[5]) == place(&next[0]) && place(&quad[2]) == place(&next[1])
            })
            .count();
        // All but where one ring's quads end and the next ring's begin.
        let rings = cache.as_ref().expect("warmed").chains.len();
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
        let cache = Some(Contours {
            chains: vec![ring_chain],
            bodies: Vec::new(),
            gravity: 1.0,
            center: camera.center,
            scale: camera.scale,
            orbit: camera.orbit,
            rect,
            cell: 1.0,
        });
        let depth = DepthMap::for_camera(&camera, rect, 100.0);
        let mut overlays = crate::overlays::Overlays::new(&camera, rect, &depth, 1.0);
        contour_strokes(&mut overlays, &camera, &cache, 2.0);
        // Four sides, four quads, the last back to the first.
        const FLOATS: usize = crate::overlays::VERTEX_FLOATS;
        let vertices: Vec<[f32; FLOATS]> = overlays.vertices().as_chunks::<FLOATS>().0.to_vec();
        assert_eq!(vertices.len(), 24);
        assert_eq!(vertices[23][..2], vertices[0][..2]);
        assert_eq!(vertices[20][..2], vertices[1][..2]);
    }

    #[test]
    fn perspective_sample_lines_spread_out_with_distance_on_a_world_lattice() {
        let (cell, focus, near) = (0.25, 3.1, 4.0);
        let lines = sample_lines(-400.0, 400.0, cell, Some((focus, near)));
        // Covering the range, in order.
        assert!(lines[0] <= -400.0 && *lines.last().expect("lines") >= 400.0);
        assert!(lines.windows(2).all(|w| w[0] < w[1]));
        for w in lines.windows(2) {
            let gap = w[1] - w[0];
            let cells = gap / cell;
            // A power of two of cells, each line on a multiple of the gap after it.
            assert!(cells.round() == cells && (cells as u64).is_power_of_two());
            let k = w[0] / gap;
            assert!(
                k.round() == k,
                "{} not on a multiple of its gap {gap}",
                w[0]
            );
            // The cell near the focus, and beyond, no more than twice as many times it as
            // the line is eye distances from the focus, nor less than a quarter of that.
            let ratio = (w[0] - focus).abs().max((w[1] - focus).abs()) / near;
            if ratio < 1.0 {
                assert_eq!(cells, 1.0, "gap {cells} cells within an eye distance");
            }
            assert!(
                cells <= 2.0 * ratio.max(1.0),
                "gap {cells} cells at {ratio}"
            );
            if (w[0] - focus).abs() > 4.0 * near && (w[1] - focus).abs() > 4.0 * near {
                assert!(
                    cells >= ratio / 4.0,
                    "gap {cells} cells at {ratio} eye distances"
                );
            }
        }
        // Far fewer than a uniform grid's 3200.
        assert!(lines.len() < 200, "{} lines", lines.len());
        // A pan keeps the lines it had, near the focus and far from it.
        let panned = sample_lines(-397.3, 402.7, cell, Some((focus + 2.7, near)));
        let kept = lines.iter().filter(|x| panned.contains(x)).count();
        assert!(
            kept * 10 > lines.len() * 8,
            "kept {kept} of {}",
            lines.len()
        );
    }

    #[test]
    fn a_perspective_view_samples_the_plane_out_to_its_reach() {
        let bodies = [body(0.0, 0.0, 2.0, 8.0)];
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        // A low view from the side.
        let camera = Camera {
            scale: 20.0,
            perspective: true,
            orbit: Orbit {
                azimuth: 0.3,
                elevation: 0.25,
            },
            ..Camera::default()
        };
        let view = sampling(&camera, rect).expect("facing the plane");
        let reach = f64::from(camera.plane_reach(rect).expect("perspective"));
        assert!(view.max[0] - view.min[0] >= 2.0 * reach);
        let grid = sampled(
            view.min,
            view.max,
            view.cell_span,
            view.spread,
            1.0,
            &bodies,
        );
        assert!(grid.xs.len() <= MAX_CELLS + 2 && grid.ys.len() <= MAX_CELLS + 2);
        // As fine near the focus as the old square of three eye distances was.
        let d = f64::from(camera.eye_distance(rect).expect("perspective"));
        assert!(grid.cell <= 2.6 * NEAR_REACH * d / TARGET_CELLS);
        assert!(!extract(&grid, &levels(&grid)).is_empty());
    }

    #[test]
    fn the_bands_fog_as_the_camera_does() {
        let shader = include_str!("field.wgsl");
        let start = crate::camera::PLANE_FOG_START;
        assert!(
            shader.contains(&format!("const FOG_START: f32 = {start};")),
            "field.wgsl's FOG_START should be {start}"
        );
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
        // The cell depends on the span only, and both lattices' sample lines sit on the
        // same world grid of cells, so they sample the very same points.
        assert_eq!(panned.cell, home.cell);
        assert!(panned.xs.len().abs_diff(home.xs.len()) <= 1);
        for lines in [&home.xs, &home.ys, &panned.xs, &panned.ys] {
            for &x in lines {
                let k = x / home.cell;
                assert!((k - k.round()).abs() < 1e-9, "{x} off the world lattice");
            }
        }
        // A lattice point both grids cover, from opposite sides: the same sample value.
        let at = |g: &ScalarGrid, x: f64, y: f64| {
            let find = |lines: &[f64], v: f64| {
                lines
                    .iter()
                    .position(|&l| (l - v).abs() < 1e-9)
                    .expect("on a sample line")
            };
            g.values[find(&g.ys, y) * g.xs.len() + find(&g.xs, x)]
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
