//! Thruster sounds, synthesized sample by sample: the main (ion) thruster in one of several
//! styles ([`IonSound`]), and the reaction control thrusters (RCS) as hisses of cold gas.
//!
//! This is only the signal processing, with no audio device: [`crate::sound`] feeds the
//! synthesizer the thrust and torque of the replay and plays what it renders.
//!
//! Everything a voice does once per sample is cheap (oscillators, noise, filters); what
//! needs `tan`, `powf` or `exp` (filter tuning, loudness curves) runs at a control rate,
//! once every [`CONTROL_PERIOD`] samples. Levels and gains are smoothed, so steps in the
//! thrust, torque, volumes or sound never click.

use std::f32::consts::{FRAC_PI_4, PI, SQRT_2, TAU};

/// Samples between two updates of the voices' slow parameters.
const CONTROL_PERIOD: u32 = 32;

/// How the main (ion) thruster sounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IonSound {
    /// A low electric drone whose filter opens up with the thrust.
    #[default]
    Hum,
    /// A high, glassy whine that rises in pitch with the thrust.
    Whine,
    /// Roaring plasma: filtered noise that brightens with the thrust.
    Hiss,
    /// A deep engine throb, pulsing faster with the thrust.
    Thrum,
    /// Discrete plasma discharges, crackling faster with the thrust.
    Pulsed,
}

impl IonSound {
    /// Every sound, in the order they are offered.
    pub const ALL: [Self; 5] = [
        Self::Hum,
        Self::Whine,
        Self::Hiss,
        Self::Thrum,
        Self::Pulsed,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Hum => "Hall hum",
            Self::Whine => "Ion whine",
            Self::Hiss => "Plasma hiss",
            Self::Thrum => "Deep thrum",
            Self::Pulsed => "Pulsed plasma",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Hum => {
                "A low electric drone: detuned sawtooth waves through a filter that opens up with the thrust, with a fizz of plasma on top."
            }
            Self::Whine => {
                "A high, glassy electric whine with a slow vibrato, rising in pitch with the thrust."
            }
            Self::Hiss => {
                "Roaring plasma: noise through two slowly wandering resonances, brighter with more thrust."
            }
            Self::Thrum => {
                "A deep engine throb that pulses faster with more thrust. Needs speakers with some bass."
            }
            Self::Pulsed => {
                "A pulsed plasma thruster: discrete discharges that crackle faster with more thrust."
            }
        }
    }
}

/// What the thrusters are doing, and how loud to play them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Controls {
    /// Thrust, as a fraction of `F_max` (0 to 1).
    pub thrust: f32,
    /// Torque, as a fraction of `tau_max` (-1 to 1).
    pub torque: f32,
    /// How the ion thruster sounds.
    pub ion: IonSound,
    /// Gain of the ion thruster (linear, 0 to 1).
    pub ion_gain: f32,
    /// Gain of the reaction control thrusters (linear, 0 to 1).
    pub rcs_gain: f32,
    /// Gain of everything (linear, 0 to 1).
    pub master_gain: f32,
}

/// The thrusters' synthesizer: renders stereo audio following [`Controls`].
pub struct Synth {
    rate: f32,
    controls: Controls,
    /// Samples rendered, for the control rate.
    clock: u32,
    noise: Noise,
    thrust: Smoother,
    gains: [Smoother; 3],
    /// The ion sound playing, and its gain while switching to another one.
    playing: IonSound,
    switch: Smoother,
    ion: IonVoices,
    rcs: Rcs,
}

impl Synth {
    pub fn new(sample_rate: f32) -> Self {
        let rate = sample_rate;
        Self {
            rate,
            controls: Controls::default(),
            clock: 0,
            noise: Noise(0x9e37_79b9),
            // Ion engines throttle smoothly: no pops when the thrust jumps.
            thrust: Smoother::new(rate, 0.04, 0.12),
            gains: [Smoother::new(rate, 0.02, 0.02); 3],
            playing: IonSound::default(),
            switch: Smoother::new(rate, 0.015, 0.015),
            ion: IonVoices::new(rate),
            rcs: Rcs::new(rate),
        }
    }

    /// Follow `controls` from the next sample on.
    pub fn set(&mut self, controls: Controls) {
        self.controls = controls;
    }

    /// Fills `out`, interleaved frames of `channels` channels, with the next samples. Mono
    /// gets the mix of left and right; channels after the first two stay silent.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        let c = self.controls;
        for frame in out.chunks_mut(channels) {
            if self.clock.is_multiple_of(CONTROL_PERIOD) {
                self.control();
            }
            self.clock = self.clock.wrapping_add(1);

            let thrust = self.thrust.next(c.thrust.clamp(0.0, 1.0));
            // Fade the old sound out before switching, then the new one in.
            let switching = self.playing != c.ion;
            let switch = self.switch.next(if switching { 0.0 } else { 1.0 });
            if switching && switch < 1e-3 {
                self.playing = c.ion;
                self.ion = IonVoices::new(self.rate);
                self.control();
            }
            let ion = self.ion.sample(self.playing, thrust, &mut self.noise) * switch;
            let (left, right) = self.rcs.sample(&mut self.noise);

            let [ion_gain, rcs_gain, master] =
                [c.ion_gain, c.rcs_gain, c.master_gain].map(|g| g.clamp(0.0, 1.0));
            let ion = ion * self.gains[0].next(ion_gain);
            let rcs_gain = self.gains[1].next(rcs_gain);
            let master = self.gains[2].next(master);
            // A soft limiter, so loud mixes saturate instead of clipping.
            let left = ((ion + left * rcs_gain) * master).tanh();
            let right = ((ion + right * rcs_gain) * master).tanh();

            match frame {
                [mono] => *mono = f32::midpoint(left, right),
                [l, r, rest @ ..] => {
                    *l = left;
                    *r = right;
                    rest.fill(0.0);
                }
                [] => {}
            }
        }
    }

    /// The slow parameters, once every [`CONTROL_PERIOD`] samples.
    fn control(&mut self) {
        let thrust = self.thrust.value;
        self.ion.control(self.playing, thrust, &mut self.noise);
        self.rcs.control(self.controls.torque.clamp(-1.0, 1.0));
    }
}

/// One voice per ion sound; only the one playing runs.
struct IonVoices {
    hum: Hum,
    whine: Whine,
    hiss: Hiss,
    thrum: Thrum,
    pulsed: Pulsed,
}

impl IonVoices {
    fn new(rate: f32) -> Self {
        Self {
            hum: Hum::new(rate),
            whine: Whine::new(rate),
            hiss: Hiss::new(rate),
            thrum: Thrum::new(rate),
            pulsed: Pulsed::new(rate),
        }
    }

    fn control(&mut self, sound: IonSound, thrust: f32, noise: &mut Noise) {
        match sound {
            IonSound::Hum => self.hum.control(thrust),
            IonSound::Whine => self.whine.control(thrust),
            IonSound::Hiss => self.hiss.control(thrust, noise),
            IonSound::Thrum => self.thrum.control(thrust),
            IonSound::Pulsed => self.pulsed.control(thrust),
        }
    }

    fn sample(&mut self, sound: IonSound, thrust: f32, noise: &mut Noise) -> f32 {
        match sound {
            IonSound::Hum => self.hum.sample(noise),
            IonSound::Whine => self.whine.sample(noise),
            IonSound::Hiss => self.hiss.sample(noise),
            IonSound::Thrum => self.thrum.sample(noise),
            IonSound::Pulsed => self.pulsed.sample(thrust, noise),
        }
    }
}

/// Two detuned sawtooth waves and a sub-octave sine, through a low-pass filter that opens
/// with the thrust, plus a faint fizz of band-passed noise.
struct Hum {
    rate: f32,
    saws: [Osc; 2],
    sub: Osc,
    filter: Svf,
    fizz: Svf,
    frequency: f32,
    amplitude: f32,
}

impl Hum {
    fn new(rate: f32) -> Self {
        let mut fizz = Svf::default();
        fizz.tune(rate, 3000.0, 2.0);
        Self {
            rate,
            saws: [Osc::default(), Osc { phase: 0.37 }],
            sub: Osc::default(),
            filter: Svf::default(),
            fizz,
            frequency: 0.0,
            amplitude: 0.0,
        }
    }

    fn control(&mut self, thrust: f32) {
        self.frequency = 70.0 + 25.0 * thrust;
        self.filter
            .tune(self.rate, 250.0 + 2200.0 * thrust.powf(0.7), 1.4);
        self.amplitude = thrust.powf(0.6);
    }

    fn sample(&mut self, noise: &mut Noise) -> f32 {
        let dt = self.frequency / self.rate;
        let saws = self.saws[0].saw(dt) + self.saws[1].saw(dt * 1.006);
        let sub = self.sub.sine(0.5 * dt);
        let body = self.filter.tick(0.35 * saws + 0.6 * sub).low;
        let fizz = self.fizz.tick(noise.next()).band;
        0.39 * self.amplitude * (body + 0.25 * self.amplitude * fizz)
    }
}

/// A sine with two quieter harmonics, doubled by a slightly detuned copy that beats against
/// it, with a slow vibrato, over a soft bed of noise.
struct Whine {
    rate: f32,
    partials: [Osc; 3],
    detuned: Osc,
    vibrato: Osc,
    bed: Svf,
    frequency: f32,
    amplitude: f32,
}

impl Whine {
    fn new(rate: f32) -> Self {
        let mut bed = Svf::default();
        bed.tune(rate, 1500.0, 0.7);
        Self {
            rate,
            partials: [Osc::default(); 3],
            detuned: Osc::default(),
            vibrato: Osc::default(),
            bed,
            frequency: 0.0,
            amplitude: 0.0,
        }
    }

    fn control(&mut self, thrust: f32) {
        self.frequency = 600.0 + 900.0 * thrust;
        self.amplitude = thrust.sqrt();
    }

    fn sample(&mut self, noise: &mut Noise) -> f32 {
        let vibrato = 1.0 + 0.004 * self.vibrato.sine(5.5 / self.rate);
        let dt = self.frequency * vibrato / self.rate;
        let tone = self.partials[0].sine(dt)
            + 0.3 * self.partials[1].sine(2.01 * dt)
            + 0.12 * self.partials[2].sine(3.98 * dt)
            + 0.5 * self.detuned.sine(1.0025 * dt);
        let bed = self.bed.tick(noise.next()).low;
        0.145 * self.amplitude * (tone + 0.6 * bed)
    }
}

/// White noise through two resonances that wander slowly, and a low rumble; the upper
/// resonance rises and grows with the thrust.
struct Hiss {
    rate: f32,
    low_band: Svf,
    high_band: Svf,
    rumble: Svf,
    /// Slow random walks of the two resonances, about -1 to 1.
    wander: [f32; 2],
    brightness: f32,
    amplitude: f32,
}

impl Hiss {
    fn new(rate: f32) -> Self {
        let mut rumble = Svf::default();
        rumble.tune(rate, 300.0, 0.7);
        Self {
            rate,
            low_band: Svf::default(),
            high_band: Svf::default(),
            rumble,
            wander: [0.0; 2],
            brightness: 0.0,
            amplitude: 0.0,
        }
    }

    fn control(&mut self, thrust: f32, noise: &mut Noise) {
        for w in &mut self.wander {
            *w = (*w * 0.995 + 0.06 * noise.next()).clamp(-1.0, 1.0);
        }
        let [a, b] = self.wander;
        self.low_band.tune(self.rate, 900.0 * (1.0 + 0.15 * a), 2.0);
        self.high_band.tune(
            self.rate,
            3200.0 * (0.8 + 0.4 * thrust) * (1.0 + 0.2 * b),
            3.0,
        );
        self.brightness = 0.3 + 0.4 * thrust;
        self.amplitude = thrust.powf(0.6);
    }

    fn sample(&mut self, noise: &mut Noise) -> f32 {
        let n = noise.next();
        let low = self.low_band.tick(n).band;
        let high = self.high_band.tick(n).band;
        let rumble = self.rumble.tick(n).low;
        0.53 * self.amplitude * (0.5 * low + self.brightness * high + 0.4 * rumble)
    }
}

/// A sub-bass sine with a filtered sawtooth an octave up, for small speakers, and brown
/// noise, all throbbing in amplitude.
struct Thrum {
    rate: f32,
    sub: Osc,
    saw: Osc,
    throb: Osc,
    filter: Svf,
    rumble: Svf,
    brown: f32,
    frequency: f32,
    throb_rate: f32,
    amplitude: f32,
}

impl Thrum {
    fn new(rate: f32) -> Self {
        let mut filter = Svf::default();
        filter.tune(rate, 220.0, 1.0);
        let mut rumble = Svf::default();
        rumble.tune(rate, 180.0, 0.7);
        Self {
            rate,
            sub: Osc::default(),
            saw: Osc::default(),
            throb: Osc::default(),
            filter,
            rumble,
            brown: 0.0,
            frequency: 0.0,
            throb_rate: 0.0,
            amplitude: 0.0,
        }
    }

    fn control(&mut self, thrust: f32) {
        self.frequency = 42.0 + 10.0 * thrust;
        self.throb_rate = 7.0 + 6.0 * thrust;
        self.amplitude = thrust.powf(0.6);
    }

    fn sample(&mut self, noise: &mut Noise) -> f32 {
        let dt = self.frequency / self.rate;
        let sub = self.sub.sine(dt);
        let saw = self.filter.tick(self.saw.saw(2.0 * dt)).low;
        self.brown = (self.brown + 0.02 * noise.next()) * 0.995;
        let rumble = self.rumble.tick(self.brown).low;
        let throb = 1.0 - 0.35 * (0.5 + 0.5 * self.throb.sine(self.throb_rate / self.rate));
        0.48 * self.amplitude * throb * (0.6 * sub + 0.5 * saw + 2.0 * rumble)
    }
}

/// Discharges at a rate growing with the thrust, each a burst of band-passed noise and a
/// short ping, with random timing and strength.
struct Pulsed {
    rate: f32,
    burst: Svf,
    ping: Osc,
    /// Seconds to the next discharge.
    wait: f32,
    burst_envelope: f32,
    ping_envelope: f32,
    burst_decay: f32,
    ping_decay: f32,
    discharges_per_second: f32,
    amplitude: f32,
}

impl Pulsed {
    fn new(rate: f32) -> Self {
        let mut burst = Svf::default();
        burst.tune(rate, 2200.0, 1.5);
        Self {
            rate,
            burst,
            ping: Osc::default(),
            wait: 0.0,
            burst_envelope: 0.0,
            ping_envelope: 0.0,
            burst_decay: decay(rate, 0.008),
            ping_decay: decay(rate, 0.02),
            discharges_per_second: 0.0,
            amplitude: 0.0,
        }
    }

    fn control(&mut self, thrust: f32) {
        self.discharges_per_second = 6.0 + 34.0 * thrust;
        self.amplitude = thrust.powf(0.4);
    }

    fn sample(&mut self, thrust: f32, noise: &mut Noise) -> f32 {
        self.wait -= 1.0 / self.rate;
        if self.wait <= 0.0 && thrust > 1e-3 {
            self.burst_envelope = 0.7 + 0.3 * noise.next().abs();
            self.ping_envelope = 1.0;
            self.ping.phase = 0.0;
            self.wait = (1.0 + 0.25 * noise.next()) / self.discharges_per_second;
        }
        let burst = self.burst.tick(noise.next()).band * self.burst_envelope;
        let ping = self.ping.sine(1100.0 / self.rate) * self.ping_envelope;
        self.burst_envelope *= self.burst_decay;
        self.ping_envelope *= self.ping_decay;
        0.72 * self.amplitude * (2.0 * burst + 0.5 * ping)
    }
}

/// The reaction control thrusters: a hiss of cold gas as strong as the torque, with a
/// valve click and a brief surge when they open, panned left for a counterclockwise turn
/// and right for a clockwise one.
struct Rcs {
    high_pass: Svf,
    band: Svf,
    click: Svf,
    strength: Smoother,
    pan: Smoother,
    /// The torque at the last control update, to notice the valves opening.
    torque: f32,
    target: f32,
    click_envelope: f32,
    surge: f32,
    click_decay: f32,
    surge_decay: f32,
    gains: (f32, f32),
}

impl Rcs {
    /// Below this fraction of `tau_max`, the valves are closed.
    const OPEN: f32 = 0.02;

    fn new(rate: f32) -> Self {
        let mut high_pass = Svf::default();
        high_pass.tune(rate, 1500.0, 0.7);
        let mut band = Svf::default();
        band.tune(rate, 5000.0, 1.2);
        let mut click = Svf::default();
        click.tune(rate, 2500.0, 3.0);
        Self {
            high_pass,
            band,
            click,
            // Valves snap open and the gas stops almost as fast.
            strength: Smoother::new(rate, 0.004, 0.07),
            pan: Smoother::new(rate, 0.02, 0.02),
            torque: 0.0,
            target: 0.0,
            click_envelope: 0.0,
            surge: 0.0,
            click_decay: decay(rate, 0.004),
            surge_decay: decay(rate, 0.03),
            gains: (1.0, 1.0),
        }
    }

    fn control(&mut self, torque: f32) {
        let was_open = self.torque.abs() > Self::OPEN;
        let open = torque.abs() > Self::OPEN;
        let reversed = was_open && open && torque.signum() != self.torque.signum();
        if open && (!was_open || reversed) {
            self.click_envelope = 1.0;
            self.surge = 1.0;
        }
        self.torque = torque;
        self.target = torque.abs().sqrt();
        // Equal-power panning, a little to the side.
        let pan = self.pan.value;
        let angle = FRAC_PI_4 * (1.0 + 0.35 * pan);
        self.gains = (SQRT_2 * angle.cos(), SQRT_2 * angle.sin());
    }

    fn sample(&mut self, noise: &mut Noise) -> (f32, f32) {
        let side = if self.torque > Self::OPEN {
            -1.0
        } else if self.torque < -Self::OPEN {
            1.0
        } else {
            self.pan.value
        };
        self.pan.next(side);
        let strength = self.strength.next(self.target);
        let n = noise.next();
        let hiss = 0.6 * self.high_pass.tick(n).high + 0.8 * self.band.tick(n).band;
        let click = self.click.tick(noise.next()).band * self.click_envelope;
        self.click_envelope *= self.click_decay;
        self.surge *= self.surge_decay;
        let out = 0.29 * (strength * (1.0 + 0.8 * self.surge) * hiss + 1.5 * click);
        (out * self.gains.0, out * self.gains.1)
    }
}

/// Per-sample factor of an exponential decay with time constant `seconds`.
fn decay(rate: f32, seconds: f32) -> f32 {
    (-1.0 / (seconds * rate)).exp()
}

/// A one-pole smoother toward a target, with separate rise and fall times.
#[derive(Clone, Copy)]
struct Smoother {
    value: f32,
    rise: f32,
    fall: f32,
}

impl Smoother {
    fn new(rate: f32, rise_seconds: f32, fall_seconds: f32) -> Self {
        Self {
            value: 0.0,
            rise: 1.0 - decay(rate, rise_seconds),
            fall: 1.0 - decay(rate, fall_seconds),
        }
    }

    fn next(&mut self, target: f32) -> f32 {
        let k = if target > self.value {
            self.rise
        } else {
            self.fall
        };
        self.value += k * (target - self.value);
        self.value
    }
}

/// A phase accumulator, from 0 to 1.
#[derive(Clone, Copy, Default)]
struct Osc {
    phase: f32,
}

impl Osc {
    /// Advances by `dt` cycles and returns the phase before.
    fn advance(&mut self, dt: f32) -> f32 {
        let phase = self.phase;
        self.phase += dt;
        self.phase -= self.phase.floor();
        phase
    }

    fn sine(&mut self, dt: f32) -> f32 {
        (TAU * self.advance(dt)).sin()
    }

    /// A band-limited sawtooth (polyBLEP), -1 to 1.
    fn saw(&mut self, dt: f32) -> f32 {
        let t = self.advance(dt);
        let blep = if t < dt {
            let t = t / dt;
            2.0 * t - t * t - 1.0
        } else if t > 1.0 - dt {
            let t = (t - 1.0) / dt;
            t * t + 2.0 * t + 1.0
        } else {
            0.0
        };
        2.0 * t - 1.0 - blep
    }
}

/// White noise from xorshift32, -1 to 1.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        // The top 24 bits, exactly representable.
        (x >> 8) as f32 / (1 << 23) as f32 - 1.0
    }
}

/// A state-variable filter (Simper's trapezoidal form): low-, band- and high-pass outputs
/// at once, stable when retuned on the fly.
#[derive(Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

struct SvfOut {
    low: f32,
    band: f32,
    high: f32,
}

impl Svf {
    fn tune(&mut self, rate: f32, cutoff: f32, q: f32) {
        let g = (PI * (cutoff / rate).min(0.49)).tan();
        self.k = 1.0 / q;
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    fn tick(&mut self, x: f32) -> SvfOut {
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        SvfOut {
            low: v2,
            band: v1,
            high: x - self.k * v1 - v2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    /// Renders `seconds` of stereo at full volume with `controls`, after a second to settle.
    fn render(controls: Controls, seconds: f32) -> Vec<f32> {
        let mut synth = Synth::new(RATE);
        synth.set(controls);
        let mut settle = vec![0.0; 2 * RATE as usize];
        synth.render(&mut settle, 2);
        let mut out = vec![0.0; 2 * (seconds * RATE) as usize];
        synth.render(&mut out, 2);
        out
    }

    fn full(ion: IonSound, thrust: f32, torque: f32) -> Controls {
        Controls {
            thrust,
            torque,
            ion,
            ion_gain: 1.0,
            rcs_gain: 1.0,
            master_gain: 1.0,
        }
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt()
    }

    #[test]
    fn silent_when_the_thrusters_are_off() {
        for ion in IonSound::ALL {
            let out = render(full(ion, 0.0, 0.0), 0.5);
            assert!(rms(&out) < 1e-4, "{ion:?} is silent at zero thrust");
        }
    }

    #[test]
    fn every_ion_sound_is_audible_and_not_too_loud() {
        for ion in IonSound::ALL {
            for thrust in [0.2, 1.0] {
                let out = render(full(ion, thrust, 0.0), 1.0);
                let level = rms(&out);
                assert!(
                    out.iter().all(|x| x.is_finite() && x.abs() <= 1.0),
                    "{ion:?} stays in range"
                );
                assert!(
                    (0.02..=0.35).contains(&level),
                    "{ion:?} at thrust {thrust}: rms {level}"
                );
            }
        }
    }

    #[test]
    fn rcs_hisses_and_pans_with_the_torque() {
        let left_turn = render(full(IonSound::Hum, 0.0, 1.0), 0.5);
        let (left, right): (Vec<f32>, Vec<f32>) = left_turn.chunks(2).map(|f| (f[0], f[1])).unzip();
        assert!(
            (0.02..=0.35).contains(&rms(&left)),
            "audible: {}",
            rms(&left)
        );
        assert!(
            rms(&left) > 1.3 * rms(&right),
            "counterclockwise is to the left"
        );
        let right_turn = render(full(IonSound::Hum, 0.0, -1.0), 0.5);
        let (left, right): (Vec<f32>, Vec<f32>) =
            right_turn.chunks(2).map(|f| (f[0], f[1])).unzip();
        assert!(rms(&right) > 1.3 * rms(&left), "clockwise is to the right");
    }

    #[test]
    fn volumes_scale_the_output() {
        let loud = rms(&render(full(IonSound::Hiss, 1.0, 0.0), 0.5));
        let mut quiet = full(IonSound::Hiss, 1.0, 0.0);
        quiet.master_gain = 0.25;
        let quiet = rms(&render(quiet, 0.5));
        assert!(quiet < 0.4 * loud, "master volume: {quiet} vs {loud}");
        let mut muted = full(IonSound::Hiss, 1.0, 0.0);
        muted.ion_gain = 0.0;
        assert!(rms(&render(muted, 0.5)) < 1e-4, "muted ion thruster");
    }

    #[test]
    fn mono_and_surround_outputs() {
        let mut synth = Synth::new(RATE);
        synth.set(full(IonSound::Whine, 1.0, 0.5));
        let mut mono = vec![0.0; 4800];
        synth.render(&mut mono, 1);
        assert!(rms(&mono) > 0.0, "mono gets the mix");
        let mut surround = vec![0.0; 6 * 4800];
        synth.render(&mut surround, 6);
        assert!(
            surround.chunks(6).all(|f| f[2..].iter().all(|&x| x == 0.0)),
            "only front left and right"
        );
    }
}
