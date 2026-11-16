//! The marks over the canvas in the 3D view, as one depth-tested wgpu pass
//! (`overlays.wgsl`): the gravity field's contour lines, the objective's orbit rings, the
//! target with its tolerance circle, the path flown and the controller's prediction -
//! everything the flat view draws as plain egui strokes on the plane, the 3D view must
//! test against the depth of the planets, ships and plumes drawn before it, so what
//! stands in front of the plane hides them.
//!
//! The strokes are tessellated into ribbons (`crate::ribbon`) exactly as the gravity
//! field's contour lines were: two vertices at each point of the polyline, to either side
//! along the miter of the segments that meet there, each carrying its signed distance
//! across the ribbon, which the fragment shader fences its coverage by. So they are
//! anti-aliased at any width, with or without multisampling, drawn through egui's own
//! blending, and match the egui strokes of the flat view in width and color. Each vertex
//! also carries its stroke's half-width, anti-aliasing feather and color, so the one pass
//! of a frame draws every mark whatever its width or color.

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use eframe::wgpu;
use egui::{Color32, Rect, Vec2};

use crate::camera::{Camera, Vec3};
use crate::ship3d::{DepthMap, to_bytes};

/// Floats per vertex of the pass: `pos`, `across`, `shape` and `color` of `overlays.wgsl`.
const VERTEX_FLOATS: usize = 10;
const VERTEX_BYTES: u64 = 4 * VERTEX_FLOATS as u64;

/// The pass's pipeline and buffers, created once, shared by every frame.
struct Resources {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    /// The ribbons' vertices, regrown whenever a frame no longer fits.
    vertices: wgpu::Buffer,
    capacity: u64,
    /// The last frame's bound vertices and their count.
    count: u32,
    bound: u64,
}

/// The marks' pass: ribbons in the canvas's device pixels, each vertex with the depth the
/// mark lies at there, its distance across the ribbon, the stroke's shape and color.
/// `msaa_samples` and `depth` must be those of egui's render pass, which eframe sets up
/// for the 3D view; the web painter multisamples nothing and has no depth.
pub fn init(render: &egui_wgpu::RenderState, samples: u32, depth: Option<wgpu::TextureFormat>) {
    let device = &render.device;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("overlays"),
        source: wgpu::ShaderSource::Wgsl(include_str!("overlays.wgsl").into()),
    });
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("overlays uniforms"),
        size: 8,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("overlays"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(8),
            },
            count: None,
        }],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("overlays"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniforms.as_entire_binding(),
        }],
    });
    render
        .renderer
        .write()
        .callback_resources
        .insert(Resources {
            pipeline: pipeline(device, &shader, &layout, render, samples, depth),
            bind_group,
            uniforms,
            vertices: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("overlays vertices"),
                size: 4,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            capacity: 4,
            count: 0,
            bound: 4,
        });
}

/// The marks' pipeline: ribbons in the canvas's device pixels, each vertex with the depth
/// the mark lies at there, its distance across the ribbon, and its stroke's shape and
/// color. `msaa_samples` and `depth` must be those of egui's render pass, which eframe
/// sets up for the 3D view; depth-tested but not written, so the marks never hide each
/// other, blended as egui blends its own shapes.
fn pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    render: &egui_wgpu::RenderState,
    samples: u32,
    depth: Option<wgpu::TextureFormat>,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("overlays"),
        bind_group_layouts: &[Some(layout)],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("overlays"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: VERTEX_BYTES,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x3, 1 => Float32, 2 => Float32x2, 3 => Float32x4
                ],
            })],
        },
        primitive: wgpu::PrimitiveState::default(),
        // Depth-tested but not written, so the marks never hide each other.
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

/// The paint callback of one frame's marks: their vertices, and the floats of the shader's
/// uniforms.
struct Callback {
    uniforms: Vec<u8>,
    vertices: Vec<f32>,
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
            queue.write_buffer(&res.uniforms, 0, &self.uniforms);
            let bytes = to_bytes(&self.vertices);
            res.count = (bytes.len() as u64 / VERTEX_BYTES) as u32;
            res.bound = bytes.len() as u64;
            if res.bound > res.capacity {
                res.capacity = res.bound.next_power_of_two();
                res.vertices = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("overlays vertices"),
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
        if let Some(res) = resources.get::<Resources>() {
            pass.set_pipeline(&res.pipeline);
            pass.set_bind_group(0, &res.bind_group, &[]);
            pass.set_vertex_buffer(0, res.vertices.slice(0..res.bound));
            pass.draw(0..res.count, 0..1);
        }
    }
}

/// The marks of a frame in the 3D view, collected into the pass's vertices: strokes through
/// world points on the plane, and billboards at points of it, each depth-tested through
/// `depth`, the map of the planets, ships and plumes drawn before them. The callback is
/// recreated every frame, as the view's device pixels move.
pub struct Overlays<'a> {
    camera: &'a Camera,
    rect: Rect,
    depth: &'a DepthMap,
    pixels_per_point: f32,
    vertices: Vec<f32>,
}

impl<'a> Overlays<'a> {
    pub fn new(camera: &'a Camera, rect: Rect, depth: &'a DepthMap, pixels_per_point: f32) -> Self {
        Self {
            camera,
            rect,
            depth,
            pixels_per_point,
            vertices: Vec::new(),
        }
    }

    /// A stroke `width` points thick in `color` through world `points` on the plane, `closed`
    /// back on its first point or open: the line the flat view draws as an egui stroke of
    /// the same width and color, with the same pixel of anti-aliasing. Each vertex is its
    /// place in the canvas's device pixels, the depth the plane lies at there, its distance
    /// across the ribbon, and the stroke's shape and color.
    pub fn stroke(
        &mut self,
        points: impl IntoIterator<Item = Vec3>,
        closed: bool,
        width: f32,
        color: Color32,
    ) {
        let points: Vec<Vec3> = points.into_iter().collect();
        let count = points.len();
        // In device pixels: half the stroke, and one pixel of feathering centered on its
        // edge, as egui feathers its own strokes. The ribbon covers both.
        let half = 0.5 * width * self.pixels_per_point;
        let feather = 1.0;
        let reach = half + 0.5 * feather;
        let on_screen = self.rect.expand(width + 8.0);
        let runs = self.camera.project_polyline_with_world(points, self.rect);
        // A ring is still one when the near plane has cut nothing from it.
        let closed = closed && runs.len() == 1 && runs[0].len() == count;
        for run in &runs {
            if run.len() < 2 || !run.iter().any(|(_, p)| on_screen.contains(*p)) {
                continue;
            }
            // From the canvas rect's corner, in device pixels like the shader wants, with
            // each point's depth and whether it is in sight.
            let points = crate::ribbon::distinct(
                run.iter().map(|&(world, screen)| {
                    (
                        (screen - self.rect.min) * self.pixels_per_point,
                        (self.depth.depth(world, 0.0), on_screen.contains(screen)),
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
                    self.push(at, depth, across, half, feather, color);
                }
            }
        }
    }

    /// A ring `width` points thick of `radius` points on screen around the world point
    /// `at`: a billboard marking the point, lying at its depth wherever the point is, so
    /// what stands in front of the point hides it as one. The flat view's
    /// `Painter::circle_stroke` of the same radius, width and color.
    pub fn ring(&mut self, at: Vec2, radius: f32, width: f32, color: Color32) {
        let Some(center) = self.camera.world_to_screen(at, self.rect) else {
            return;
        };
        let depth = self.depth.depth(Vec3::plane(at), 0.0);
        let (half, feather) = (0.5 * width * self.pixels_per_point, 1.0);
        let reach = half + 0.5 * feather;
        let radius = radius * self.pixels_per_point;
        let center = (center - self.rect.min) * self.pixels_per_point;
        let points: Vec<Vec2> = (0..64)
            .map(|i| {
                let angle = i as f32 / 64.0 * std::f32::consts::TAU;
                center + Vec2::angled(angle) * radius
            })
            .collect();
        let offsets = crate::ribbon::offsets(&points, reach, true);
        for (i, j) in crate::ribbon::segments(points.len(), true) {
            let (a_plus, a_minus) = (points[i] + offsets[i], points[i] - offsets[i]);
            let (b_plus, b_minus) = (points[j] + offsets[j], points[j] - offsets[j]);
            for (at, across) in [
                (a_plus, reach),
                (a_minus, -reach),
                (b_minus, -reach),
                (a_plus, reach),
                (b_minus, -reach),
                (b_plus, reach),
            ] {
                self.push(at, depth, across, half, feather, color);
            }
        }
    }

    /// A filled disc of `radius` points on screen at the world point `at`: a billboard
    /// marking the point as `ring` does, filled as the flat view's `Painter::circle_filled`
    /// of the same radius and color. Each fragment covers its pixel fully: every vertex
    /// sits on the line along the ribbon's middle, `across` 0, well within the stroke.
    pub fn disc(&mut self, at: Vec2, radius: f32, color: Color32) {
        let Some(screen) = self.camera.world_to_screen(at, self.rect) else {
            return;
        };
        let depth = self.depth.depth(Vec3::plane(at), 0.0);
        let radius = radius * self.pixels_per_point;
        // A disc of under a pixel is nothing to draw.
        if radius < 0.25 {
            return;
        }
        let center = (screen - self.rect.min) * self.pixels_per_point;
        // Any half-width at least half the feather covers a filled shape fully.
        let (half, feather) = (1.0, 1.0);
        let corner = |i: usize| {
            let angle = i as f32 / 12.0 * std::f32::consts::TAU;
            center + Vec2::angled(angle) * radius
        };
        for i in 0..12 {
            for (at, across) in [(center, 0.0), (corner(i), 0.0), (corner(i + 1), 0.0)] {
                self.push(at, depth, across, half, feather, color);
            }
        }
    }

    /// One vertex: its place in the canvas's device pixels, the depth the mark lies at
    /// there, its distance across the ribbon, and the stroke's shape and color.
    fn push(&mut self, at: Vec2, depth: f32, across: f32, half: f32, feather: f32, color: Color32) {
        let [r, g, b, a] = color.to_normalized_gamma_f32();
        self.vertices
            .extend([at.x, at.y, depth, across, half, feather, r, g, b, a]);
    }

    /// The vertices added so far, for tests of what the marks became.
    #[cfg(test)]
    pub(crate) fn vertices(&self) -> &[f32] {
        &self.vertices
    }

    /// The pass of the marks added so far, to paint over the 3D view after the planets,
    /// ships and plumes, or `None` when there is nothing to draw.
    pub fn finish(self) -> Option<egui::PaintCallback> {
        if self.vertices.is_empty() {
            return None;
        }
        let size = [
            self.rect.width() * self.pixels_per_point,
            self.rect.height() * self.pixels_per_point,
        ];
        Some(egui_wgpu::Callback::new_paint_callback(
            self.rect,
            Callback {
                uniforms: to_bytes(&size),
                vertices: self.vertices,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Camera {
        Camera {
            scale: 20.0,
            ..Camera::default()
        }
    }

    /// The vertices of the marks as ten-float vertices.
    fn vertices<'b>(overlays: &'b Overlays<'_>) -> &'b [[f32; VERTEX_FLOATS]] {
        overlays.vertices.as_chunks::<VERTEX_FLOATS>().0
    }

    #[test]
    fn a_strokes_vertices_are_its_ribbons() {
        let camera = camera();
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let depth = DepthMap::for_camera(&camera, rect, 100.0);
        let color = Color32::from_rgba_unmultiplied(10, 240, 120, 153);
        let mut overlays = Overlays::new(&camera, rect, &depth, 2.0);
        // Points far apart in a straight line: one quad of the stroke.
        overlays.stroke(
            [
                Vec3::plane(Vec2::new(0.0, 0.0)),
                Vec3::plane(Vec2::new(40.0, 0.0)),
            ],
            false,
            1.5,
            color,
        );
        let vertices = vertices(&overlays);
        assert_eq!(vertices.len(), 6, "one segment, one quad");
        // Half the stroke in device pixels, and one pixel of feathering: every vertex is
        // the ribbon's reach across, both sides.
        assert!(vertices.iter().all(|v| v[4] == 1.5 && v[5] == 1.0));
        assert!(vertices.iter().all(|v| v[3].abs() == 2.0));
        // The color as egui's: channels from 0 to 1, premultiplied by their alpha, so the
        // mark is as translucent as the stroke, not opaque.
        let [r, g, b, a] = color.to_normalized_gamma_f32();
        assert!(vertices.iter().all(|v| v[6..] == [r, g, b, a]));
        assert!(a > 0.5 && a < 1.0 && r <= a && g <= a && b <= a);
        // Nothing else to draw: no callback.
        assert!(overlays.finish().is_some());
        let empty = Overlays::new(&camera, rect, &depth, 2.0);
        assert!(empty.finish().is_none());
    }

    #[test]
    fn consecutive_segments_share_their_vertices_where_they_join() {
        let camera = camera();
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let depth = DepthMap::for_camera(&camera, rect, 100.0);
        let mut overlays = Overlays::new(&camera, rect, &depth, 1.0);
        overlays.stroke(
            [
                Vec3::plane(Vec2::new(0.0, 0.0)),
                Vec3::plane(Vec2::new(10.0, 2.5)),
                Vec3::plane(Vec2::new(20.0, 5.0)),
            ],
            false,
            1.0,
            Color32::WHITE,
        );
        let vertices = vertices(&overlays);
        assert_eq!(vertices.len(), 12, "two segments, two quads");
        let place = |v: &[f32; VERTEX_FLOATS]| [v[0], v[1]];
        assert_eq!(place(&vertices[5]), place(&vertices[6]));
        assert_eq!(place(&vertices[2]), place(&vertices[7]));
    }

    #[test]
    fn a_closed_stroke_closes_on_itself() {
        let camera = camera();
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let depth = DepthMap::for_camera(&camera, rect, 100.0);
        let mut overlays = Overlays::new(&camera, rect, &depth, 1.0);
        let square = |i: usize| {
            let (x, y) = match i % 4 {
                1 => (4.0, 0.0),
                2 => (4.0, 4.0),
                3 => (0.0, 4.0),
                _ => (0.0, 0.0),
            };
            Vec3::plane(Vec2::new(x, y))
        };
        overlays.stroke((0..4).map(square), true, 2.0, Color32::WHITE);
        // Four sides, four quads, the last back to the first.
        let vertices = vertices(&overlays);
        assert_eq!(vertices.len(), 24);
        assert_eq!(vertices[23][..2], vertices[0][..2]);
        assert_eq!(vertices[20][..2], vertices[1][..2]);
    }

    #[test]
    fn a_ring_and_disc_mark_the_point_at_its_depth() {
        let camera = camera();
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let depth = DepthMap::for_camera(&camera, rect, 100.0);
        let mut overlays = Overlays::new(&camera, rect, &depth, 1.0);
        let at = Vec2::new(40.0, 30.0);
        assert!(overlays.vertices.is_empty());
        overlays.ring(at, 10.0, 2.0, Color32::WHITE);
        overlays.disc(at, 4.0, Color32::WHITE);
        let vertices = vertices(&overlays);
        // 64 ring segments' quads and 12 disc triangles.
        assert_eq!(vertices.len(), 6 * 64 + 3 * 12);
        // All at the plane's depth there, and on the billboard around the point.
        let plane = depth.depth(Vec3::plane(at), 0.0);
        assert!(vertices.iter().all(|v| v[2] == plane));
        let center = camera.world_to_screen(at, rect).expect("in sight") - rect.min;
        for v in vertices {
            let distance = Vec2::new(v[0], v[1]) - center;
            assert!(
                distance.length() < 16.0,
                "vertex {distance:?} off the billboard"
            );
        }
        // Off the canvas, where nothing is in sight, no vertex at all.
        let mut off = Overlays::new(&camera, rect, &depth, 1.0);
        off.stroke(
            [
                Vec3::plane(Vec2::new(1e9, 0.0)),
                Vec3::plane(Vec2::new(2e9, 0.0)),
            ],
            false,
            1.0,
            Color32::WHITE,
        );
        assert!(off.vertices.is_empty());
        assert!(off.finish().is_none());
    }
}
