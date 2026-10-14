//! Thruster plumes in the 3D view: a blue jet out of the main (ion) thruster at the center
//! of the tail, as strong as the thrust, and white puffs out of the reaction control
//! thrusters at the ship's six tips, as strong as the torque.
//!
//! The ship's cross-section is an equilateral triangle, so there are three tips at the nose
//! and three at the tail; seen from above, one of each three is under the hull (on the
//! center line) and the other two are on its edges. To turn the ship counterclockwise the
//! nose thrusters blow to the right and the tail thrusters to the left (and the other way
//! round to turn it clockwise), a couple that turns the ship without pushing it.
//!
//! The plumes are particle systems without state: each frame recomputes the particles
//! emitted during the last moments from the trajectory itself, so they stay consistent with
//! the timeline when the replay is paused, scrubbed or sped up. Each particle leaves its
//! nozzle at its emission time with the ship's velocity then plus the exhaust velocity, so
//! a jet bends as the ship turns. A divergence-free swirl field (the curl of a few drifting
//! waves) carries the particles more the older they get, which breaks the jets up into
//! coherent eddies like a gas, while they expand, cool and fade.
//!
//! Particles are soft sprites added onto the canvas (egui colors with zero alpha add up),
//! so where many overlap, near a nozzle, they saturate toward white.
//!
//! Lengths are in units of the ship's half length `L`, so the plumes scale with the ship.

use egui::{Color32, Pos2, Vec2};
use ship2d_optimization::State;

use crate::camera::Camera;
use crate::replay::Replay;

/// How one kind of jet looks.
struct Jet {
    /// Particles emitted per second per nozzle at full strength.
    rate: f64,
    /// Seconds a particle lives.
    lifetime: f64,
    /// Exhaust speed relative to the ship at full strength, in `L` per second.
    speed: f32,
    /// Radius of the nozzle, in `L`.
    nozzle_radius: f32,
    /// Half angle of the jet's cone, radians.
    spread: f32,
    /// How far the swirl field carries a particle by the end of its life, in `L`.
    turbulence: f32,
    /// Size of the swirls, in `L`.
    eddy_size: f32,
    /// Brightness of one fresh particle at full strength (0 to 1).
    gain: f32,
    /// How much longer than wide a fresh particle is, smeared along its flight.
    streak: f32,
    /// Radius of a fresh particle, and how much it grows over its life, in `L`.
    size: (f32, f32),
    /// Colors along a particle's life: fresh, early, late (0 to 255).
    colors: [[f32; 3]; 3],
}

/// The main thruster: a long, blue ion jet.
const MAIN: Jet = Jet {
    rate: 1600.0,
    lifetime: 0.4,
    speed: 9.0,
    nozzle_radius: 0.12,
    spread: 0.16,
    turbulence: 0.55,
    eddy_size: 0.9,
    gain: 0.12,
    streak: 2.0,
    size: (0.07, 0.5),
    colors: [
        [205.0, 240.0, 255.0],
        [60.0, 185.0, 255.0],
        [25.0, 70.0, 230.0],
    ],
};

/// The reaction control thrusters: short, wide, white puffs of cold gas.
const RCS: Jet = Jet {
    rate: 500.0,
    lifetime: 0.2,
    speed: 5.0,
    nozzle_radius: 0.015,
    spread: 0.3,
    turbulence: 0.12,
    eddy_size: 0.4,
    gain: 0.2,
    streak: 2.5,
    size: (0.025, 0.22),
    colors: [
        [255.0, 255.0, 255.0],
        [215.0, 225.0, 240.0],
        [120.0, 130.0, 150.0],
    ],
};

/// Distance of the main nozzle behind the ship's center, in `L`.
const MAIN_NOZZLE: f32 = 0.98;

/// The reaction control thrusters, in model coordinates (`L`; x to the ship's right, y to
/// its nose) as seen from above, and whether each is on the visible edge rather than under
/// the hull. The tips are the corners of equilateral triangles: of circumradius 0.14 at the
/// nose and 0.38 at the tail, one corner under the hull and two at +-30 degrees above it.
const RCS_NOZZLES: [(f32, f32, bool); 6] = [
    (0.0, 0.92, false),
    (0.121, 0.92, true),
    (-0.121, 0.92, true),
    (0.0, -0.96, false),
    (0.329, -0.96, true),
    (-0.329, -0.96, true),
];

/// How fast the swirls drift and change, per second.
const EDDY_SPEED: f32 = 1.7;

/// A nozzle at an instant: where it is (world), and which way its exhaust goes.
#[derive(Clone, Copy)]
struct Nozzle {
    position: Vec2,
    direction: Vec2,
}

/// The sprite texture, a soft round blob.
pub struct Plume {
    sprite: egui::TextureHandle,
}

impl Plume {
    pub fn new(ctx: &egui::Context) -> Self {
        const SIZE: usize = 64;
        let pixels = (0..SIZE * SIZE)
            .map(|i| {
                let (x, y) = ((i % SIZE) as f32, (i / SIZE) as f32);
                let d = Vec2::new(x + 0.5, y + 0.5) / SIZE as f32 * 2.0 - Vec2::splat(1.0);
                // Gaussian, cut off smoothly at the edge.
                let r2 = d.length_sq();
                let a = (-4.0 * r2).exp() * (1.0 - r2).max(0.0);
                let v = (a * 255.0).round() as u8;
                Color32::from_rgba_premultiplied(v, v, v, v)
            })
            .collect();
        let image = egui::ColorImage::new([SIZE, SIZE], pixels);
        Self {
            sprite: ctx.load_texture("plume_sprite", image, egui::TextureOptions::LINEAR),
        }
    }

    /// The jets at time `t`, from a ship of half length `half_length`. Paint them before
    /// the ship, which covers their start.
    pub fn paint_jets(
        &self,
        painter: &egui::Painter,
        camera: &Camera,
        canvas: egui::Rect,
        replay: &Replay,
        t: f64,
        half_length: f32,
    ) {
        let l = half_length;
        let mut mesh = egui::Mesh::with_texture(self.sprite.id());

        // The main jet's luminous body: a soft cone from the nozzle, and a brighter core.
        let thrust = smoothed(t, |t| replay.thrust_fraction(t));
        if thrust > 0.0 {
            let [nozzle] = main_nozzle(&replay.state_at_time(t), l);
            let (along, across) = screen_axes(-nozzle.direction, l * camera.scale);
            let [hot, warm, _] = MAIN.colors;
            for (distance, length, width, color, gain) in
                [(0.9, 1.0, 0.28, warm, 0.22), (0.45, 0.5, 0.11, hot, 0.3)]
            {
                let center = nozzle.position + nozzle.direction * distance * l;
                let center = camera.world_to_screen(center, canvas);
                let color = additive(color, gain * thrust);
                add_sprite(&mut mesh, center, along * length, across * width, color);
            }
        }

        let view = View {
            camera,
            canvas,
            replay,
            t,
            l,
        };
        view.particles(
            &mut mesh,
            &MAIN,
            0,
            |t| replay.thrust_fraction(t),
            |s, _| main_nozzle(s, l),
        );
        view.particles(
            &mut mesh,
            &RCS,
            1,
            |t| replay.torque_fraction(t),
            |s, torque| rcs_nozzles(s, torque, l),
        );
        painter.add(egui::Shape::mesh(mesh));
    }

    /// The glows where the jets leave their nozzles at time `t`. Paint them over the ship.
    pub fn paint_nozzles(
        &self,
        painter: &egui::Painter,
        camera: &Camera,
        canvas: egui::Rect,
        replay: &Replay,
        t: f64,
        half_length: f32,
    ) {
        let l = half_length;
        let s = replay.state_at_time(t);
        let mut mesh = egui::Mesh::with_texture(self.sprite.id());

        let thrust = smoothed(t, |t| replay.thrust_fraction(t));
        if thrust > 0.0 {
            let [nozzle] = main_nozzle(&s, l);
            let center = camera.world_to_screen(nozzle.position, canvas);
            let (along, across) = screen_axes(-nozzle.direction, l * camera.scale);
            let [hot, warm, _] = MAIN.colors;
            // A wide dim halo, the glowing ring seen edge on, and its white-hot core.
            add_sprite(
                &mut mesh,
                center,
                along * 0.45,
                across * 0.55,
                additive(warm, 0.35 * thrust),
            );
            add_sprite(
                &mut mesh,
                center,
                along * 0.09,
                across * 0.28,
                additive(hot, 0.8 * thrust),
            );
            let white = [255.0, 255.0, 255.0];
            add_sprite(
                &mut mesh,
                center,
                along * 0.05,
                across * 0.12,
                additive(white, 0.9 * thrust),
            );
        }

        // A flash at each visible reaction control nozzle that is firing.
        let torque = smoothed(t, |t| replay.torque_fraction(t));
        if torque != 0.0 {
            let strength = torque.abs();
            let r = 0.07 * l * camera.scale * strength.sqrt();
            let nozzles = rcs_nozzles(&s, torque, l);
            for (nozzle, &(_, _, visible)) in nozzles.iter().zip(&RCS_NOZZLES) {
                if visible {
                    let center = camera.world_to_screen(nozzle.position, canvas);
                    let color = additive(RCS.colors[0], 0.9 * strength);
                    add_sprite(
                        &mut mesh,
                        center,
                        Vec2::new(r, 0.0),
                        Vec2::new(0.0, r),
                        color,
                    );
                }
            }
        }
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// What the particles are drawn for: time `t` of `replay`, a ship of half length `l`, seen
/// through `camera` in `canvas`.
struct View<'a> {
    camera: &'a Camera,
    canvas: egui::Rect,
    replay: &'a Replay,
    t: f64,
    l: f32,
}

impl View<'_> {
    /// Adds the live particles of `jet` to `mesh`. `strength` gives how hard the jet fires
    /// at a time (-1 to 1; the sign is the nozzles' business), and `nozzles` where its
    /// nozzles are and which way they blow, given the state and strength then. `salt` keeps
    /// different jets' particles from sharing random numbers.
    fn particles<const N: usize>(
        &self,
        mesh: &mut egui::Mesh,
        jet: &Jet,
        salt: u64,
        strength: impl Fn(f64) -> f64,
        nozzles: impl Fn(&State, f32) -> [Nozzle; N],
    ) {
        let Self {
            camera,
            canvas,
            replay,
            t,
            l,
        } = *self;
        let first = ((t - jet.lifetime) * jet.rate).ceil().max(0.0) as u64;
        let last = (t.min(replay.duration()) * jet.rate).floor().max(0.0) as u64;
        for i in first..=last {
            let emitted = i as f64 / jet.rate;
            let signed = strength(emitted) as f32;
            let power = signed.abs();
            if power <= 0.0 {
                continue;
            }
            let age = (t - emitted) as f32;
            let life = age / jet.lifetime as f32;
            let s = replay.state_at_time(emitted);
            let ship_velocity = Vec2::new(s.vx as f32, s.vy as f32);

            for (j, nozzle) in nozzles(&s, signed).into_iter().enumerate() {
                let [gate, lateral, angle, speed, size] =
                    random((i * N as u64 + j as u64) ^ (salt << 56));
                // Fewer particles at lower strength.
                if gate >= power {
                    continue;
                }
                let across = nozzle.direction.rot90();
                let start = nozzle.position + across * lateral * jet.nozzle_radius * l;
                // Off-axis particles lean outward, which opens the jet into a cone.
                let direction =
                    rotate(nozzle.direction, jet.spread * (0.6 * angle + 0.4 * lateral));
                let exhaust = jet.speed * l * (0.55 + 0.45 * power) * (1.0 + 0.15 * speed);
                let mut p = start + (ship_velocity + direction * exhaust) * age;
                let eddies = swirl(p / (jet.eddy_size * l), t as f32 * EDDY_SPEED);
                p += eddies * jet.turbulence * l * life.powf(1.5);

                let fade = (1.0 - life).powi(2) * (life * 12.0).min(1.0);
                let color = additive(
                    life_color(&jet.colors, life),
                    jet.gain * power.sqrt() * fade,
                );
                let radius = l * (jet.size.0 + jet.size.1 * life) * (1.0 + 0.3 * size);
                let center = camera.world_to_screen(p, canvas);
                let (along, across) = screen_axes(direction, radius * camera.scale);
                let streak = 1.0 + (jet.streak - 1.0) * (1.0 - life);
                add_sprite(mesh, center, along * streak, across, color);
            }
        }
    }
}

/// The main thruster in state `s`: behind the tail, blowing backward.
fn main_nozzle(s: &State, l: f32) -> [Nozzle; 1] {
    let heading = Vec2::angled(s.theta as f32);
    [Nozzle {
        position: position(s) - heading * MAIN_NOZZLE * l,
        direction: -heading,
    }]
}

/// The reaction control thrusters in state `s` under torque fraction `torque`: the nose
/// ones blowing to the ship's right and the tail ones to its left for a counterclockwise
/// (positive) torque, the other way round for a clockwise one.
fn rcs_nozzles(s: &State, torque: f32, l: f32) -> [Nozzle; 6] {
    let heading = Vec2::angled(s.theta as f32);
    // `rot90` turns +x to -y: clockwise in the world (y up), so to the ship's right.
    let right = heading.rot90();
    let sign = torque.signum();
    RCS_NOZZLES.map(|(x, y, _)| Nozzle {
        position: position(s) + (heading * y + right * x) * l,
        // At the nose (y > 0) to the right for positive torque, at the tail to the left.
        direction: right * sign * y.signum(),
    })
}

fn position(s: &State) -> Vec2 {
    Vec2::new(s.x as f32, s.y as f32)
}

/// `fraction` averaged over the last moments before `t`, so glows don't snap from one
/// control sample to the next.
fn smoothed(t: f64, fraction: impl Fn(f64) -> f64) -> f32 {
    (0..4)
        .map(|i| fraction(t - 0.02 * f64::from(i)) as f32)
        .sum::<f32>()
        / 4.0
}

/// Screen vectors (y down) of length `length` along world direction `direction` and
/// across it.
fn screen_axes(direction: Vec2, length: f32) -> (Vec2, Vec2) {
    let along = Vec2::new(direction.x, -direction.y) * length;
    (along, along.rot90())
}

/// A sprite centered on `center` with half axes `a` and `b`.
fn add_sprite(mesh: &mut egui::Mesh, center: Pos2, a: Vec2, b: Vec2, color: Color32) {
    let base = mesh.vertices.len() as u32;
    for (corner, uv) in [
        (-a - b, Pos2::new(0.0, 0.0)),
        (a - b, Pos2::new(1.0, 0.0)),
        (a + b, Pos2::new(1.0, 1.0)),
        (-a + b, Pos2::new(0.0, 1.0)),
    ] {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: center + corner,
            uv,
            color,
        });
    }
    mesh.indices
        .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// `rgb` (0 to 255) scaled by `brightness`, with zero alpha, so it adds onto what is below.
fn additive(rgb: [f32; 3], brightness: f32) -> Color32 {
    let [r, g, b] = rgb.map(|c| (c * brightness.clamp(0.0, 1.0)).round() as u8);
    Color32::from_rgba_premultiplied(r, g, b, 0)
}

/// A particle's color at `life` (0 fresh, 1 gone), along `colors`: fresh, early, late.
fn life_color(colors: &[[f32; 3]; 3], life: f32) -> [f32; 3] {
    let [hot, warm, cool] = *colors;
    let (from, to, f) = if life < 0.3 {
        (hot, warm, life / 0.3)
    } else {
        (warm, cool, (life - 0.3) / 0.7)
    };
    std::array::from_fn(|i| from[i] + (to[i] - from[i]) * f)
}

fn rotate(v: Vec2, angle: f32) -> Vec2 {
    let (sin, cos) = angle.sin_cos();
    Vec2::new(cos * v.x - sin * v.y, sin * v.x + cos * v.y)
}

/// A divergence-free 2D flow at `p` and time `t`: the curl `(dpsi/dy, -dpsi/dx)` of a
/// stream function `psi`, a sum of waves drifting at different speeds. Being the curl of
/// something, it only swirls, neither gathering nor scattering the particles, like an
/// incompressible fluid. Its magnitude is about 1.
fn swirl(p: Vec2, t: f32) -> Vec2 {
    // Wave vector, drift speed, phase, amplitude.
    const WAVES: [(f32, f32, f32, f32, f32); 4] = [
        (1.0, 0.6, 1.3, 0.0, 0.45),
        (-0.7, 1.1, -0.9, 2.1, 0.35),
        (1.9, -1.3, 1.7, 4.2, 0.2),
        (-2.3, -2.1, -2.2, 0.7, 0.15),
    ];
    WAVES
        .iter()
        .fold(Vec2::ZERO, |sum, &(kx, ky, speed, phase, amplitude)| {
            let c = amplitude * (kx * p.x + ky * p.y + speed * t + phase).cos();
            sum + Vec2::new(ky * c, -kx * c)
        })
}

/// Five numbers for particle `i`: the first in [0, 1), the others in [-1, 1).
fn random(i: u64) -> [f32; 5] {
    // splitmix64
    let mut state = i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    std::array::from_fn(|n| {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        let unit = (z >> 40) as f32 / (1u64 << 24) as f32;
        if n == 0 { unit } else { 2.0 * unit - 1.0 }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swirl_has_no_divergence() {
        let e = 1e-3;
        for &(x, y, t) in &[(0.0, 0.0, 0.0), (1.3, -0.4, 2.0), (-3.0, 5.5, 7.1)] {
            let p = Vec2::new(x, y);
            let dx = (swirl(p + Vec2::X * e, t) - swirl(p - Vec2::X * e, t)).x / (2.0 * e);
            let dy = (swirl(p + Vec2::Y * e, t) - swirl(p - Vec2::Y * e, t)).y / (2.0 * e);
            assert!((dx + dy).abs() < 1e-2, "divergence {} at {p:?}", dx + dy);
        }
    }

    #[test]
    fn random_numbers_are_in_range_and_vary() {
        let all: Vec<[f32; 5]> = (0..1000).map(random).collect();
        assert!(all.iter().all(|r| (0.0..1.0).contains(&r[0])));
        assert!(
            all.iter()
                .flatten()
                .skip(1)
                .all(|v| (-1.0..1.0).contains(v))
        );
        let mean = all.iter().map(|r| r[0]).sum::<f32>() / 1000.0;
        assert!((mean - 0.5).abs() < 0.05, "mean {mean}");
    }

    /// The reaction control jets' recoil (opposite to the exhaust) must turn the ship the
    /// way the torque does, and cancel out as a net force.
    #[test]
    fn rcs_jets_make_a_pure_couple_with_the_torque() {
        let s = State {
            theta: 0.8,
            ..State::default()
        };
        for torque in [1.0, -0.5] {
            let nozzles = rcs_nozzles(&s, torque, 1.0);
            let (mut force, mut moment) = (Vec2::ZERO, 0.0);
            for n in nozzles {
                let recoil = -n.direction;
                force += recoil;
                moment += n.position.x * recoil.y - n.position.y * recoil.x;
            }
            assert!(force.length() < 1e-5, "net force {force:?}");
            assert!(
                moment * torque > 0.0,
                "moment {moment} against torque {torque}"
            );
        }
    }
}
