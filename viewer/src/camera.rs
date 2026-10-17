//! View camera: an orthographic camera that orbits a center point, and maps between world
//! coordinates and screen pixels (y-down).
//!
//! Supports panning, cursor-anchored zoom and turntable orbiting (azimuth about the world z
//! axis, elevation above the xy plane).
//!
//! World convention: x right and y up (matches the ship simulation), z out of the plane
//! toward the default viewer. The default view looks straight down at the plane from +z,
//! with +y up the screen: the 2D view.
//! Screen convention: positive y points down (egui).

use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::ops::{Add, Mul, Neg, Sub};

use egui::{Pos2, Rect, Vec2};

const MIN_SCALE: f32 = 0.01;
const MAX_SCALE: f32 = 1000.0;

/// A point or direction in the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    pub const X: Self = Self::new(1.0, 0.0, 0.0);
    pub const Y: Self = Self::new(0.0, 1.0, 0.0);
    pub const Z: Self = Self::new(0.0, 0.0, 1.0);

    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// The point `v` of the simulation's plane, `z = 0`.
    pub const fn plane(v: Vec2) -> Self {
        Self::new(v.x, v.y, 0.0)
    }

    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    pub fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, k: f32) -> Self {
        Self::new(self.x * k, self.y * k, self.z * k)
    }
}

impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

/// Which way the camera looks, turntable style.
///
/// The camera sits at `azimuth` around the world z axis and `elevation` above the xy
/// plane, and looks at the center, with world +z up the screen (+y when looking straight
/// down).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Orbit {
    /// Radians; 0 looks from -y toward +y, `PI / 2` from +x toward -x. In (-PI, PI].
    pub azimuth: f32,
    /// Radians, from `-PI / 2` (from below) to `PI / 2` (from above).
    pub elevation: f32,
}

impl Default for Orbit {
    fn default() -> Self {
        Self::TOP
    }
}

impl Orbit {
    /// Straight down from +z, with +x right and +y up the screen: the 2D view.
    pub const TOP: Self = Self {
        azimuth: 0.0,
        elevation: FRAC_PI_2,
    };

    /// The view from the side `toward_viewer` points to (a unit vector), looking back
    /// along it. Seen from straight above or below, +x is to the right.
    pub fn facing(toward_viewer: Vec3) -> Self {
        let (azimuth, elevation) = if toward_viewer.z.abs() > 0.9999 {
            (0.0, FRAC_PI_2.copysign(toward_viewer.z))
        } else {
            (
                toward_viewer.x.atan2(-toward_viewer.y),
                toward_viewer.z.asin(),
            )
        };
        Self { azimuth, elevation }.normalized()
    }

    /// The same view, with the azimuth wrapped into (-PI, PI] and the elevation in range.
    pub fn normalized(self) -> Self {
        Self {
            azimuth: wrap_angle(self.azimuth),
            elevation: self.elevation.clamp(-FRAC_PI_2, FRAC_PI_2),
        }
    }

    /// The screen's right and up directions, and the direction toward the viewer, in the
    /// world: a right-handed orthonormal basis.
    pub fn basis(self) -> [Vec3; 3] {
        let (sin_a, cos_a) = self.azimuth.sin_cos();
        let (sin_e, cos_e) = self.elevation.sin_cos();
        let right = Vec3::new(cos_a, sin_a, 0.0);
        let up = Vec3::new(-sin_a * sin_e, cos_a * sin_e, cos_e);
        let toward_viewer = Vec3::new(sin_a * cos_e, -cos_a * cos_e, sin_e);
        [right, up, toward_viewer]
    }

    pub fn toward_viewer(self) -> Vec3 {
        self.basis()[2]
    }

    /// The view a fraction `f` (0 to 1) of the way to `target`, the short way round.
    pub fn toward(self, target: Self, f: f32) -> Self {
        Self {
            azimuth: self.azimuth + wrap_angle(target.azimuth - self.azimuth) * f,
            elevation: self.elevation + (target.elevation - self.elevation) * f,
        }
        .normalized()
    }

    /// Whether the two views differ by less than `angle` radians in both angles.
    pub fn is_near(self, other: Self, angle: f32) -> bool {
        wrap_angle(self.azimuth - other.azimuth).abs() < angle
            && (self.elevation - other.elevation).abs() < angle
    }
}

/// `angle` wrapped into (-PI, PI].
fn wrap_angle(angle: f32) -> f32 {
    let wrapped = angle.rem_euclid(TAU);
    if wrapped > PI { wrapped - TAU } else { wrapped }
}

/// Field of view of the perspective camera across the canvas's longer side: that of
/// Blender's default camera, a 50 mm lens on a 36 mm sensor, `2 atan(18 / 50)`.
const FIELD_OF_VIEW: f32 = 0.691_150_4;
/// The perspective camera's near clipping plane, as a fraction of the eye's distance to the
/// center: nothing nearer the eye is drawn.
const NEAR: f32 = 0.02;

#[derive(Debug, Clone, Copy)]
pub struct Camera {
    /// World point that maps to the screen rect's center; the camera orbits around it.
    pub center: Vec3,
    /// Screen pixels per world unit, on the plane through the center that faces the viewer.
    /// Always positive.
    pub scale: f32,
    pub orbit: Orbit,
    /// Perspective projection rather than orthographic.
    pub perspective: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: Vec3::ZERO,
            scale: 4.0,
            orbit: Orbit::TOP,
            perspective: false,
        }
    }
}

impl Camera {
    /// In perspective, how far the eye is from the center, toward the viewer: as far as
    /// makes the plane through the center facing the viewer [`Self::scale`] pixels per world
    /// unit in `rect`, as in orthographic, so switching projections keeps it the same size.
    /// Zooming in moves the eye closer. `None` in orthographic.
    pub fn eye_distance(&self, rect: Rect) -> Option<f32> {
        self.perspective.then(|| {
            rect.width().max(rect.height()) / (2.0 * self.scale * (FIELD_OF_VIEW / 2.0).tan())
        })
    }

    /// World point `p` in view coordinates, from the center: across the screen, up it, and
    /// toward the viewer.
    pub fn view(&self, p: Vec3) -> Vec3 {
        let [right, up, toward_viewer] = self.orbit.basis();
        let v = p - self.center;
        Vec3::new(v.dot(right), v.dot(up), v.dot(toward_viewer))
    }

    /// Screen pixels per world unit at view depth `z` (toward the viewer from the center),
    /// or `None` nearer the eye than the near plane, or behind it.
    fn scale_at_depth(&self, z: f32, rect: Rect) -> Option<f32> {
        match self.eye_distance(rect) {
            None => Some(self.scale),
            Some(d) => {
                let distance = d - z;
                (distance > NEAR * d).then(|| self.scale * d / distance)
            }
        }
    }

    /// Screen pixels per world unit at world point `p`, or `None` when it is not in front
    /// of the eye.
    pub fn scale_at(&self, p: Vec3, rect: Rect) -> Option<f32> {
        self.scale_at_depth(self.view(p).z, rect)
    }

    /// Screen position of world point `world`, or `None` when it is not in front of the eye.
    pub fn project(&self, world: Vec3, rect: Rect) -> Option<Pos2> {
        let v = self.view(world);
        let scale = self.scale_at_depth(v.z, rect)?;
        Some(rect.center() + Vec2::new(v.x, -v.y) * scale)
    }

    /// Screen position of point `world` of the plane `z = 0`, or `None` when it is not in
    /// front of the eye.
    pub fn world_to_screen(&self, world: Vec2, rect: Rect) -> Option<Pos2> {
        self.project(Vec3::plane(world), rect)
    }

    /// How the screen position of world point `p` moves, in pixels, as `p` moves along `v`
    /// (by `v`, if small): the projection's derivative. `None` when `p` is not in front of
    /// the eye.
    pub fn screen_vector_at(&self, p: Vec3, v: Vec3, rect: Rect) -> Option<Vec2> {
        let q = self.view(p);
        let scale = self.scale_at_depth(q.z, rect)?;
        let dv = self.view(self.center + v);
        // Moving toward the eye also spreads the point away from the center.
        let spread = self.eye_distance(rect).map_or(0.0, |d| dv.z / (d - q.z));
        Some(Vec2::new(dv.x + spread * q.x, -(dv.y + spread * q.y)) * scale)
    }

    /// The polyline through world points `points` on screen, as the runs of it in front of
    /// the eye, cut where it crosses the near plane.
    pub fn project_polyline(
        &self,
        points: impl IntoIterator<Item = Vec3>,
        rect: Rect,
    ) -> Vec<Vec<Pos2>> {
        // Deepest view depth in front of the near plane, a hair inside it.
        let limit = self.eye_distance(rect).map(|d| d * (1.0 - NEAR * 1.001));
        let inside = |z: f32| limit.is_none_or(|limit| z < limit);
        let mut runs = Vec::new();
        let mut run = Vec::new();
        let mut previous: Option<(Vec3, f32)> = None;
        for p in points {
            let z = self.view(p).z;
            if let (Some((q, zq)), Some(limit)) = (previous, limit)
                && inside(z) != inside(zq)
            {
                let crossing = q + (p - q) * ((limit - zq) / (z - zq));
                run.extend(self.project(crossing, rect));
                if inside(zq) {
                    runs.push(std::mem::take(&mut run));
                }
            }
            if inside(z) {
                run.extend(self.project(p, rect));
            }
            previous = Some((p, z));
        }
        if !run.is_empty() {
            runs.push(run);
        }
        runs
    }

    /// The world point at `screen` on the plane through the center that faces the viewer.
    fn screen_to_view_plane(&self, screen: Pos2, rect: Rect) -> Vec3 {
        let [right, up, _] = self.orbit.basis();
        let d = (screen - rect.center()) / self.scale;
        self.center + right * d.x - up * d.y
    }

    /// The point of the plane `z = 0` seen at `screen`, or `None` when the view is edge on
    /// to the plane, or, in perspective, when it is not in front of the eye there (above
    /// the horizon).
    pub fn screen_to_world(&self, screen: Pos2, rect: Rect) -> Option<Vec2> {
        let toward_viewer = self.orbit.toward_viewer();
        let q = self.screen_to_view_plane(screen, rect);
        // The line of sight through `q`: parallel to the view, or from the eye.
        let (origin, direction) = match self.eye_distance(rect) {
            None => (q, -toward_viewer),
            Some(d) => {
                let eye = self.center + toward_viewer * d;
                (eye, q - eye)
            }
        };
        if direction.z.abs() < 1e-4 * direction.length() {
            return None;
        }
        let t = -origin.z / direction.z;
        if self.perspective && t <= 0.0 {
            return None;
        }
        let p = origin + direction * t;
        Some(Vec2::new(p.x, p.y))
    }

    /// Translate the view by a screen-space drag delta (egui convention:
    /// positive y is downward). The scene appears to follow the cursor.
    pub fn pan_by_screen(&mut self, delta: Vec2) {
        let [right, up, _] = self.orbit.basis();
        self.center = self.center - right * (delta.x / self.scale) + up * (delta.y / self.scale);
    }

    /// Multiply scale by `factor`, keeping the world point under
    /// `screen_anchor` (on the plane through the center facing the viewer) fixed on screen.
    pub fn zoom_at(&mut self, factor: f32, screen_anchor: Pos2, rect: Rect) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let world_anchor = self.screen_to_view_plane(screen_anchor, rect);
        self.scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        let [right, up, _] = self.orbit.basis();
        let d = (screen_anchor - rect.center()) / self.scale;
        self.center = world_anchor - right * d.x + up * d.y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_min_size(Pos2::new(10.0, 20.0), egui::vec2(800.0, 600.0))
    }

    fn assert_near(a: Vec2, b: Vec2, eps: f32) {
        assert!((a - b).length() < eps, "expected {a:?} ~ {b:?}");
    }

    fn assert_near3(a: Vec3, b: Vec3) {
        assert!((a - b).dot(a - b) < 1e-10, "expected {a:?} ~ {b:?}");
    }

    /// A tilted view, not along any axis.
    fn tilted() -> Camera {
        Camera {
            center: Vec3::new(1.0, -2.0, 0.5),
            scale: 25.0,
            orbit: Orbit {
                azimuth: 0.6,
                elevation: 0.4,
            },
            perspective: false,
        }
    }

    /// `camera` in perspective.
    fn in_perspective(camera: Camera) -> Camera {
        Camera {
            perspective: true,
            ..camera
        }
    }

    fn project(cam: &Camera, p: Vec3) -> Pos2 {
        cam.project(p, rect()).expect("in front of the eye")
    }

    #[test]
    fn round_trip_world_screen() {
        for orbit in [Orbit::TOP, tilted().orbit] {
            for perspective in [false, true] {
                let cam = Camera {
                    center: Vec3::new(3.0, -2.5, 0.0),
                    scale: 25.0,
                    orbit,
                    perspective,
                };
                let r = rect();
                for &(x, y) in &[(0.0, 0.0), (1.5, 4.2), (-10.0, 7.7), (12.456, -7.9)] {
                    let w = Vec2::new(x, y);
                    let s = cam.world_to_screen(w, r).expect("in front of the eye");
                    let w2 = cam.screen_to_world(s, r).expect("the plane is not edge on");
                    assert_near(w, w2, 1e-3);
                }
            }
        }
    }

    #[test]
    fn top_view_is_the_2d_view() {
        let cam = Camera {
            center: Vec3::new(3.0, -2.5, 0.0),
            scale: 25.0,
            ..Camera::default()
        };
        let p = project(&cam, Vec3::new(4.0, -1.5, 0.0));
        assert_near(
            p.to_vec2(),
            (rect().center() + egui::vec2(25.0, -25.0)).to_vec2(),
            1e-4,
        );
    }

    #[test]
    fn perspective_keeps_the_center_plane_and_enlarges_what_is_nearer() {
        let ortho = tilted();
        let cam = in_perspective(ortho);
        let [right, up, toward_viewer] = cam.orbit.basis();
        // On the plane through the center facing the viewer, the two projections agree.
        let on_plane = cam.center + right * 3.0 - up * 2.0;
        assert_near(
            project(&cam, on_plane).to_vec2(),
            project(&ortho, on_plane).to_vec2(),
            1e-3,
        );
        // Nearer the eye, farther from the center of the screen; farther, nearer it.
        let c = rect().center();
        let nearer = on_plane + toward_viewer * 2.0;
        let farther = on_plane - toward_viewer * 2.0;
        assert!(project(&cam, nearer).distance(c) > project(&ortho, nearer).distance(c));
        assert!(project(&cam, farther).distance(c) < project(&ortho, farther).distance(c));
    }

    #[test]
    fn nothing_behind_the_eye_is_projected() {
        let cam = in_perspective(tilted());
        let d = cam.eye_distance(rect()).expect("perspective");
        let toward_viewer = cam.orbit.toward_viewer();
        assert!(
            cam.project(cam.center + toward_viewer * (d * 1.5), rect())
                .is_none()
        );
        assert!(
            cam.project(cam.center + toward_viewer * (d * 0.5), rect())
                .is_some()
        );
    }

    #[test]
    fn polyline_is_cut_at_the_near_plane() {
        let cam = in_perspective(tilted());
        let d = cam.eye_distance(rect()).expect("perspective");
        let [right, _, toward_viewer] = cam.orbit.basis();
        let front = |x: f32| cam.center + right * x;
        let behind = cam.center + toward_viewer * (2.0 * d);
        let runs = cam.project_polyline([front(-1.0), behind, front(1.0)], rect());
        assert_eq!(runs.len(), 2);
        assert!(runs.iter().all(|run| run.len() == 2));
        // In orthographic, nothing is cut.
        let runs = tilted().project_polyline([front(-1.0), behind, front(1.0)], rect());
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].len(), 3);
    }

    #[test]
    fn screen_vector_is_the_projections_derivative() {
        for cam in [tilted(), in_perspective(tilted())] {
            let p = Vec3::new(2.0, 1.0, 0.3);
            let v = Vec3::new(0.3, -0.5, 0.8);
            let h = 1e-3;
            let numeric = (project(&cam, p + v * h) - project(&cam, p)) / h;
            let exact = cam
                .screen_vector_at(p, v, rect())
                .expect("in front of the eye");
            assert_near(numeric, exact, 0.05);
        }
    }

    #[test]
    fn edge_on_has_no_plane_point() {
        let cam = Camera {
            orbit: Orbit::facing(Vec3::X),
            ..Camera::default()
        };
        assert_eq!(cam.screen_to_world(Pos2::new(100.0, 100.0), rect()), None);
    }

    #[test]
    fn perspective_has_no_plane_point_above_the_horizon() {
        // Looking almost level, the top of the screen is sky.
        let cam = in_perspective(Camera {
            orbit: Orbit {
                azimuth: 0.0,
                elevation: 0.05,
            },
            ..Camera::default()
        });
        assert_eq!(cam.screen_to_world(rect().center_top(), rect()), None);
        assert!(
            cam.screen_to_world(rect().center_bottom(), rect())
                .is_some()
        );
    }

    #[test]
    fn zoom_anchor_is_fixed_point() {
        for mut cam in [Camera::default(), tilted(), in_perspective(tilted())] {
            let r = rect();
            let anchor = Pos2::new(200.0, 150.0);
            let world_before = cam.screen_to_view_plane(anchor, r);
            cam.zoom_at(2.5, anchor, r);
            assert_near(
                project(&cam, world_before).to_vec2(),
                anchor.to_vec2(),
                1e-3,
            );
        }
    }

    #[test]
    fn pan_is_invertible_and_follows_the_cursor() {
        for mut cam in [Camera::default(), tilted(), in_perspective(tilted())] {
            // On the plane through the center facing the viewer, which pans with the cursor
            // in perspective too.
            let [right, up, _] = cam.orbit.basis();
            let point = cam.center + right * 2.0 + up * 3.0;
            let before = project(&cam, point);
            let center = cam.center;
            cam.pan_by_screen(Vec2::new(40.0, -30.0));
            assert_near(
                project(&cam, point).to_vec2(),
                (before + Vec2::new(40.0, -30.0)).to_vec2(),
                1e-3,
            );
            cam.pan_by_screen(Vec2::new(-40.0, 30.0));
            assert_near3(cam.center, center);
        }
    }

    #[test]
    fn scale_is_clamped() {
        let r = rect();
        let mut cam = Camera::default();
        cam.zoom_at(1e9, Pos2::new(100.0, 100.0), r);
        assert!(cam.scale <= MAX_SCALE);
        cam.zoom_at(1e-9, Pos2::new(100.0, 100.0), r);
        assert!(cam.scale >= MIN_SCALE);
    }

    #[test]
    fn y_axis_is_world_up() {
        let cam = Camera::default();
        let s_origin = project(&cam, Vec3::ZERO);
        let s_up = project(&cam, Vec3::Y);
        assert!(s_up.y < s_origin.y, "world +y should map to screen up");
    }

    #[test]
    fn basis_is_right_handed_and_orthonormal() {
        for orbit in [Orbit::TOP, tilted().orbit, Orbit::facing(-Vec3::Z)] {
            let [right, up, toward_viewer] = orbit.basis();
            for v in [right, up, toward_viewer] {
                assert!((v.dot(v) - 1.0).abs() < 1e-5);
            }
            assert!(right.dot(up).abs() < 1e-5);
            assert_near3(right.cross(up), toward_viewer);
        }
    }

    #[test]
    fn facing_an_axis_looks_back_along_it() {
        for axis in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z] {
            assert_near3(Orbit::facing(axis).toward_viewer(), axis);
        }
        assert_eq!(Orbit::facing(Vec3::Z), Orbit::TOP);
        // Side views keep +z up the screen.
        for axis in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y] {
            assert_near3(Orbit::facing(axis).basis()[1], Vec3::Z);
        }
    }

    #[test]
    fn toward_goes_the_short_way_round() {
        let a = Orbit {
            azimuth: 3.0,
            elevation: 0.0,
        };
        let b = Orbit {
            azimuth: -3.0,
            elevation: 0.0,
        };
        let half = a.toward(b, 0.5);
        assert!(half.azimuth.abs() > 3.0, "{half:?} should pass through PI");
        assert!(a.toward(b, 1.0).is_near(b, 1e-5));
    }
}
