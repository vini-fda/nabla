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

#[derive(Debug, Clone, Copy)]
pub struct Camera {
    /// World point that maps to the screen rect's center; the camera orbits around it.
    pub center: Vec3,
    /// Screen pixels per world unit. Always positive.
    pub scale: f32,
    pub orbit: Orbit,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: Vec3::ZERO,
            scale: 4.0,
            orbit: Orbit::TOP,
        }
    }
}

impl Camera {
    /// Screen position of world point `world`.
    pub fn project(&self, world: Vec3, rect: Rect) -> Pos2 {
        rect.center() + self.screen_vector(world - self.center)
    }

    /// Screen position of point `world` of the plane `z = 0`.
    pub fn world_to_screen(&self, world: Vec2, rect: Rect) -> Pos2 {
        self.project(Vec3::plane(world), rect)
    }

    /// Screen displacement, in pixels, of the world displacement `v`.
    pub fn screen_vector(&self, v: Vec3) -> Vec2 {
        let [right, up, _] = self.orbit.basis();
        Vec2::new(v.dot(right), -v.dot(up)) * self.scale
    }

    /// The world point at `screen` on the plane through the center that faces the viewer.
    fn screen_to_view_plane(&self, screen: Pos2, rect: Rect) -> Vec3 {
        let [right, up, _] = self.orbit.basis();
        let d = (screen - rect.center()) / self.scale;
        self.center + right * d.x - up * d.y
    }

    /// The point of the plane `z = 0` seen at `screen`, or `None` when the view is edge on
    /// to the plane.
    pub fn screen_to_world(&self, screen: Pos2, rect: Rect) -> Option<Vec2> {
        let toward_viewer = self.orbit.toward_viewer();
        if toward_viewer.z.abs() < 1e-4 {
            return None;
        }
        let q = self.screen_to_view_plane(screen, rect);
        let p = q - toward_viewer * (q.z / toward_viewer.z);
        Some(Vec2::new(p.x, p.y))
    }

    /// Translate the view by a screen-space drag delta (egui convention:
    /// positive y is downward). The scene appears to follow the cursor.
    pub fn pan_by_screen(&mut self, delta: Vec2) {
        let [right, up, _] = self.orbit.basis();
        self.center = self.center - right * (delta.x / self.scale) + up * (delta.y / self.scale);
    }

    /// Multiply scale by `factor`, keeping the world point under
    /// `screen_anchor` fixed on screen.
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
        }
    }

    #[test]
    fn round_trip_world_screen() {
        for orbit in [Orbit::TOP, tilted().orbit] {
            let cam = Camera {
                center: Vec3::new(3.0, -2.5, 0.0),
                scale: 25.0,
                orbit,
            };
            let r = rect();
            for &(x, y) in &[(0.0, 0.0), (1.5, 4.2), (-10.0, 7.7), (123.456, -78.9)] {
                let w = Vec2::new(x, y);
                let s = cam.world_to_screen(w, r);
                let w2 = cam.screen_to_world(s, r).expect("the plane is not edge on");
                assert_near(w, w2, 1e-3);
            }
        }
    }

    #[test]
    fn top_view_is_the_2d_view() {
        let cam = Camera {
            center: Vec3::new(3.0, -2.5, 0.0),
            scale: 25.0,
            orbit: Orbit::TOP,
        };
        let r = rect();
        let p = cam.world_to_screen(Vec2::new(4.0, -1.5), r);
        assert_near(
            p.to_vec2(),
            (r.center() + egui::vec2(25.0, -25.0)).to_vec2(),
            1e-4,
        );
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
    fn zoom_anchor_is_fixed_point() {
        for mut cam in [Camera::default(), tilted()] {
            let r = rect();
            let anchor = Pos2::new(200.0, 150.0);
            let world_before = cam.screen_to_view_plane(anchor, r);
            cam.zoom_at(2.5, anchor, r);
            assert_near(
                cam.project(world_before, r).to_vec2(),
                anchor.to_vec2(),
                1e-3,
            );
        }
    }

    #[test]
    fn pan_is_invertible_and_follows_the_cursor() {
        for mut cam in [Camera::default(), tilted()] {
            let r = rect();
            let point = Vec3::new(2.0, 3.0, -1.0);
            let before = cam.project(point, r);
            let center = cam.center;
            cam.pan_by_screen(Vec2::new(40.0, -30.0));
            assert_near(
                cam.project(point, r).to_vec2(),
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
        let r = rect();
        let s_origin = cam.world_to_screen(Vec2::ZERO, r);
        let s_up = cam.world_to_screen(Vec2::new(0.0, 1.0), r);
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
