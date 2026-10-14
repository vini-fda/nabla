//! The ion thruster's plume in the 3D view: a blue jet out of the main thruster at the
//! center of the ship's tail, as bright and as long as the thrust it was emitted with.
//!
//! The plume is a particle system without state: each frame it recomputes the particles
//! emitted during the last [`LIFETIME`] seconds from the trajectory itself, so it stays
//! consistent with the timeline when the replay is paused, scrubbed or sped up. Each
//! particle leaves the nozzle at its emission time with the ship's velocity then plus the
//! exhaust velocity, backward along the heading then, so the jet bends as the ship turns.
//! A divergence-free swirl field (the curl of a few drifting waves) carries the particles
//! more the older they get, which breaks the jet up into coherent eddies like a gas, while
//! they expand and cool from white-cyan to deep blue.
//!
//! Particles are soft sprites added onto the canvas (egui colors with zero alpha add up),
//! so where many overlap, near the nozzle, they saturate toward white.
//!
//! Lengths are in units of the ship's half length `L`, so the plume scales with the ship.

use egui::{Color32, Pos2, Vec2};

use crate::camera::Camera;
use crate::replay::Replay;

/// Particles emitted per second at full thrust.
const EMISSION_RATE: f64 = 1600.0;
/// Seconds a particle lives.
const LIFETIME: f64 = 0.4;
/// Exhaust speed relative to the ship, in `L` per second, at full thrust.
const EXHAUST_SPEED: f32 = 9.0;
/// Distance of the nozzle behind the ship's center, and the nozzle's radius, in `L`.
const NOZZLE_OFFSET: f32 = 0.98;
const NOZZLE_RADIUS: f32 = 0.12;
/// Half angle of the jet's cone, radians.
const SPREAD: f32 = 0.16;
/// How far the swirl field carries a particle by the end of its life, in `L`.
const TURBULENCE: f32 = 0.55;
/// Size of the swirls, in `L`, and how fast they drift and change, per second.
const EDDY_SIZE: f32 = 0.9;
const EDDY_SPEED: f32 = 1.7;
/// Brightness of one fresh particle at full thrust (0 to 1).
const PARTICLE_GAIN: f32 = 0.12;
/// How much longer than wide a fresh particle is, smeared along its flight like a streak.
const STREAK: f32 = 2.0;

/// Colors along a particle's life: fresh, early, late.
const HOT: [f32; 3] = [205.0, 240.0, 255.0];
const WARM: [f32; 3] = [60.0, 185.0, 255.0];
const COOL: [f32; 3] = [25.0, 70.0, 230.0];

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

    /// The jet at time `t`, behind a ship of half length `half_length`. Paint it before the
    /// ship, which covers its start.
    pub fn paint_jet(
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

        // The jet's luminous body: a soft cone from the nozzle, and a brighter core in it.
        let thrust = smoothed_thrust(replay, t);
        if thrust > 0.0 {
            let (nozzle, heading) = nozzle(replay, t, l);
            let (along, across) = screen_axes(heading, l * camera.scale);
            for (distance, length, width, color, gain) in
                [(0.9, 1.0, 0.28, WARM, 0.22), (0.45, 0.5, 0.11, HOT, 0.3)]
            {
                let center = camera.world_to_screen(nozzle - heading * distance * l, canvas);
                let color = additive(color, gain * thrust);
                add_sprite(&mut mesh, center, along * length, across * width, color);
            }
        }

        let first = ((t - LIFETIME) * EMISSION_RATE).ceil().max(0.0) as u64;
        let last = (t.min(replay.duration()) * EMISSION_RATE).floor().max(0.0) as u64;
        for i in first..=last {
            let emitted = i as f64 / EMISSION_RATE;
            let thrust = replay.thrust_fraction(emitted) as f32;
            let [gate, lateral, angle, speed, size] = random(i);
            // Fewer particles at lower thrust, none without.
            if gate >= thrust {
                continue;
            }
            let age = (t - emitted) as f32;
            let life = age / LIFETIME as f32;

            let s = replay.state_at_time(emitted);
            let heading = Vec2::angled(s.theta as f32);
            let normal = heading.rot90();
            let nozzle = Vec2::new(s.x as f32, s.y as f32) - heading * NOZZLE_OFFSET * l
                + normal * lateral * NOZZLE_RADIUS * l;
            // Off-axis particles lean outward, which opens the jet into a cone.
            let direction = rotate(-heading, SPREAD * (0.6 * angle + 0.4 * lateral));
            let exhaust = EXHAUST_SPEED * l * (0.55 + 0.45 * thrust) * (1.0 + 0.15 * speed);
            let ship_velocity = Vec2::new(s.vx as f32, s.vy as f32);
            let mut p = nozzle + (ship_velocity + direction * exhaust) * age;
            p +=
                swirl(p / (EDDY_SIZE * l), t as f32 * EDDY_SPEED) * TURBULENCE * l * life.powf(1.5);

            let fade = (1.0 - life).powi(2) * (life * 12.0).min(1.0);
            let brightness = PARTICLE_GAIN * thrust.sqrt() * fade;
            let color = additive(life_color(life), brightness);
            let radius = l * (0.07 + 0.5 * life) * (1.0 + 0.3 * size);
            let center = camera.world_to_screen(p, canvas);
            let (along, across) = screen_axes(-direction, radius * camera.scale);
            let streak = 1.0 + (STREAK - 1.0) * (1.0 - life);
            add_sprite(&mut mesh, center, along * streak, across, color);
        }
        painter.add(egui::Shape::mesh(mesh));
    }

    /// The glow where the jet leaves the nozzle, at time `t`. Paint it over the ship.
    pub fn paint_nozzle(
        &self,
        painter: &egui::Painter,
        camera: &Camera,
        canvas: egui::Rect,
        replay: &Replay,
        t: f64,
        half_length: f32,
    ) {
        let thrust = smoothed_thrust(replay, t);
        if thrust <= 0.0 {
            return;
        }
        let (nozzle, heading) = nozzle(replay, t, half_length);
        let center = camera.world_to_screen(nozzle, canvas);
        let (along, across) = screen_axes(heading, half_length * camera.scale);

        let mut mesh = egui::Mesh::with_texture(self.sprite.id());
        // A wide dim halo, the glowing ring seen edge on, and its white-hot core.
        let halo = additive(WARM, 0.35 * thrust);
        add_sprite(&mut mesh, center, along * 0.45, across * 0.55, halo);
        let ring = additive(HOT, 0.8 * thrust);
        add_sprite(&mut mesh, center, along * 0.09, across * 0.28, ring);
        let core = additive([255.0, 255.0, 255.0], 0.9 * thrust);
        add_sprite(&mut mesh, center, along * 0.05, across * 0.12, core);
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// Thrust as a fraction of `F_max` over the last moments before `t`, so glows don't snap
/// from one control sample to the next.
fn smoothed_thrust(replay: &Replay, t: f64) -> f32 {
    (0..4)
        .map(|i| replay.thrust_fraction(t - 0.02 * f64::from(i)) as f32)
        .sum::<f32>()
        / 4.0
}

/// Where the nozzle is at time `t` (world), and the ship's heading then.
fn nozzle(replay: &Replay, t: f64, half_length: f32) -> (Vec2, Vec2) {
    let s = replay.state_at_time(t);
    let heading = Vec2::angled(s.theta as f32);
    let nozzle = Vec2::new(s.x as f32, s.y as f32) - heading * NOZZLE_OFFSET * half_length;
    (nozzle, heading)
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

/// A particle's color at `life` (0 fresh, 1 gone).
fn life_color(life: f32) -> [f32; 3] {
    let (from, to, f) = if life < 0.3 {
        (HOT, WARM, life / 0.3)
    } else {
        (WARM, COOL, (life - 0.3) / 0.7)
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
}
