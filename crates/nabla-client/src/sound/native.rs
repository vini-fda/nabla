//! The native audio output: cpal calls the synthesizer on the operating system's audio thread.
//!
//! The app hands the audio callback its controls through atomics, and bumps a heartbeat every
//! frame: when the frames stop (a minimized window), the callback goes quiet on its own.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use nabla_synth::worklet::QUIET_SECONDS;
use nabla_synth::{Controls, Synth};

/// The default output, playing the synthesizer.
pub struct Output {
    shared: Arc<Shared>,
    _stream: cpal::Stream,
}

impl Drop for Output {
    fn drop(&mut self) {
        log::debug!("audio: stream dropped");
    }
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

    /// What the audio callback did since the last call, on one line for the log: how often it
    /// ran and how many frames it rendered, the loudest sample, and whether it went quiet for
    /// want of heartbeats from the app.
    pub fn status(&self) -> String {
        let callbacks = self.shared.callbacks.swap(0, Relaxed);
        let frames = self.shared.frames.swap(0, Relaxed);
        let peak = f32::from_bits(self.shared.peak.swap(0, Relaxed));
        let quiet = self.shared.quiet.load(Relaxed);
        let per_callback = if callbacks == 0 {
            0
        } else {
            frames / u64::from(callbacks)
        };
        format!(
            "callbacks {callbacks}, frames {frames} ({per_callback} a callback), peak {peak:.3}, \
             quiet {quiet}, rate {} Hz, {} channels",
            self.shared.rate.load(Relaxed),
            self.shared.channels.load(Relaxed)
        )
    }
}

/// The audio callback's inputs, written by the app every frame.
#[derive(Default)]
struct Shared {
    /// `f32` bits of [`Controls`]' fields.
    thrust: AtomicU32,
    torque: AtomicU32,
    gains: [AtomicU32; 3],
    /// Bumped every frame.
    heartbeat: AtomicU32,
    /// What the callback did since the status was last read, for the log: the callbacks, the
    /// frames they rendered, and the loudest sample (`f32` bits, which order like the
    /// non-negative floats they are).
    callbacks: AtomicU32,
    frames: AtomicU64,
    peak: AtomicU32,
    /// The callback has gone quiet for want of heartbeats.
    quiet: AtomicBool,
    /// The stream's format, for the log.
    rate: AtomicU32,
    channels: AtomicU32,
}

impl Shared {
    fn publish(&self, c: &Controls) {
        self.thrust.store(c.thrust.to_bits(), Relaxed);
        self.torque.store(c.torque.to_bits(), Relaxed);
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
        Controls {
            thrust: f(&self.thrust),
            torque: f(&self.torque),
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
    log::debug!(
        "audio: device {}, {format}, {} Hz, {} channels, buffer {:?}",
        device
            .description()
            .map_or_else(|e| format!("(unnamed: {e})"), |d| d.to_string()),
        config.sample_rate,
        config.channels,
        config.buffer_size
    );
    shared.rate.store(config.sample_rate, Relaxed);
    shared.channels.store(u32::from(config.channels), Relaxed);
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
                let quiet = quiet_frames > QUIET_SECONDS * rate;
                if quiet {
                    controls.thrust = 0.0;
                    controls.torque = 0.0;
                }
                synth.set(controls);
                buffer.resize(data.len(), 0.0);
                synth.render(&mut buffer, channels);
                // The log reads these on the app's thread: nothing here allocates or blocks.
                let peak = buffer.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
                shared.callbacks.fetch_add(1, Relaxed);
                shared
                    .frames
                    .fetch_add((data.len() / channels.max(1)) as u64, Relaxed);
                shared.peak.fetch_max(peak.to_bits(), Relaxed);
                shared.quiet.store(quiet, Relaxed);
                for (out, &sample) in data.iter_mut().zip(&buffer) {
                    *out = T::from_sample(sample);
                }
            },
            |e| log::warn!("audio: output stream error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    log::debug!("audio: stream playing");
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_round_trip_through_the_shared_atomics() {
        let shared = Shared::default();
        let controls = Controls {
            thrust: 0.25,
            torque: -0.5,
            ion_gain: 0.1,
            rcs_gain: 0.2,
            master_gain: 0.3,
        };
        shared.publish(&controls);
        assert_eq!(shared.controls(), controls);
    }
}
