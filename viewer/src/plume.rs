//! Thruster plumes in the 3D view: a blue jet out of the main (ion) thruster's bell at the
//! center of the tail, as strong as the thrust, and white puffs out of the reaction control
//! thrusters, as strong as the torque.
//!
//! The ship's cross-section is an equilateral triangle, and the reaction control thrusters
//! come in two sets of three, each nozzle blowing straight out from the ship's axis: pods at
//! the corners of the tail, and small nozzles in the nose. In each set two are on the upper
//! sides, 30 degrees above the horizontal, and one is underneath. Their three directions,
//! 120 degrees apart, span the cross-section, so mixing two of them pushes the tail (or the
//! nose) any way across the ship. To turn the ship counterclockwise the tail goes to its
//! right and the nose to its left: in the tail the upper left pod fires, and the bottom one
//! at half strength cancels its downward push; in the nose the upper right nozzle and the
//! bottom one, the same way (and the other way round to turn it clockwise). The two sets
//! push equally and oppositely, a couple that turns the ship without pushing it.
//!
//! The plumes are particle systems without state: each frame recomputes the particles
//! emitted during the last moments from the trajectory itself, so they stay consistent with
//! the timeline when the replay is paused, scrubbed or sped up. Each particle leaves its
//! nozzle at its emission time with the nozzle's velocity then (the ship's velocity, plus
//! the spin's `omega x r` at the nozzle) plus the exhaust velocity, so a turning ship sweeps
//! its jets around like a garden hose. The jets also flap and bunch up by an amount that
//! depends on when their gas left the nozzle, growing with its age: each wiggle travels
//! downstream with the gas that carries it, like the eddies of a real jet, while the gas
//! expands, cools and fades.
//!
//! Particles are soft sprites added onto the canvas (colors with zero alpha add up), so where
//! many overlap, near a nozzle, they saturate toward white. They are drawn after the 3D ship
//! and depth tested against it (see [`crate::sprites`]), so the hull hides the parts of the
//! plumes behind it, and only those, from whichever side the camera looks.
//!
//! Lengths are in units of the ship's half length `L`, so the plumes scale with the ship.

use egui::{Color32, Vec2};
use ship2d_optimization::State;

use crate::camera::{Camera, Vec3};
use crate::replay::Replay;
use crate::ship3d::{DepthMap, Image};
use crate::sprites::{SpriteBatch, SpriteRenderer};

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
    /// How far the jet flaps sideways by the end of a particle's life, in `L`.
    turbulence: f32,
    /// How often the jet flaps, per second of emission.
    flap_rate: f64,
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
    turbulence: 0.45,
    flap_rate: 2.5,
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
    turbulence: 0.1,
    flap_rate: 6.0,
    gain: 0.2,
    streak: 2.5,
    size: (0.025, 0.22),
    colors: [
        [255.0, 255.0, 255.0],
        [215.0, 225.0, 240.0],
        [120.0, 130.0, 150.0],
    ],
};

/// Distance of the main nozzle (the rim of the engine's bell, set into the tail) behind the
/// ship's center, in `L`.
const MAIN_NOZZLE: f32 = 0.86;

/// The two sets of reaction control nozzles: how far they are in front of the ship's center
/// (behind it if negative) and out from its axis, in `L`, and how big their jets are,
/// relative to the [`RCS`] jet. In the tail, on the outer faces of the pods, halfway along
/// them; in the nose, small rings near its tip.
const RCS_SETS: [RcsSet; 2] = [
    RcsSet {
        forward: -0.9,
        out: 0.43,
        scale: 1.0,
    },
    RcsSet {
        forward: 0.96,
        out: 0.11,
        scale: 0.6,
    },
];

/// One set of three reaction control nozzles round the axis: see [`RCS_SETS`].
struct RcsSet {
    forward: f32,
    out: f32,
    scale: f32,
}

/// The reaction control nozzles' outward directions across the ship, as (to its right, up
/// out of the plane): at 30, 150 and 270 degrees from its right, round the axis. The same in
/// both sets.
const RCS_PODS: [(f32, f32); 3] = [(0.866_025_4, 0.5), (-0.866_025_4, 0.5), (0.0, -1.0)];

/// How far round the hull from the viewer a tip may be, as the cosine of the angle between
/// its direction from the ship's axis and the view, for its flash to show: a little past
/// the silhouette.
const RCS_MIN_FACING: f32 = -0.1;

/// How far toward the viewer the glows at the nozzles are drawn, in `L`, past the hull
/// right around them: the main nozzle's over the engine bell and the tail, as when seen from
/// above, and the reaction control ones off the hull's tips. The hull farther in front still
/// hides them.
const MAIN_GLOW_LIFT: f32 = 0.45;
const RCS_GLOW_LIFT: f32 = 0.1;

/// The least a sprite is foreshortened along its axis, as a fraction of its length, so the
/// round cross-section of a jet seen end on still shows.
const MIN_FORESHORTENING: f32 = 0.35;

/// A nozzle at an instant: where it is (world), which way its exhaust goes, and how hard it
/// fires.
#[derive(Clone, Copy)]
struct Nozzle {
    position: Vec3,
    /// Which way the nozzle faces out of the hull, a unit vector.
    outward: Vec3,
    /// Which way the exhaust leaves, a unit vector.
    direction: Vec3,
    /// Two unit vectors across `direction` and each other, which the jet spreads along.
    across: [Vec3; 2],
    /// Fraction of the jet's strength the nozzle fires at (0 to 1).
    share: f32,
    /// Size of the nozzle and its jet, relative to the jet's own.
    scale: f32,
}

/// Draws the plumes, as soft sprites depth tested against the 3D ship.
pub struct Plume {
    renderer: SpriteRenderer,
}

impl Plume {
    /// Sets up the sprites' drawing in egui's render pass, as [`crate::ship3d::ShipRenderer::new`].
    pub fn new(
        render_state: &eframe::egui_wgpu::RenderState,
        msaa_samples: u32,
        depth_format: Option<eframe::wgpu::TextureFormat>,
    ) -> Self {
        // The sprite: a soft round blob, premultiplied.
        const SIZE: u32 = 64;
        let pixels = (0..SIZE * SIZE)
            .flat_map(|i| {
                let (x, y) = ((i % SIZE) as f32, (i / SIZE) as f32);
                let d = Vec2::new(x + 0.5, y + 0.5) / SIZE as f32 * 2.0 - Vec2::splat(1.0);
                // Gaussian, cut off smoothly at the edge.
                let r2 = d.length_sq();
                let a = (-4.0 * r2).exp() * (1.0 - r2).max(0.0);
                [(a * 255.0).round() as u8; 4]
            })
            .collect();
        let sprite = Image {
            width: SIZE,
            height: SIZE,
            pixels,
        };
        Self {
            renderer: SpriteRenderer::new(render_state, &sprite, msaa_samples, depth_format),
        }
    }

    /// Paints the plumes at time `t`, from a ship of half length `half_length`, seen through
    /// `camera` in `canvas`, with the ship's depth `depth`. Paint them after the ship, which
    /// hides them where it is nearer.
    pub fn paint(
        &self,
        camera: &Camera,
        canvas: egui::Rect,
        replay: &Replay,
        t: f64,
        half_length: f32,
        depth: &DepthMap,
    ) -> Option<egui::PaintCallback> {
        let view = View {
            camera,
            canvas,
            replay,
            t,
            l: half_length,
            depth,
        };
        let mut batch = SpriteBatch::default();
        view.jets(&mut batch);
        view.nozzles(&mut batch);
        self.renderer.paint_callback(canvas, batch)
    }
}

/// What the plumes are drawn for: time `t` of `replay`, a ship of half length `l`, seen
/// through `camera` in `canvas`, with the ship's depth `depth`.
struct View<'a> {
    camera: &'a Camera,
    canvas: egui::Rect,
    replay: &'a Replay,
    t: f64,
    l: f32,
    depth: &'a DepthMap,
}

impl View<'_> {
    /// Adds a sprite at world point `p`, drawn `lift` world units toward the viewer, with half
    /// axes of world lengths `[along, across]` along `direction` and across it, as
    /// [`screen_axes`] sees them; or round, of radius `along`, if `direction` is `None`.
    /// Sprites not in front of the eye are left out.
    fn sprite(
        &self,
        batch: &mut SpriteBatch,
        p: Vec3,
        lift: f32,
        direction: Option<Vec3>,
        [along, across]: [f32; 2],
        color: Color32,
    ) {
        let (camera, canvas) = (self.camera, self.canvas);
        let Some(center) = camera.project(p, canvas) else {
            return;
        };
        let axes = match direction {
            Some(direction) => screen_axes(camera, canvas, p, direction, [along, across]),
            None => camera
                .scale_at(p, canvas)
                .map(|scale| (Vec2::new(along * scale, 0.0), Vec2::new(0.0, along * scale))),
        };
        if let Some((a, b)) = axes {
            batch.add(center, a, b, self.depth.depth(p, lift), color);
        }
    }

    /// The jets.
    fn jets(&self, batch: &mut SpriteBatch) {
        let Self { replay, t, l, .. } = *self;

        // The main jet's luminous body: a soft cone from the nozzle, and a brighter core,
        // each centered half its length downstream, so it is already bright where it
        // leaves the tail.
        let thrust = smoothed(t, |t| replay.thrust_fraction(t));
        if thrust > 0.0 {
            let [nozzle] = main_nozzle(&replay.state_at_time(t), l);
            let [hot, warm, _] = MAIN.colors;
            for (length, width, color, gain) in [(1.0, 0.28, warm, 0.22), (0.5, 0.11, hot, 0.3)] {
                let center = nozzle.position + nozzle.direction * (0.55 * length * l);
                let color = additive(color, gain * thrust);
                self.sprite(
                    batch,
                    center,
                    0.0,
                    Some(-nozzle.direction),
                    [length * l, width * l],
                    color,
                );
            }
        }

        self.particles(
            batch,
            &MAIN,
            0,
            |t| replay.thrust_fraction(t),
            |s, _| main_nozzle(s, l),
        );
        self.particles(
            batch,
            &RCS,
            1,
            |t| replay.torque_fraction(t),
            |s, torque| rcs_nozzles(s, torque, l),
        );
    }

    /// The glows where the jets leave their nozzles.
    fn nozzles(&self, batch: &mut SpriteBatch) {
        let Self {
            camera,
            replay,
            t,
            l,
            ..
        } = *self;
        let s = replay.state_at_time(t);

        let thrust = smoothed(t, |t| replay.thrust_fraction(t));
        if thrust > 0.0 {
            let [nozzle] = main_nozzle(&s, l);
            let p = nozzle.position;
            let lift = MAIN_GLOW_LIFT * l;
            let [hot, warm, _] = MAIN.colors;
            let white = [255.0, 255.0, 255.0];
            // A wide dim halo, the glowing ring seen edge on, and its white-hot core.
            for (length, width, color, gain) in [
                (0.45, 0.55, warm, 0.35),
                (0.09, 0.28, hot, 0.8),
                (0.05, 0.12, white, 0.9),
            ] {
                let color = additive(color, gain * thrust);
                let axes = [length * l, width * l];
                self.sprite(batch, p, lift, Some(-nozzle.direction), axes, color);
            }
        }

        // A flash at each visible reaction control nozzle that is firing.
        let torque = smoothed(t, |t| replay.torque_fraction(t));
        if torque != 0.0 {
            let toward_viewer = camera.orbit.toward_viewer();
            for nozzle in rcs_nozzles(&s, torque, l) {
                let strength = torque.abs() * nozzle.share;
                if strength > 0.0 && nozzle.outward.dot(toward_viewer) > RCS_MIN_FACING {
                    let r = 0.07 * l * nozzle.scale * strength.sqrt();
                    let color = additive(RCS.colors[0], 0.9 * strength);
                    self.sprite(
                        batch,
                        nozzle.position,
                        RCS_GLOW_LIFT * l,
                        None,
                        [r, r],
                        color,
                    );
                }
            }
        }
    }

    /// Adds the live particles of `jet` to `batch`. `strength` gives how hard the jet fires
    /// at a time (-1 to 1; the sign is the nozzles' business), and `nozzles` where its
    /// nozzles are and which way they blow, given the state and strength then. `salt` keeps
    /// different jets' particles from sharing random numbers.
    fn particles<const N: usize>(
        &self,
        batch: &mut SpriteBatch,
        jet: &Jet,
        salt: u64,
        strength: impl Fn(f64) -> f64,
        nozzles: impl Fn(&State, f32) -> [Nozzle; N],
    ) {
        let Self { replay, t, l, .. } = *self;
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

            for (j, nozzle) in nozzles(&s, signed).into_iter().enumerate() {
                let power = power * nozzle.share;
                // A smaller nozzle's jet is smaller all over, and as long-lived.
                let l = l * nozzle.scale;
                let [gate, lateral, angle, speed, size, lateral2, angle2] =
                    random((i * N as u64 + j as u64) ^ (salt << 56));
                // Fewer particles at lower strength.
                if gate >= power {
                    continue;
                }
                let [across, across2] = nozzle.across;
                let start = nozzle.position
                    + (across * lateral + across2 * lateral2) * (jet.nozzle_radius * l);
                // Off-axis particles lean outward, which opens the jet into a cone.
                let lean =
                    |angle: f32, lateral: f32| (jet.spread * (0.6 * angle + 0.4 * lateral)).tan();
                let direction = unit(
                    nozzle.direction
                        + across * lean(angle, lateral)
                        + across2 * lean(angle2, lateral2),
                );
                let exhaust = jet.speed * l * (0.55 + 0.45 * power) * (1.0 + 0.15 * speed);
                let velocity = Vec3::plane(velocity_at(&s, Vec2::new(start.x, start.y)));
                let mut p = start + (velocity + direction * exhaust) * age;
                // Flapping across the jet, both ways, and bunching along it, half as much.
                let seed = (salt << 8) + 3 * j as u64;
                let growth = jet.turbulence * l * life.powf(1.5);
                p = p
                    + across * (flap(emitted, jet.flap_rate, seed) * growth)
                    + across2 * (flap(emitted, jet.flap_rate, seed + 2) * growth)
                    + nozzle.direction * (flap(emitted, jet.flap_rate, seed + 1) * 0.5 * growth);

                // Fades in over the first moments only, so the jet is bright at the nozzle.
                let fade = (1.0 - life).powi(2) * (life * 40.0).min(1.0);
                let color = additive(
                    life_color(&jet.colors, life),
                    jet.gain * power.sqrt() * fade,
                );
                let radius = l * (jet.size.0 + jet.size.1 * life) * (1.0 + 0.3 * size);
                let streak = 1.0 + (jet.streak - 1.0) * (1.0 - life);
                let axes = [radius * streak, radius];
                self.sprite(batch, p, 0.0, Some(direction), axes, color);
            }
        }
    }
}

/// The main thruster in state `s`: in the tail, on the axis, blowing backward.
fn main_nozzle(s: &State, l: f32) -> [Nozzle; 1] {
    let [nose, right] = ship_axes(s);
    [Nozzle {
        position: Vec3::plane(position(s)) - nose * (MAIN_NOZZLE * l),
        outward: -nose,
        direction: -nose,
        across: [right, Vec3::Z],
        share: 1.0,
        scale: 1.0,
    }]
}

/// The reaction control thrusters in state `s` under torque fraction `torque`, the tail's
/// three and then the nose's: each nozzle blowing straight out from the axis, as hard as
/// [`rcs_mix`] has it to push the tail to the ship's right and the nose to its left for a
/// counterclockwise (positive) torque, and the other way round for a clockwise one.
fn rcs_nozzles(s: &State, torque: f32, l: f32) -> [Nozzle; 6] {
    let [nose, right] = ship_axes(s);
    std::array::from_fn(|i| {
        let set = &RCS_SETS[i / 3];
        // The recoil across the ship, to its right for the tail turning counterclockwise.
        let recoil = torque.signum() * -set.forward.signum();
        let (x, z) = RCS_PODS[i % 3];
        let outward = right * x + Vec3::Z * z;
        let center = Vec3::plane(position(s)) + nose * (set.forward * l);
        Nozzle {
            position: center + outward * (set.out * l),
            outward,
            direction: outward,
            across: [nose, outward.cross(nose)],
            share: rcs_mix([recoil, 0.0])[i % 3],
            scale: set.scale,
        }
    })
}

/// How hard each of a set of reaction control nozzles fires (0 to 1) for the set to recoil
/// along `recoil`, a unit vector across the ship as (to its right, up): the least thrust
/// whose recoils add up to that direction, scaled for the nozzle working hardest to fire at
/// full strength.
fn rcs_mix(recoil: [f32; 2]) -> [f32; 3] {
    // Each nozzle recoils against its outward direction. The three directions add up to
    // zero, so adding the same to every share changes nothing: starting from the mix that
    // spreads the work evenly, add just enough for none to pull.
    let even = RCS_PODS.map(|(x, z)| -2.0 / 3.0 * (recoil[0] * x + recoil[1] * z));
    let least = even.into_iter().fold(f32::INFINITY, f32::min);
    let mix = even.map(|share| share - least);
    let most = mix.into_iter().fold(0.0, f32::max);
    if most > 0.0 {
        mix.map(|share| share / most)
    } else {
        mix
    }
}

/// The ship's nose and its right in state `s`, unit vectors in the plane.
fn ship_axes(s: &State) -> [Vec3; 2] {
    let heading = Vec2::angled(s.theta as f32);
    // `rot90` turns +x to -y: clockwise in the world (y up), so to the ship's right.
    [Vec3::plane(heading), Vec3::plane(heading.rot90())]
}

fn position(s: &State) -> Vec2 {
    Vec2::new(s.x as f32, s.y as f32)
}

/// `v` scaled to unit length.
fn unit(v: Vec3) -> Vec3 {
    v * (1.0 / v.length())
}

/// `fraction` averaged over the last moments before `t`, so glows don't snap from one
/// control sample to the next.
fn smoothed(t: f64, fraction: impl Fn(f64) -> f64) -> f32 {
    (0..4)
        .map(|i| fraction(t - 0.02 * f64::from(i)) as f32)
        .sum::<f32>()
        / 4.0
}

/// A sprite's half axes on screen (y down) for world lengths `[along, across]` along
/// `direction` and across it, at world point `p` seen through `camera` in
/// `canvas`. Seen at a slant, the axis along `direction` is foreshortened, down to
/// [`MIN_FORESHORTENING`]; the one across it, round the jet, isn't. `None` when `p` is not
/// in front of the eye.
fn screen_axes(
    camera: &Camera,
    canvas: egui::Rect,
    p: Vec3,
    direction: Vec3,
    [along, across]: [f32; 2],
) -> Option<(Vec2, Vec2)> {
    let scale = camera.scale_at(p, canvas)?;
    let projected = camera.screen_vector_at(p, direction * along, canvas)?;
    let unit = projected.normalized();
    let unit = if unit.is_finite() { unit } else { Vec2::X };
    let along = unit * projected.length().max(MIN_FORESHORTENING * along * scale);
    Some((along, unit.rot90() * across * scale))
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

/// Velocity of the ship's point `point` (world) in state `s`: the ship's velocity plus
/// `omega x r` of its spin, `r` from the ship's center to the point.
fn velocity_at(s: &State, point: Vec2) -> Vec2 {
    let r = point - position(s);
    Vec2::new(s.vx as f32, s.vy as f32) + Vec2::new(-r.y, r.x) * s.omega as f32
}

/// A smooth wiggle (about -1 to 1) of the time `emitted` the gas left the nozzle, about
/// `rate` times a second, different for each `seed`. All the gas from one instant moves
/// alike, so each wiggle is carried downstream with the gas, as eddies are in a jet.
fn flap(emitted: f64, rate: f64, seed: u64) -> f32 {
    // Relative frequencies (not multiples of each other) and amplitudes.
    const PARTS: [(f64, f32); 3] = [(1.0, 0.55), (2.31, 0.3), (5.17, 0.15)];
    let [_, a, b, c, ..] = random(seed.wrapping_mul(0x5851_F42D_4C95_7F2D));
    PARTS
        .iter()
        .zip([a, b, c])
        .map(|(&(frequency, amplitude), phase)| {
            let angle = std::f64::consts::TAU * frequency * rate * emitted;
            amplitude * (angle + std::f64::consts::PI * f64::from(phase)).sin() as f32
        })
        .sum()
}

/// Seven numbers for particle `i`: the first in [0, 1), the others in [-1, 1).
fn random(i: u64) -> [f32; 7] {
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
    fn a_spinning_ship_sweeps_its_tail_sideways() {
        // Pointing along +x, spinning counterclockwise at 2 rad/s, not moving.
        let s = State {
            omega: 2.0,
            ..State::default()
        };
        let tail = Vec2::new(-1.0, 0.0);
        let v = velocity_at(&s, tail);
        // Counterclockwise, the tail (behind the center) swings toward -y.
        assert!((v - Vec2::new(0.0, -2.0)).length() < 1e-6, "{v:?}");
        assert_eq!(velocity_at(&s, position(&s)), Vec2::ZERO);
    }

    #[test]
    fn flapping_is_smooth_and_bounded() {
        for seed in 0..10 {
            for k in 0..1000 {
                let t = f64::from(k) * 0.01;
                let now = flap(t, 3.0, seed);
                assert!(now.abs() <= 1.0, "{now}");
                // Changes little over a thousandth of a second.
                assert!((flap(t + 0.001, 3.0, seed) - now).abs() < 0.05);
            }
        }
    }

    #[test]
    fn random_numbers_are_in_range_and_vary() {
        let all: Vec<[f32; 7]> = (0..1000).map(random).collect();
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

    /// The reaction control jets' recoil (opposite to the exhaust) must make a couple that
    /// turns the ship the way the torque does, without pushing it: the tail and the nose are
    /// each pushed sideways, neither up nor down nor along the ship, oppositely.
    #[test]
    fn rcs_jets_make_a_pure_couple_with_the_torque() {
        let s = State {
            x: 1.0,
            y: -2.0,
            theta: 0.8,
            ..State::default()
        };
        let [nose, right] = ship_axes(&s);
        for torque in [1.0, -0.5] {
            let nozzles = rcs_nozzles(&s, torque, 1.0);
            let (mut force, mut moment) = (Vec3::ZERO, 0.0);
            for (set, nozzles) in nozzles.chunks(3).enumerate() {
                let mut set_force = Vec3::ZERO;
                for n in nozzles {
                    let recoil = -n.direction * n.share;
                    let r = n.position - Vec3::plane(position(&s));
                    set_force = set_force + recoil;
                    moment += r.x * recoil.y - r.y * recoil.x;
                }
                assert!(set_force.z.abs() < 1e-5, "vertical force {set_force:?}");
                assert!(set_force.dot(nose).abs() < 1e-5, "along {set_force:?}");
                // The tail (set 0) to the right for a counterclockwise torque.
                let side = if set == 0 { 1.0 } else { -1.0 };
                assert!(set_force.dot(right) * torque * side > 0.0, "{set_force:?}");
                force = force + set_force;
            }
            assert!(force.length() < 1e-5, "net force {force:?}");
            assert!(
                moment * torque > 0.0,
                "moment {moment} against torque {torque}"
            );
        }
    }

    /// One top pod fires at full strength, the bottom one at half, the other top one not.
    #[test]
    fn rcs_mix_fires_two_pods() {
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5);
        assert!(near(rcs_mix([1.0, 0.0]), [0.0, 1.0, 0.5]));
        assert!(near(rcs_mix([-1.0, 0.0]), [1.0, 0.0, 0.5]));
        // Straight up: the bottom pod alone.
        assert!(near(rcs_mix([0.0, 1.0]), [0.0, 0.0, 1.0]));
    }

    /// Each set of nozzles sits round the axis: out from it by the same distance, 120 degrees
    /// apart, at the tail or the nose.
    #[test]
    fn rcs_nozzles_are_round_the_axis() {
        let s = State::default();
        let nozzles = rcs_nozzles(&s, 1.0, 1.0);
        for (set, nozzles) in RCS_SETS.iter().zip(nozzles.chunks(3)) {
            for n in nozzles {
                let axis = Vec3::new(n.position.x, 0.0, 0.0);
                assert!((n.position.x - set.forward).abs() < 1e-5);
                assert!(((n.position - axis).length() - set.out).abs() < 1e-5);
            }
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                let cos = nozzles[a].outward.dot(nozzles[b].outward);
                assert!((cos + 0.5).abs() < 1e-5, "{cos}");
            }
        }
    }
}
