//! The native audio output: cpal calls the synthesizer on the operating system's audio thread.
//!
//! The app hands the audio callback its controls through atomics, and bumps a heartbeat every
//! frame: when the frames stop (a minimized window), the callback goes quiet on its own.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use nabla_synth::worklet::QUIET_SECONDS;
use nabla_synth::{Controls, IonSound, Synth};

/// The default output, playing the synthesizer.
pub struct Output {
    shared: Arc<Shared>,
    _stream: cpal::Stream,
}

impl Output {
    /// Opens the default output and starts playing the synthesizer on it.
    pub fn open() -> Result<Self, String> {
        let shared = Arc::<Shared>::default();
        let stream = open(Arc::clone(&shared))?;
        Ok(Self {
            shared,
            _stream: stream,
        })
    }

    /// Hands the audio callback the controls of this frame.
    pub fn publish(&self, controls: &Controls) {
        self.shared.publish(controls);
    }
}

/// The audio callback's inputs, written by the app every frame.
#[derive(Default)]
struct Shared {
    /// `f32` bits of [`Controls`]' fields, and the ion sound's index in [`IonSound::ALL`].
    thrust: AtomicU32,
    torque: AtomicU32,
    ion: AtomicU32,
    gains: [AtomicU32; 3],
    /// Bumped every frame.
    heartbeat: AtomicU32,
}

impl Shared {
    fn publish(&self, c: &Controls) {
        let ion = IonSound::ALL.iter().position(|&s| s == c.ion).unwrap_or(0);
        self.thrust.store(c.thrust.to_bits(), Relaxed);
        self.torque.store(c.torque.to_bits(), Relaxed);
        self.ion.store(ion as u32, Relaxed);
        for (gain, value) in self
            .gains
            .iter()
            .zip([c.ion_gain, c.rcs_gain, c.master_gain])
        {
            gain.store(value.to_bits(), Relaxed);
        }
        self.heartbeat.fetch_add(1, Relaxed);
    }

    fn controls(&self) -> Controls {
        let f = |a: &AtomicU32| f32::from_bits(a.load(Relaxed));
        let ion = IonSound::ALL
            .get(self.ion.load(Relaxed) as usize)
            .copied()
            .unwrap_or_default();
        Controls {
            thrust: f(&self.thrust),
            torque: f(&self.torque),
            ion,
            ion_gain: f(&self.gains[0]),
            rcs_gain: f(&self.gains[1]),
            master_gain: f(&self.gains[2]),
        }
    }
}

/// Opens the default output and starts playing the synthesizer on it.
fn open(shared: Arc<Shared>) -> Result<cpal::Stream, String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no audio output device")?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let format = config.sample_format();
    let config = StreamConfig::from(config);
    match format {
        SampleFormat::F32 => play::<f32>(&device, &config, shared),
        SampleFormat::I16 => play::<i16>(&device, &config, shared),
        SampleFormat::U16 => play::<u16>(&device, &config, shared),
        SampleFormat::I32 => play::<i32>(&device, &config, shared),
        other => Err(format!("unsupported sample format {other}")),
    }
}

fn play<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &StreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String> {
    let channels = usize::from(config.channels);
    let rate = config.sample_rate as f32;
    let mut synth = Synth::new(rate);
    let mut buffer = Vec::new();
    let mut heartbeat = 0;
    let mut quiet_frames = 0.0;
    let stream = device
        .build_output_stream(
            *config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let mut controls = shared.controls();
                let beat = shared.heartbeat.load(Relaxed);
                if beat == heartbeat {
                    quiet_frames += (data.len() / channels.max(1)) as f32;
                } else {
                    heartbeat = beat;
                    quiet_frames = 0.0;
                }
                if quiet_frames > QUIET_SECONDS * rate {
                    controls.thrust = 0.0;
                    controls.torque = 0.0;
                }
                synth.set(controls);
                buffer.resize(data.len(), 0.0);
                synth.render(&mut buffer, channels);
                for (out, &sample) in data.iter_mut().zip(&buffer) {
                    *out = T::from_sample(sample);
                }
            },
            |e| log::warn!("Audio output: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_round_trip_through_the_shared_atomics() {
        let shared = Shared::default();
        for ion in IonSound::ALL {
            let controls = Controls {
                thrust: 0.25,
                torque: -0.5,
                ion,
                ion_gain: 0.1,
                rcs_gain: 0.2,
                master_gain: 0.3,
            };
            shared.publish(&controls);
            assert_eq!(shared.controls(), controls, "{ion:?}");
        }
    }
}
