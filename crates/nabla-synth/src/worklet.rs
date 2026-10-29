//! The synthesizer for the web app's `AudioWorklet` (`crates/nabla/assets/synth-worklet.js`).
//!
//! The worklet runs on the browser's audio rendering thread, whose scope has no DOM and no
//! `TextDecoder`, so it cannot load the app's wasm-bindgen module. Instead this crate is built
//! alone as a `cdylib` with no imports, `nabla_synth.wasm`, and the worklet calls these plain
//! functions: one synthesizer per wasm instance, so no pointer is ever passed in, and the only
//! address passed out is the output's. Natively they are ordinary functions, for the tests.
//!
//! [`synth_set`] and [`synth_render`] are the actions `Receive` and `Render` of the
//! specification in `docs/audio_worklet.typ`. When no message has come for [`QUIET_SECONDS`]
//! (the page's main thread stopped drawing frames: a hidden tab, a minimized window), the
//! thrusters go quiet. A slow frame, however slow, does not interrupt the sound.
//!
//! Nothing allocates after [`synth_init`], so wasm memory never grows, and the worklet's
//! views of the output stay valid.

#![expect(
    unsafe_code,
    reason = "the exported functions need #[unsafe(no_mangle)]"
)]

use std::cell::RefCell;

use crate::{Controls, IonSound, Synth};

/// Frames per render quantum of an `AudioWorklet`: the most [`synth_render`] renders at once.
pub const QUANTUM: usize = 128;

/// Seconds without a message after which the thrusters go quiet.
pub const QUIET_SECONDS: f32 = 1.0;

struct Worklet {
    synth: Synth,
    controls: Controls,
    /// Quanta rendered since the last message.
    idle: u32,
    /// Quanta without a message after which the thrusters go quiet.
    quiet_quanta: u32,
    interleaved: [f32; 2 * QUANTUM],
    /// The output, planar: left, then right.
    out: [[f32; QUANTUM]; 2],
}

thread_local! {
    static WORKLET: RefCell<Option<Box<Worklet>>> = const { RefCell::new(None) };
}

/// Creates the synthesizer for `sample_rate`; returns the address of its output: two planar
/// channels of [`QUANTUM`] `f32`s, left then right.
#[unsafe(no_mangle)]
pub extern "C" fn synth_init(sample_rate: f32) -> usize {
    let quanta_per_second = sample_rate / QUANTUM as f32;
    let worklet = Box::new(Worklet {
        synth: Synth::new(sample_rate),
        controls: Controls::default(),
        idle: 0,
        quiet_quanta: (QUIET_SECONDS * quanta_per_second).ceil() as u32,
        interleaved: [0.0; 2 * QUANTUM],
        out: [[0.0; QUANTUM]; 2],
    });
    let address = worklet.out.as_ptr() as usize;
    WORKLET.set(Some(worklet));
    address
}

/// Holds the controls of a frame of the app; `ion` is an index into [`IonSound::ALL`].
#[unsafe(no_mangle)]
pub extern "C" fn synth_set(
    thrust: f32,
    torque: f32,
    ion: u32,
    ion_gain: f32,
    rcs_gain: f32,
    master_gain: f32,
) {
    with(|w| {
        let ion = IonSound::ALL.get(ion as usize).copied().unwrap_or_default();
        w.controls = Controls {
            thrust,
            torque,
            ion,
            ion_gain,
            rcs_gain,
            master_gain,
        };
        w.idle = 0;
    });
}

/// Renders one quantum of `frames` frames (at most [`QUANTUM`]) into the output.
#[unsafe(no_mangle)]
pub extern "C" fn synth_render(frames: u32) {
    with(|w| {
        let frames = (frames as usize).min(QUANTUM);
        let mut effective = w.controls;
        if w.idle >= w.quiet_quanta {
            (effective.thrust, effective.torque) = (0.0, 0.0);
        }
        w.idle = w.idle.saturating_add(1);
        w.synth.set(effective);
        let interleaved = &mut w.interleaved[..2 * frames];
        w.synth.render(interleaved, 2);
        let [left, right] = &mut w.out;
        for ((&[l, r], left), right) in interleaved.as_chunks().0.iter().zip(left).zip(right) {
            *left = l;
            *right = r;
        }
    });
}

/// Runs `f` on the synthesizer, if [`synth_init`] has created it.
fn with(f: impl FnOnce(&mut Worklet)) {
    WORKLET.with_borrow_mut(|w| {
        if let Some(w) = w.as_deref_mut() {
            f(w);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    /// A copy of the output.
    fn output() -> [[f32; QUANTUM]; 2] {
        let mut out = [[0.0; QUANTUM]; 2];
        with(|w| out = w.out);
        out
    }

    fn rms(out: &[[f32; QUANTUM]; 2]) -> f32 {
        let squares: f32 = out.iter().flatten().map(|x| x * x).sum();
        (squares / (2 * QUANTUM) as f32).sqrt()
    }

    fn full_thrust() {
        synth_set(1.0, 0.0, 0, 1.0, 1.0, 1.0);
    }

    /// Renders `seconds` of quanta, and returns the last one.
    fn render(seconds: f32) -> [[f32; QUANTUM]; 2] {
        for _ in 0..(seconds * RATE / QUANTUM as f32).ceil() as usize {
            synth_render(QUANTUM as u32);
        }
        output()
    }

    #[test]
    fn nothing_happens_before_init() {
        // Each test runs on a thread of its own, with a synthesizer of its own.
        full_thrust();
        synth_render(QUANTUM as u32);
        WORKLET.with_borrow(|w| assert!(w.is_none(), "no synthesizer yet"));
    }

    #[test]
    fn planar_output_matches_interleaved() {
        synth_init(RATE);
        full_thrust();
        let mut reference = Synth::new(RATE);
        reference.set(Controls {
            thrust: 1.0,
            torque: 0.0,
            ion: IonSound::ALL[0],
            ion_gain: 1.0,
            rcs_gain: 1.0,
            master_gain: 1.0,
        });
        let mut interleaved = [0.0; 2 * QUANTUM];
        for _ in 0..100 {
            synth_render(QUANTUM as u32);
            reference.render(&mut interleaved, 2);
        }
        let out = output();
        assert!(rms(&out) > 0.01, "audible");
        for (i, frame) in interleaved.as_chunks::<2>().0.iter().enumerate() {
            assert_eq!([out[0][i], out[1][i]], *frame, "frame {i}");
        }
    }

    #[test]
    fn short_quanta_fill_the_start_of_the_output() {
        synth_init(RATE);
        full_thrust();
        render(0.5);
        let before = output();
        synth_render(10);
        let after = output();
        assert_ne!(after[0][..10], before[0][..10], "rendered");
        assert_eq!(after[0][10..], before[0][10..], "left alone");
    }

    #[test]
    fn goes_quiet_without_messages_and_recovers_on_one() {
        synth_init(RATE);
        full_thrust();
        assert!(rms(&render(0.9 * QUIET_SECONDS)) > 0.01, "playing");
        // Going quiet is a fade: the ion sounds take about a second and a half to die out.
        assert!(
            rms(&render(0.1 * QUIET_SECONDS + 2.0)) < 1e-3,
            "quiet after the fade"
        );
        full_thrust();
        assert!(rms(&render(0.3)) > 0.01, "playing again");
    }

    #[test]
    fn unknown_ion_sounds_fall_back_to_the_default() {
        synth_init(RATE);
        synth_set(0.5, 0.0, u32::MAX, 1.0, 1.0, 1.0);
        with(|w| assert_eq!(w.controls.ion, IonSound::default(), "default sound"));
    }
}
