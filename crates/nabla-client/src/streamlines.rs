//! The field lines of the bodies' gravity on the plane `z = 0`: the streamlines of the
//! simulation's acceleration field (`nabla_sim::physics::gravity_acceleration`), drawn as
//! glowing red curves with arrowheads, and pulses of light falling along them the way the
//! field points (`streamlines.wgsl`).
//!
//! Everything about them is anchored to the world, so nothing moves or flickers as the view
//! pans, zooms or turns:
//!
//! - The lines are traced once per world, not per view: from evenly spaced points on each
//!   body's surface outward, against the field, to far beyond anything the view can show.
//!   Every body is a sink of the field, so each of those lines runs off to infinity, and
//!   between them they are the classic picture of the field. A body's lines are as many as the
//!   square root of its mass, which is how densely a plane cuts the lines of a field whose
//!   flux is the mass.
//! - A view draws a subset of each line's points: every point has an importance, how far the
//!   line would stray without it, and is drawn when that is more than a small part of a pixel
//!   at the view's zoom. The subset depends on the zoom alone, and grows as it does: panning
//!   changes no vertex, and zooming in only adds vertices between those already there, all of
//!   them on the one traced curve.
//! - The arrowheads sit where the lines cross a lattice of potentials, fixed like the contour
//!   lines' levels (`field.rs`) and as self-similar, halved again and again: an arrowhead
//!   shows once the zoom has made room between its coarser neighbours on the line, and fades
//!   in there without the others moving.
//! - The pulses' phase is the log-potential at each fragment, so they are wavefronts on the
//!   equipotentials, whatever the tessellation under them.
//!
//! The lines are tessellated into ribbons with miter joins (`ribbon.rs`), wide enough for
//! their glow, and
//! the arrowheads into triangles, both in device pixels and both carrying the distances their
//! fragments are anti-aliased by. One wgpu pass draws them, depth-tested against the planets
//! and ships of the 3D view like the contour lines.

use std::collections::HashMap;

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use eframe::wgpu::{self, util::DeviceExt as _};
use egui::{Rect, Vec2};

use crate::camera::{Camera, Vec3};
use crate::field::{self, LEVEL_BASE};
use crate::physics::gravity_acceleration;
use crate::ship3d::{DepthMap, to_bytes};
use crate::world::CelestialBody;

/// How many lines leave the heaviest body, and the fewest that leave any: the others have as
/// many as the square root of their mass, next to the heaviest's.
const MOST_LINES: f64 = 24.0;
const FEWEST_LINES: usize = 6;

/// The tracing's step, as a part of the distance to the nearest body's center, shortened
/// wherever the line would turn more than `MAX_TURN` radians over it, down to `MIN_STEP` of
/// that distance. Between them they bound how far a line strays from its chords: by
/// `STEP * MAX_TURN / 8` of the distance, a small part of a pixel at the closest zoom.
const STEP: f64 = 0.05;
const MAX_TURN: f64 = 0.004;
const MIN_STEP: f64 = 1e-5;
/// How far the lines are traced, in radii of the system of bodies, and the most points of one.
const FAR: f64 = 1e5;
const MAX_POINTS: usize = 20_000;

/// A point of a line is drawn when the line would stray more than this without it, in device
/// pixels.
const TOLERANCE: f32 = 0.05;

/// The core line's width, and how far its glow reaches beyond it, in points.
const CORE_WIDTH: f32 = 1.25;
const GLOW_REACH: f32 = 10.0;

/// The arrowheads' length and half width, in points.
const ARROW_LENGTH: f32 = 11.0;
const ARROW_HALF_WIDTH: f32 = 4.0;
/// The arrowheads' lattice: `ARROW_STEPS` to each band between two contour lines, starting
/// `ARROW_OFFSET` of a band from a contour line. A third is the offset that keeps every
/// refinement of the lattice at least a third of its own spacing from the contour lines.
const ARROW_STEPS: f64 = 32.0;
const ARROW_OFFSET: f64 = 1.0 / 3.0;
/// An arrowhead of the lattice's index `j` competes for room with those `2^trailing_zeros(j)`
/// indices from it, the next coarser ones; this bounds that exponent.
const MAX_STRIDE_LOG: u32 = 16;
/// The room an arrowhead needs along its line to show at all, and to show in full, in points.
const ROOM_MIN: f32 = 40.0;
const ROOM_FULL: f32 = 52.0;
/// And the room it needs beside it, from the arrowheads of the lines before its own.
const CLEARANCE_MIN: f32 = 10.0;
const CLEARANCE_FULL: f32 = 15.0;

/// The seconds a pulse takes to fall to where the next one was (`PULSE_BANDS` bands of the
/// potential further in, in `streamlines.wgsl`).
const PULSE_SECONDS: f64 = 2.4;

/// Floats per vertex: `VertexIn` of `streamlines.wgsl`.
const VERTEX_FLOATS: usize = 11;

// ---------------------------------------------------------------------------------------------
// The lines of a world.

/// An arrowhead on a line.
#[derive(Clone, Copy, Debug)]
struct Arrow {
    /// Its index in the lattice of potentials.
    index: i64,
    position: [f64; 2],
    /// The field's direction there: a unit vector.
    direction: [f64; 2],
    /// How far it is from the nearest arrowhead of the same index on a line before its own,
    /// in world units: where lines run close together, as a small body's do once a large one
    /// has swept them aside, the first of them keeps its arrowheads, and the others get
    /// theirs when the zoom has made room beside it.
    clearance: f32,
}

/// One field line, from a body out to far away.
struct Line {
    /// A point inside the body, back along the line from where it leaves the surface: the 3D
    /// view starts the line there, under the sphere that hides it, so the line comes out of
    /// the sphere rather than ending at its edge.
    stem: [f64; 2],
    /// From the body's surface outward.
    points: Vec<[f64; 2]>,
    /// How far the line would stray without each point, in world units; see `importance`.
    importance: Vec<f32>,
    /// From the body outward.
    arrows: Vec<Arrow>,
}

/// The field lines of a world, with the bodies and gravity they were traced for.
pub struct Streamlines {
    bodies: Vec<CelestialBody>,
    gravity: f64,
    lines: Vec<Line>,
}

/// The field lines of the world, from `cache`, traced again only when the world changes.
pub fn streamlines<'a>(
    bodies: &[CelestialBody],
    gravity: f64,
    cache: &'a mut Option<Streamlines>,
) -> &'a Streamlines {
    if !cache
        .as_ref()
        .is_some_and(|c| c.bodies == bodies && c.gravity == gravity)
    {
        *cache = Some(Streamlines {
            bodies: bodies.to_vec(),
            gravity,
            lines: trace_all(bodies, gravity),
        });
    }
    cache.as_ref().expect("set")
}

fn center(body: &CelestialBody) -> [f64; 2] {
    [f64::from(body.position[0]), f64::from(body.position[1])]
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// The unit vector against the field at `p`, the way the lines are traced, or `None` where the
/// field vanishes (at a saddle between bodies, or without gravity).
fn outward(bodies: &[CelestialBody], gravity: f64, p: [f64; 2]) -> Option<[f64; 2]> {
    let a = gravity_acceleration(bodies, gravity, p);
    let norm = a[0].hypot(a[1]);
    (norm.is_finite() && norm > 0.0).then(|| [-a[0] / norm, -a[1] / norm])
}

/// The point an arc length `h` along the line through `p`, outward: one step of the classic
/// Runge-Kutta method on the unit field.
fn step(bodies: &[CelestialBody], gravity: f64, p: [f64; 2], h: f64) -> Option<[f64; 2]> {
    let at = |q: [f64; 2]| outward(bodies, gravity, q);
    let k1 = at(p)?;
    let k2 = at([p[0] + 0.5 * h * k1[0], p[1] + 0.5 * h * k1[1]])?;
    let k3 = at([p[0] + 0.5 * h * k2[0], p[1] + 0.5 * h * k2[1]])?;
    let k4 = at([p[0] + h * k3[0], p[1] + h * k3[1]])?;
    Some([
        p[0] + h / 6.0 * (k1[0] + 2.0 * k2[0] + 2.0 * k3[0] + k4[0]),
        p[1] + h / 6.0 * (k1[1] + 2.0 * k2[1] + 2.0 * k3[1] + k4[1]),
    ])
}

/// The lines of every body: see the module's documentation.
fn trace_all(bodies: &[CelestialBody], gravity: f64) -> Vec<Line> {
    let heaviest = bodies.iter().map(|b| b.mass).fold(0.0, f64::max);
    let total: f64 = bodies.iter().map(|b| b.mass.max(0.0)).sum();
    if !(gravity > 0.0 && heaviest > 0.0 && total.is_finite()) {
        return Vec::new();
    }
    // The system's center of mass and its radius about it: the lines stop `FAR` radii out.
    let mut middle = [0.0; 2];
    for body in bodies {
        let c = center(body);
        middle[0] += body.mass.max(0.0) / total * c[0];
        middle[1] += body.mass.max(0.0) / total * c[1];
    }
    let radius = bodies
        .iter()
        .map(|b| distance(center(b), middle) + f64::from(b.radius))
        .fold(0.0, f64::max);
    let far = FAR * radius;
    let mut lines = Vec::new();
    for (index, body) in bodies.iter().enumerate() {
        if !(body.mass > 0.0 && body.radius > 0.0) {
            continue;
        }
        let count = ((MOST_LINES * (body.mass / heaviest).sqrt()).round() as usize)
            .clamp(FEWEST_LINES, MOST_LINES as usize);
        let (c, r) = (center(body), f64::from(body.radius));
        for k in 0..count {
            let (sin, cos) = (std::f64::consts::TAU * (k as f64 + 0.5) / count as f64).sin_cos();
            let start = [c[0] + r * cos, c[1] + r * sin];
            // Not from under another body's surface.
            let buried = bodies.iter().enumerate().any(|(other, b)| {
                other != index && distance(start, center(b)) < f64::from(b.radius)
            });
            if buried {
                continue;
            }
            let Some(heading) = outward(bodies, gravity, start) else {
                continue;
            };
            let points = trace(bodies, gravity, start, middle, far);
            if points.len() < 2 {
                continue;
            }
            lines.push(Line {
                stem: [
                    start[0] - 0.1 * r * heading[0],
                    start[1] - 0.1 * r * heading[1],
                ],
                importance: importance(&points),
                arrows: arrows(bodies, gravity, &points),
                points,
            });
        }
    }
    clear(&mut lines);
    lines
}

/// Set every arrowhead's clearance from the lines before its own. Arrowheads of one index are
/// on one equipotential, across the lines from each other, so the nearest is among them.
fn clear(lines: &mut [Line]) {
    let mut before: HashMap<i64, Vec<[f64; 2]>> = HashMap::new();
    for line in lines {
        for arrow in &mut line.arrows {
            arrow.clearance = before
                .get(&arrow.index)
                .into_iter()
                .flatten()
                .map(|other| distance(*other, arrow.position))
                .fold(f64::INFINITY, f64::min) as f32;
        }
        for arrow in &line.arrows {
            before.entry(arrow.index).or_default().push(arrow.position);
        }
    }
}

/// The line from `start` on a body's surface outward, until `far` from `middle`.
fn trace(
    bodies: &[CelestialBody],
    gravity: f64,
    start: [f64; 2],
    middle: [f64; 2],
    far: f64,
) -> Vec<[f64; 2]> {
    let mut points = vec![start];
    let mut p = start;
    let mut h = f64::INFINITY;
    while points.len() < MAX_POINTS {
        let nearest = bodies
            .iter()
            .map(|b| distance(p, center(b)))
            .fold(f64::INFINITY, f64::min);
        let Some(heading) = outward(bodies, gravity, p) else {
            break;
        };
        h = h.min(STEP * nearest);
        // Halve the step until the line turns little enough over it.
        let next = loop {
            let Some(q) = step(bodies, gravity, p, h) else {
                break None;
            };
            let Some(after) = outward(bodies, gravity, q) else {
                break None;
            };
            let turn = (heading[0] * after[1] - heading[1] * after[0])
                .atan2(heading[0] * after[0] + heading[1] * after[1])
                .abs();
            if turn <= MAX_TURN || h <= MIN_STEP * nearest {
                break Some(q);
            }
            h *= 0.5;
        };
        let Some(q) = next else {
            break;
        };
        points.push(q);
        if distance(q, middle) > far {
            break;
        }
        p = q;
        h *= 2.0;
    }
    points
}

fn distance_to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
    let length2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if length2 > 0.0 {
        ((ap[0] * ab[0] + ap[1] * ab[1]) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (ap[0] - t * ab[0]).hypot(ap[1] - t * ab[1])
}

/// How far the line would stray without each of its points, in world units, for a view to
/// draw the points whose importance is more than its tolerance.
///
/// The points are ranked as a binary tree over their indices: the odd ones are the leaves,
/// each standing between its two neighbours; the odd multiples of two stand between their
/// neighbours two away, and so on. A point's importance is how far the points it stands for
/// lie from the chord between the two it stands between, and no less than the importance of
/// the two points under it in the tree. So the points a tolerance keeps are a whole top of
/// the tree: a coarser tolerance's points are among a finer one's, whatever part of the line
/// is in view. The line's two ends are always kept.
fn importance(points: &[[f64; 2]]) -> Vec<f32> {
    let n = points.len();
    let mut out = vec![0.0_f32; n];
    let Some(last) = n.checked_sub(1) else {
        return out;
    };
    let mut stride = 1;
    while stride < n {
        let mut i = stride;
        while i < last {
            let (a, b) = (i - stride, (i + stride).min(last));
            let mut worst = (a + 1..b)
                .map(|j| distance_to_segment(points[j], points[a], points[b]))
                .fold(0.0, f64::max) as f32;
            if stride > 1 {
                worst = worst.max(out[i - stride / 2]);
                if let Some(&under) = out.get(i + stride / 2) {
                    worst = worst.max(under);
                }
            }
            out[i] = worst;
            i += 2 * stride;
        }
        stride *= 2;
    }
    out[0] = f32::INFINITY;
    out[last] = f32::INFINITY;
    out
}

/// Where the potential `phi` is in the arrowheads' lattice: its integers are the arrowheads.
fn lattice(phi: f64) -> f64 {
    ((-phi).ln() / LEVEL_BASE.ln() - ARROW_OFFSET) * ARROW_STEPS
}

/// The arrowheads of the line through `points`: where it crosses the lattice's potentials,
/// from the body's surface outward. The potential only rises along the line, so each is
/// crossed once.
fn arrows(bodies: &[CelestialBody], gravity: f64, points: &[[f64; 2]]) -> Vec<Arrow> {
    let mut out = Vec::new();
    let at = |p: [f64; 2]| lattice(field::potential(gravity, bodies, p));
    let mut before = at(points[0]);
    for pair in points.windows(2) {
        let after = at(pair[1]);
        if !(before.is_finite() && after.is_finite()) {
            break;
        }
        // Outward the lattice coordinate falls: the indices in `(after, before]`.
        let mut index = before.floor() as i64;
        while index as f64 > after {
            let along = |t: f64| {
                [
                    pair[0][0] + t * (pair[1][0] - pair[0][0]),
                    pair[0][1] + t * (pair[1][1] - pair[0][1]),
                ]
            };
            // Where on the chord: the lattice coordinate is nearly linear along it, and a
            // few secant steps make up the rest.
            let (mut t0, mut v0, mut t1, mut v1) = (0.0, before, 1.0, after);
            let mut t = (before - index as f64) / (before - after);
            for _ in 0..4 {
                let v = at(along(t));
                if !v.is_finite() || v == index as f64 {
                    break;
                }
                if v > index as f64 {
                    (t0, v0) = (t, v);
                } else {
                    (t1, v1) = (t, v);
                }
                t = t0 + (t1 - t0) * (v0 - index as f64) / (v0 - v1);
            }
            let position = along(t);
            if let Some(against) = outward(bodies, gravity, position) {
                out.push(Arrow {
                    index,
                    position,
                    direction: [-against[0], -against[1]],
                    clearance: f32::INFINITY,
                });
            }
            index -= 1;
        }
        before = after;
    }
    out
}

// ---------------------------------------------------------------------------------------------
// A frame's mesh: the lines' ribbons and the arrowheads' triangles, in device pixels.

/// The vertices of one frame, for `streamlines.wgsl`.
struct Mesh<'a> {
    camera: &'a Camera,
    rect: Rect,
    depth: &'a DepthMap,
    pixels_per_point: f32,
    /// Half the ribbons' width, glow and all, and the feather, in device pixels.
    reach: f32,
    feather: f32,
    vertices: Vec<f32>,
}

impl Mesh<'_> {
    /// `screen`, in points, as device pixels from the canvas rect's corner.
    fn pixels(&self, screen: egui::Pos2) -> Vec2 {
        (screen - self.rect.min) * self.pixels_per_point
    }

    /// The last three of a vertex's floats but one: the world point and the perspective's
    /// scale there, for the shader to find each fragment's own world point from.
    fn anchor(&self, world: Vec3) -> [f32; 3] {
        let q = self
            .camera
            .scale_at(world, self.rect)
            .map_or(1.0, |scale| scale / self.camera.scale);
        [world.x * q, world.y * q, q]
    }

    fn push(&mut self, at: Vec2, depth: f32, shape: [f32; 4], anchor: [f32; 3], opacity: f32) {
        self.vertices.extend([at.x, at.y, depth]);
        self.vertices.extend(shape);
        self.vertices.extend(anchor);
        self.vertices.push(opacity);
    }

    /// The ribbon (`crate::ribbon`) along `run`, a stretch of a line as its world points and
    /// where they are on screen, `reach` to each side: wide enough for the glow, each vertex
    /// with its distance across. Clip first, so segments through the canvas never become huge
    /// screen-space triangles when a perspective view comes close to the plane.
    fn ribbon(&mut self, run: &[(Vec3, egui::Pos2)]) {
        let points: Vec<_> = run
            .iter()
            .map(|&(world, screen)| {
                (
                    self.pixels(screen),
                    (self.depth.depth(world, 0.0), self.anchor(world)),
                )
            })
            .collect();
        let bounds = Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            Vec2::new(self.rect.width(), self.rect.height()) * self.pixels_per_point,
        )
        .expand(2.0 * self.reach + self.feather);
        let clipped = crate::ribbon::clip(&points, bounds, false, |(da, aa), (db, ab), t| {
            (
                da + (db - da) * t,
                std::array::from_fn(|i| aa[i] + (ab[i] - aa[i]) * t),
            )
        });
        for points in clipped {
            self.ribbon_run(&points);
        }
    }

    fn ribbon_run(&mut self, points: &[(Vec2, (f32, [f32; 3]))]) {
        let points = crate::ribbon::distinct(points.iter().copied(), false);
        if points.len() < 2 {
            return;
        }
        let places: Vec<Vec2> = points.iter().map(|(p, _)| *p).collect();
        let offsets = crate::ribbon::offsets(&places, self.reach, false);
        // Each point's two vertices: where, and the rest of what they share.
        let sides: Vec<(Vec2, Vec2, f32, [f32; 3])> = points
            .iter()
            .zip(offsets)
            .map(|(&(p, (depth, anchor)), offset)| (p + offset, p - offset, depth, anchor))
            .collect();
        let (plus, minus) = ([self.reach, 0.0, 0.0, 0.0], [-self.reach, 0.0, 0.0, 0.0]);
        for pair in sides.windows(2) {
            let ((a_plus, a_minus, a_depth, a_anchor), (b_plus, b_minus, b_depth, b_anchor)) =
                (pair[0], pair[1]);
            self.push(a_plus, a_depth, plus, a_anchor, 1.0);
            self.push(a_minus, a_depth, minus, a_anchor, 1.0);
            self.push(b_minus, b_depth, minus, b_anchor, 1.0);
            self.push(a_plus, a_depth, plus, a_anchor, 1.0);
            self.push(b_minus, b_depth, minus, b_anchor, 1.0);
            self.push(b_plus, b_depth, plus, b_anchor, 1.0);
        }
    }

    /// The arrowhead at `position` on the plane, pointing along `direction`: a triangle lying
    /// in the plane, the size on screen its constants say wherever it is, its centroid on the
    /// line. Each edge is pushed out by half the feather, and each vertex carries its
    /// distances to the three edges, for the fragments to fence themselves in by.
    fn arrow(&mut self, position: Vec2, direction: Vec2, opacity: f32) {
        let center = Vec3::plane(position);
        let Some(scale) = self.camera.scale_at(center, self.rect) else {
            return;
        };
        let (along, across) = (direction / scale, direction.rot90() / scale);
        let back = position - along * (ARROW_LENGTH / 3.0);
        let corners = [
            position + along * (2.0 * ARROW_LENGTH / 3.0),
            back + across * ARROW_HALF_WIDTH,
            back - across * ARROW_HALF_WIDTH,
        ]
        .map(|corner| self.camera.project(Vec3::plane(corner), self.rect));
        let [Some(a), Some(b), Some(c)] = corners else {
            return;
        };
        let (a, b, c) = (self.pixels(a), self.pixels(b), self.pixels(c));
        // Each corner's opposite edge.
        let edges = [(b - c).length(), (c - a).length(), (a - b).length()];
        let perimeter: f32 = edges.iter().sum();
        let doubled_area = ((b - a).x * (c - a).y - (b - a).y * (c - a).x).abs();
        let inradius = doubled_area / perimeter;
        // Edge on to the view, the triangle has no inside to draw.
        if !(inradius > 0.25 && inradius.is_finite()) {
            return;
        }
        // Scaling about the incenter moves every edge out by the same distance.
        let incenter = (a * edges[0] + b * edges[1] + c * edges[2]) / perimeter;
        let grow = (inradius + 0.5 * self.feather) / inradius;
        let depth = self.depth.depth(center, 0.0);
        let anchor = self.anchor(center);
        for (i, corner) in [a, b, c].into_iter().enumerate() {
            let mut shape = [0.0, 0.0, 0.0, 1.0];
            // The grown triangle's height over this corner's opposite edge.
            shape[i] = doubled_area * grow / edges[i];
            self.push(
                incenter + (corner - incenter) * grow,
                depth,
                shape,
                anchor,
                opacity,
            );
        }
    }
    /// The ribbons of every line's stretches whose segments may cross `region`, the part of
    /// the plane in view, as its `(min, max)` corners. Where there are spheres to hide what
    /// is `under_surface`, the lines start there, from their stems.
    fn lines(
        &mut self,
        streamlines: &Streamlines,
        (min, max): ([f64; 2], [f64; 2]),
        under_surface: bool,
    ) {
        let (camera, rect, pixels_per_point) = (self.camera, self.rect, self.pixels_per_point);
        // The tolerance in world units: a part of a device pixel, at the zoom where the
        // point is.
        let tolerance = |p: [f64; 2]| {
            let scale = camera
                .scale_at(plane(camera, p), rect)
                .unwrap_or(camera.scale);
            TOLERANCE / (scale * pixels_per_point)
        };
        let mut run: Vec<Vec3> = Vec::new();
        for line in &streamlines.lines {
            let mut previous: Option<[f64; 2]> = under_surface.then_some(line.stem);
            for i in drawn(line, tolerance) {
                let p = line.points[i];
                if let Some(q) = previous {
                    let apart = (0..2).any(|axis| {
                        p[axis].max(q[axis]) < min[axis] || p[axis].min(q[axis]) > max[axis]
                    });
                    if apart {
                        self.stretch(&mut run);
                    } else {
                        if run.is_empty() {
                            run.push(plane(camera, q));
                        }
                        run.push(plane(camera, p));
                    }
                }
                previous = Some(p);
            }
            self.stretch(&mut run);
        }
    }

    /// The ribbons of `run`, a stretch of a line, emptying it: one, or in perspective one
    /// for each part of it in front of the eye.
    fn stretch(&mut self, run: &mut Vec<Vec3>) {
        let projected = self
            .camera
            .project_polyline_with_world(run.drain(..), self.rect);
        for part in projected {
            self.ribbon(&part);
        }
    }

    /// The arrowheads in `region`, over the lines: each as opaque as the room it has on its
    /// line, between the next coarser arrowheads of the lattice either side of it, or the
    /// line's ends, and beside it, from the lines before its own.
    fn arrows(&mut self, streamlines: &Streamlines, (min, max): ([f64; 2], [f64; 2])) {
        let (camera, rect) = (self.camera, self.rect);
        let inside =
            |p: [f64; 2]| min[0] <= p[0] && p[0] <= max[0] && min[1] <= p[1] && p[1] <= max[1];
        let screen = |p: [f64; 2]| camera.project(plane(camera, p), rect);
        for line in &streamlines.lines {
            for (k, arrow) in line.arrows.iter().enumerate() {
                if !inside(arrow.position) {
                    continue;
                }
                let Some(at) = screen(arrow.position) else {
                    continue;
                };
                let stride = 1_usize << arrow.index.trailing_zeros().min(MAX_STRIDE_LOG);
                let inward = k
                    .checked_sub(stride)
                    .map_or(line.points[0], |j| line.arrows[j].position);
                let outward = line
                    .arrows
                    .get(k + stride)
                    .map_or(line.points[line.points.len() - 1], |a| a.position);
                let room = [inward, outward]
                    .into_iter()
                    .filter_map(screen)
                    .map(|p| (p - at).length())
                    .fold(f32::INFINITY, f32::min);
                let beside = camera
                    .scale_at(plane(camera, arrow.position), rect)
                    .map_or(f32::INFINITY, |scale| arrow.clearance * scale);
                let opacity = smoothstep(ROOM_MIN, ROOM_FULL, room)
                    * smoothstep(CLEARANCE_MIN, CLEARANCE_FULL, beside);
                if opacity > 0.0 {
                    self.arrow(
                        camera.local(arrow.position[0], arrow.position[1]),
                        Vec2::new(arrow.direction[0] as f32, arrow.direction[1] as f32),
                        opacity,
                    );
                }
            }
        }
    }
}

/// The point `p` of the world's plane, as `camera` takes it: from its floating origin.
fn plane(camera: &Camera, p: [f64; 2]) -> Vec3 {
    Vec3::plane(camera.local(p[0], p[1]))
}

fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The indices of `line`'s points a view draws: those whose importance is more than
/// `tolerance` there (in world units). A function of the tolerance alone, not of what part
/// of the line is in view.
fn drawn<'a>(
    line: &'a Line,
    tolerance: impl Fn([f64; 2]) -> f32 + 'a,
) -> impl Iterator<Item = usize> + 'a {
    (0..line.points.len()).filter(move |&i| line.importance[i] > tolerance(line.points[i]))
}

/// The paint callback of the field lines on the canvas, or `None` when there is nothing to
/// show: no lines, or the plane edge on. Depth-tested through `depth`, the map of the planets
/// and ships the 3D view drew before them; the flat view has none, and with no spheres to
/// hide what is inside the bodies, its lines start at their surfaces. `time` is the clock the
/// pulses run by, in seconds.
pub fn paint(
    camera: &Camera,
    rect: Rect,
    streamlines: &Streamlines,
    depth: Option<&DepthMap>,
    pixels_per_point: f32,
    time: f64,
) -> Option<egui::PaintCallback> {
    let (vertices, uniforms) = build(camera, rect, streamlines, depth, pixels_per_point, time)?;
    Some(egui_wgpu::Callback::new_paint_callback(
        rect,
        Callback {
            // At their places from the floating origin, as the vertices' world points are.
            bodies: camera.local_bodies(&streamlines.bodies),
            uniforms: to_bytes(&uniforms),
            vertices,
        },
    ))
}

/// What `paint` draws: the frame's vertices, and the floats of the shader's uniforms.
fn build(
    camera: &Camera,
    rect: Rect,
    streamlines: &Streamlines,
    depth: Option<&DepthMap>,
    pixels_per_point: f32,
    time: f64,
) -> Option<(Vec<f32>, [f32; 12])> {
    let facing = camera.orbit.toward_viewer().z.abs();
    if facing < field::MIN_FACING || streamlines.lines.is_empty() {
        return None;
    }
    let fade = ((facing - field::MIN_FACING) / (field::FULL_FACING - field::MIN_FACING)).min(1.0);
    // The part of the plane in view, in the world's coordinates, as the lines are.
    let (min, max) = field::region(camera, rect)?;
    let (min, max) = (
        [min[0] + camera.origin[0], min[1] + camera.origin[1]],
        [max[0] + camera.origin[0], max[1] + camera.origin[1]],
    );
    let depth_given = depth.is_some();
    // Without a scene to share the depth of, a map of the plane in view, as the bands'.
    let span = (max[0] - min[0]).max(max[1] - min[1]);
    let own_depth = DepthMap::for_camera(camera, rect, (0.75 * span) as f32);
    let depth = depth.unwrap_or(&own_depth);
    let half = 0.5 * CORE_WIDTH * pixels_per_point;
    let glow = GLOW_REACH * pixels_per_point;
    let mut mesh = Mesh {
        camera,
        rect,
        depth,
        pixels_per_point,
        // The ribbon covers the glow, and the core's feather where there is no glow.
        reach: half + glow.max(1.0),
        feather: 1.0,
        vertices: Vec::new(),
    };
    mesh.lines(streamlines, (min, max), depth_given);
    mesh.arrows(streamlines, (min, max));
    if mesh.vertices.is_empty() {
        return None;
    }
    let phase = (time / PULSE_SECONDS).rem_euclid(1.0) as f32;
    let uniforms = [
        rect.width() * pixels_per_point,
        rect.height() * pixels_per_point,
        half,
        mesh.feather,
        streamlines.gravity as f32,
        streamlines.bodies.len() as f32,
        phase,
        glow.max(1.0),
        fade,
        0.0,
        0.0,
        0.0,
    ];
    Some((mesh.vertices, uniforms))
}

// ---------------------------------------------------------------------------------------------
// The wgpu pass.

const UNIFORM_BYTES: u64 = 48;
const VERTEX_BYTES: u64 = 4 * VERTEX_FLOATS as u64;

struct Resources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    bodies_buf: wgpu::Buffer,
    bodies_key: Vec<CelestialBody>,
    /// The frame's vertices, regrown whenever a frame no longer fits.
    vertices: wgpu::Buffer,
    capacity: u64,
    /// The last frame's bound vertices and their bytes.
    count: u32,
    bound: u64,
}

pub fn init(render: &egui_wgpu::RenderState, samples: u32, depth: Option<wgpu::TextureFormat>) {
    let device = &render.device;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gravity field lines"),
        source: wgpu::ShaderSource::Wgsl(include_str!("streamlines.wgsl").into()),
    });
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gravity field lines uniforms"),
        size: UNIFORM_BYTES,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bodies_buf = bodies_buffer(device, &[]);
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("gravity field lines"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(UNIFORM_BYTES),
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
    let pipeline = pipeline(device, &shader, &layout, render, samples, depth);
    let resources = Resources {
        bind_group: bind_group(device, &layout, &uniforms, &bodies_buf),
        pipeline,
        layout,
        uniforms,
        bodies_buf,
        bodies_key: Vec::new(),
        vertices: device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gravity field lines vertices"),
            size: 4,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        capacity: 4,
        count: 0,
        bound: 4,
    };
    render.renderer.write().callback_resources.insert(resources);
}

/// The pipeline: as the contour lines' (`field.rs`), screen-space vertices with the plane's
/// depth, tested against what drew before but not written, blended as egui blends.
fn pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    render: &egui_wgpu::RenderState,
    samples: u32,
    depth: Option<wgpu::TextureFormat>,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("gravity field lines"),
        bind_group_layouts: &[Some(layout)],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("gravity field lines"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: VERTEX_BYTES,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x3, 1 => Float32x4, 2 => Float32x3, 3 => Float32
                ],
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

/// The bodies as the shader reads them: x, y, radius and gravitational mass of each.
fn bodies_buffer(device: &wgpu::Device, bodies: &[CelestialBody]) -> wgpu::Buffer {
    let mut floats: Vec<f32> = bodies
        .iter()
        .flat_map(|b| [b.position[0], b.position[1], b.radius, b.mass as f32])
        .collect();
    if floats.is_empty() {
        floats.extend([0.0; 4]);
    }
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("gravity field lines bodies"),
        contents: &to_bytes(&floats),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    })
}

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    bodies: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gravity field lines"),
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

/// One frame's field lines: the mesh, the uniforms, and the bodies whose potential the
/// shader's pulses follow.
struct Callback {
    bodies: Vec<CelestialBody>,
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
            // Upload the bodies only when the world's change.
            if res.bodies_key != self.bodies {
                res.bodies_buf = bodies_buffer(device, &self.bodies);
                res.bind_group = bind_group(device, &res.layout, &res.uniforms, &res.bodies_buf);
                res.bodies_key.clone_from(&self.bodies);
            }
            queue.write_buffer(&res.uniforms, 0, &self.uniforms);
            let bytes = to_bytes(&self.vertices);
            res.count = (bytes.len() as u64 / VERTEX_BYTES) as u32;
            res.bound = bytes.len() as u64;
            if res.bound > res.capacity {
                res.capacity = res.bound.next_power_of_two();
                res.vertices = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gravity field lines vertices"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Orbit;

    fn body(x: f32, y: f32, r: f32, m: f64) -> CelestialBody {
        CelestialBody {
            name: "body".to_owned(),
            position: [x, y],
            radius: r,
            mass: m,
        }
    }

    /// The planet and moon of the preset world.
    fn planet_and_moon() -> [CelestialBody; 2] {
        [body(0.0, -12.0, 7.0, 343.0), body(12.0, 4.0, 2.5, 15.625)]
    }

    fn rect() -> Rect {
        Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))
    }

    /// The vertices of a frame's mesh, as `[f32; VERTEX_FLOATS]` each.
    fn vertices_of(mesh: &Mesh<'_>) -> Vec<[f32; VERTEX_FLOATS]> {
        mesh.vertices.as_chunks::<VERTEX_FLOATS>().0.to_vec()
    }

    fn mesh<'a>(camera: &'a Camera, depth: &'a DepthMap) -> Mesh<'a> {
        Mesh {
            camera,
            rect: rect(),
            depth,
            pixels_per_point: 2.0,
            reach: 9.0,
            feather: 1.0,
            vertices: Vec::new(),
        }
    }

    #[test]
    fn a_lone_bodys_lines_are_evenly_spaced_rays() {
        let bodies = [body(3.0, -2.0, 2.0, 8.0)];
        let lines = trace_all(&bodies, 1.0);
        assert_eq!(lines.len(), MOST_LINES as usize);
        for (k, line) in lines.iter().enumerate() {
            let angle = std::f64::consts::TAU * (k as f64 + 0.5) / MOST_LINES;
            let (sin, cos) = angle.sin_cos();
            // From under the surface, through it, to far away, along the one ray.
            assert!((distance(line.stem, [3.0, -2.0]) - 1.8).abs() < 1e-9);
            assert!((distance(line.points[0], [3.0, -2.0]) - 2.0).abs() < 1e-9);
            let last = line.points[line.points.len() - 1];
            assert!(distance(last, [3.0, -2.0]) > FAR * 2.0);
            for p in line.points.iter().chain([&line.stem]) {
                let d = [p[0] - 3.0, p[1] + 2.0];
                let off = (d[0] * sin - d[1] * cos).abs();
                assert!(
                    off < 1e-9 * (1.0 + d[0].hypot(d[1])),
                    "line {k}: {p:?} is {off} off its ray"
                );
            }
            // Its arrowheads point at the body, on the lattice's potentials.
            assert!(!line.arrows.is_empty());
            for arrow in &line.arrows {
                assert!((arrow.direction[0] + cos).abs() < 1e-9);
                assert!((arrow.direction[1] + sin).abs() < 1e-9);
                let at = lattice(field::potential(1.0, &bodies, arrow.position));
                assert!(
                    (at - arrow.index as f64).abs() < 1e-6,
                    "arrow {} at lattice coordinate {at}",
                    arrow.index
                );
            }
        }
    }

    #[test]
    fn lines_follow_the_field_out_to_far_away() {
        let bodies = planet_and_moon();
        let lines = trace_all(&bodies, 1.0);
        // The planet's lines, and the moon's: as the square root of its mass next to the
        // planet's, but no fewer than the fewest.
        assert_eq!(lines.len(), 24 + 6);
        for line in &lines {
            let last = line.points[line.points.len() - 1];
            assert!(
                line.points.len() < MAX_POINTS && last[0].hypot(last[1]) > 1e5,
                "a line of {} points ends at {last:?}",
                line.points.len()
            );
            let mut phi = f64::NEG_INFINITY;
            for pair in line.points.windows(2) {
                // Each chord runs against the field at its middle, up the potential.
                let chord = [pair[1][0] - pair[0][0], pair[1][1] - pair[0][1]];
                let middle = [
                    f64::midpoint(pair[0][0], pair[1][0]),
                    f64::midpoint(pair[0][1], pair[1][1]),
                ];
                let against = outward(&bodies, 1.0, middle).expect("a field");
                let length = chord[0].hypot(chord[1]);
                let sine = (chord[0] * against[1] - chord[1] * against[0]) / length;
                assert!(
                    sine.abs() < 0.25 * MAX_TURN,
                    "chord at {middle:?} off the field by {sine}"
                );
                assert!(chord[0] * against[0] + chord[1] * against[1] > 0.0);
                let here = field::potential(1.0, &bodies, pair[0]);
                assert!(here > phi, "the potential falls along a line");
                phi = here;
            }
            // No line enters a body on its way out.
            for p in &line.points[1..] {
                for b in &bodies {
                    assert!(distance(*p, center(b)) > f64::from(b.radius));
                }
            }
            // The arrowheads' indices fall by one from each to the next, outward.
            for pair in line.arrows.windows(2) {
                assert_eq!(pair[1].index, pair[0].index - 1);
            }
        }
    }

    #[test]
    fn a_tolerances_points_are_among_a_finer_ones_and_stay_within_it() {
        let lines = trace_all(&planet_and_moon(), 1.0);
        for line in &lines {
            let kept = |tolerance: f32| -> Vec<usize> { drawn(line, move |_| tolerance).collect() };
            let mut finer = kept(1e-6);
            for tolerance in [1e-5, 1e-4, 1e-3, 1e-2, 1e-1, 1.0, 10.0] {
                let coarser = kept(tolerance);
                assert!(coarser.len() <= finer.len());
                assert!(
                    coarser.iter().all(|i| finer.binary_search(i).is_ok()),
                    "tolerance {tolerance}: a point not among the finer ones"
                );
                assert_eq!(coarser[0], 0);
                assert_eq!(coarser[coarser.len() - 1], line.points.len() - 1);
                // Every point left out is within a few tolerances of the line drawn.
                for pair in coarser.windows(2) {
                    for j in pair[0] + 1..pair[1] {
                        let off = distance_to_segment(
                            line.points[j],
                            line.points[pair[0]],
                            line.points[pair[1]],
                        );
                        assert!(
                            off <= 4.0 * f64::from(tolerance),
                            "tolerance {tolerance}: point {j} is {off} off"
                        );
                    }
                }
                finer = coarser;
            }
        }
    }

    #[test]
    fn the_cache_traces_only_when_the_world_changes() {
        let bodies = planet_and_moon();
        let mut cache = None;
        let first = streamlines(&bodies, 1.0, &mut cache).lines.as_ptr();
        assert_eq!(streamlines(&bodies, 1.0, &mut cache).lines.as_ptr(), first);
        let heavier = streamlines(&bodies, 2.0, &mut cache);
        assert_eq!(heavier.gravity, 2.0);
        assert!(streamlines(&[], 1.0, &mut cache).lines.is_empty());
        assert!(streamlines(&bodies, 0.0, &mut cache).lines.is_empty());
    }

    #[test]
    fn a_ribbons_sides_run_its_reach_from_every_segment() {
        let camera = Camera::default();
        let depth = DepthMap::for_camera(&camera, rect(), 100.0);
        let mut mesh = mesh(&camera, &depth);
        // A bend, as world points and where they are on screen, in points.
        let run: Vec<(Vec3, egui::Pos2)> = [(10.0, 10.0), (60.0, 10.0), (100.0, 40.0)]
            .map(|(x, y)| (Vec3::new(x, y, 0.0), egui::pos2(x, y)))
            .to_vec();
        mesh.ribbon(&run);
        let vertices = vertices_of(&mesh);
        // Two segments, a quad each.
        assert_eq!(vertices.len(), 12);
        let pixels: Vec<Vec2> = run.iter().map(|(_, p)| p.to_vec2() * 2.0).collect();
        for (quad, segment) in vertices.chunks(6).zip(pixels.windows(2)) {
            let along = (segment[1] - segment[0]).normalized();
            for v in quad {
                // The distance across each vertex carries is its own from the segment's
                // line, to the side its sign says: so it is every fragment's between them.
                let from = Vec2::new(v[0], v[1]) - segment[0];
                let across = along.y * from.x - along.x * from.y;
                assert!((across - v[3]).abs() < 1e-3, "{across} against {}", v[3]);
                assert!((v[3].abs() - 9.0).abs() < 1e-6);
                assert_eq!((v[6], v[9], v[10]), (0.0, 1.0, 1.0));
            }
        }
        // The two quads share their vertices at the bend: nothing overlaps, nothing gaps.
        assert_eq!(vertices[2][..2], vertices[7][..2]);
        assert_eq!(vertices[5][..2], vertices[6][..2]);
    }

    #[test]
    fn a_ribbon_clips_enormous_projected_segments_to_the_canvas() {
        let camera = Camera::default();
        let depth = DepthMap::for_camera(&camera, rect(), 100.0);
        let mut mesh = mesh(&camera, &depth);
        let run = [
            (Vec3::new(-100.0, 0.0, 0.0), egui::pos2(-1e9, 300.0)),
            (Vec3::new(100.0, 0.0, 0.0), egui::pos2(1e9, 300.0)),
        ];
        mesh.ribbon(&run);
        let vertices = vertices_of(&mesh);
        assert_eq!(vertices.len(), 6);
        assert!(
            vertices
                .iter()
                .all(|v| v[..2].iter().all(|x| x.is_finite() && x.abs() < 2000.0))
        );
        assert!(
            vertices
                .iter()
                .all(|v| v[7..10].iter().all(|x| x.is_finite()))
        );
    }

    #[test]
    fn an_arrowhead_carries_its_distances_to_its_edges() {
        let camera = Camera::default();
        let depth = DepthMap::for_camera(&camera, rect(), 100.0);
        let mut mesh = mesh(&camera, &depth);
        mesh.arrow(Vec2::new(5.0, -3.0), Vec2::new(0.6, 0.8), 0.75);
        let vertices = vertices_of(&mesh);
        assert_eq!(vertices.len(), 3);
        let corners: Vec<Vec2> = vertices.iter().map(|v| Vec2::new(v[0], v[1])).collect();
        // The world's (0.6, 0.8) is right and up the screen: the tip leads that way.
        let centroid = (corners[0] + corners[1] + corners[2]) / 3.0;
        let lead = (corners[0] - centroid).normalized();
        assert!((lead - Vec2::new(0.6, -0.8)).length() < 1e-3);
        // Its size in device pixels, the feather's half more to each edge.
        let length = (corners[0] - (corners[1] + corners[2]) / 2.0).length();
        assert!((length - 2.0 * ARROW_LENGTH).abs() < 2.5, "length {length}");
        for (i, v) in vertices.iter().enumerate() {
            assert_eq!((v[6], v[10]), (1.0, 0.75));
            // Each corner is on two of the edges, and its height from the third.
            let (b, c) = (corners[(i + 1) % 3], corners[(i + 2) % 3]);
            let edge = (c - b).normalized();
            let from = corners[i] - b;
            let height = (edge.x * from.y - edge.y * from.x).abs();
            for (j, d) in v[3..6].iter().enumerate() {
                let want = if i == j { height } else { 0.0 };
                assert!(
                    (d - want).abs() < 1e-3,
                    "corner {i}, edge {j}: {d}, want {want}"
                );
            }
        }
    }

    #[test]
    fn a_pan_draws_the_same_points_and_arrowheads() {
        let bodies = planet_and_moon();
        let mut cache = None;
        let lines = streamlines(&bodies, 1.0, &mut cache);
        let depth_of = |camera: &Camera| DepthMap::for_camera(camera, rect(), 100.0);
        let home = Camera {
            scale: 6.0,
            ..Camera::default()
        };
        // Panned by a whole number of device pixels, so the shift is exact.
        let mut panned = home;
        panned.center.x += 3.5;
        panned.center.y -= 2.0;
        let shift = Vec2::new(-3.5, -2.0) * 6.0 * 2.0;
        let (home_depth, panned_depth) = (depth_of(&home), depth_of(&panned));
        let frame = |camera: &Camera, depth: &DepthMap| -> Vec<[f32; VERTEX_FLOATS]> {
            let (vertices, _) = build(camera, rect(), lines, Some(depth), 2.0, 0.0).expect("lines");
            vertices.as_chunks::<VERTEX_FLOATS>().0.to_vec()
        };
        let (before, after) = (frame(&home, &home_depth), frame(&panned, &panned_depth));
        // Every vertex on the canvas after the pan was drawn before it, the shift away,
        // with the same distances and opacity: the pan picked no other points of the
        // lines, and moved no arrowhead along its line or changed how much of it shows.
        let on_canvas = |v: &[f32; VERTEX_FLOATS]| {
            (0.0..=1600.0).contains(&v[0]) && (0.0..=1200.0).contains(&v[1])
        };
        let key = |x: f32, y: f32| ((x * 8.0).round() as i64, (y * 8.0).round() as i64);
        let mut drawn_before: HashMap<(i64, i64), Vec<[f32; VERTEX_FLOATS]>> = HashMap::new();
        for v in &before {
            drawn_before.entry(key(v[0], v[1])).or_default().push(*v);
        }
        let mut checked = 0;
        for v in after.iter().filter(|v| on_canvas(v)) {
            // Where it was, to a hundredth of a pixel, among the buckets around there.
            let was = [v[0] - shift.x, v[1] - shift.y];
            let (kx, ky) = key(was[0], was[1]);
            let same = |w: &&[f32; VERTEX_FLOATS]| {
                (w[0] - was[0]).abs() < 1e-2
                    && (w[1] - was[1]).abs() < 1e-2
                    && (3..7).chain([10]).all(|i| (w[i] - v[i]).abs() < 1e-2)
            };
            let found = (-1..=1)
                .flat_map(|dx| (-1..=1).map(move |dy| (kx + dx, ky + dy)))
                .filter_map(|k| drawn_before.get(&k))
                .flatten()
                .any(|w| same(&w));
            assert!(
                found,
                "no vertex like the one at {:?} before the pan",
                &v[..2]
            );
            checked += 1;
        }
        assert!(checked > 1000, "only {checked} vertices on the canvas");
    }

    #[test]
    fn zooming_in_only_adds_points() {
        let lines = trace_all(&planet_and_moon(), 1.0);
        for line in &lines {
            let at = |scale: f32| -> Vec<usize> {
                drawn(line, move |_| TOLERANCE / (scale * 2.0)).collect()
            };
            let mut coarser = at(0.01);
            for scale in [0.1, 1.0, 6.0, 6.1, 50.0, 1000.0] {
                let finer = at(scale);
                assert!(coarser.iter().all(|i| finer.binary_search(i).is_ok()));
                coarser = finer;
            }
        }
    }

    #[test]
    fn nothing_to_draw_edge_on_or_without_lines() {
        let bodies = planet_and_moon();
        let mut cache = None;
        let lines = streamlines(&bodies, 1.0, &mut cache);
        let edge_on = Camera {
            orbit: Orbit {
                azimuth: 0.3,
                elevation: 0.0,
            },
            ..Camera::default()
        };
        assert!(paint(&edge_on, rect(), lines, None, 1.0, 0.0).is_none());
        let mut none = None;
        let empty = streamlines(&[], 1.0, &mut none);
        assert!(paint(&Camera::default(), rect(), empty, None, 1.0, 0.0).is_none());
    }
}
