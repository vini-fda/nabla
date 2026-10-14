//! Thruster sounds: plays [`crate::synth`] on the default audio output (natively, or with Web
//! Audio in the browser), following the replay's thrust and torque while it plays, and the
//! "Sound" section of the controls panel: the ion thruster's sound, volume knobs, and
//! buttons to hear each thruster on its own.
//!
//! The audio device is opened the first time there is something to play, which in a browser
//! is after the page has been interacted with, as autoplay rules require. The app hands the
//! audio callback its controls through atomics, and bumps a heartbeat every frame: when the
//! frames stop (a hidden tab, a minimized window), the callback goes quiet on its own.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use egui::{Pos2, Response, Sense, Stroke, Ui, Vec2, vec2};

use crate::synth::{Controls, IonSound, Synth};
use crate::theme::palette;

/// Seconds without a frame after which the audio callback goes quiet.
const HEARTBEAT_TIMEOUT: f32 = 0.25;

/// Default knob positions: master, ion thruster, reaction control thrusters.
const DEFAULT_VOLUMES: [f32; 3] = [0.7, 0.8, 0.25];

/// Sound settings, and the audio output once opened.
pub struct Sound {
    enabled: bool,
    ion: IonSound,
    /// Knob positions (0 to 1) for the master, ion thruster and RCS volumes.
    volumes: [f32; 3],
    /// Whether a "test" button is held this frame.
    test_ion: bool,
    test_rcs: bool,
    shared: Arc<Shared>,
    stream: Option<cpal::Stream>,
    /// Why the output could not be opened; not retried until sound is turned back on.
    error: Option<String>,
}

impl Default for Sound {
    fn default() -> Self {
        Self {
            enabled: true,
            ion: IonSound::default(),
            volumes: DEFAULT_VOLUMES,
            test_ion: false,
            test_rcs: false,
            shared: Arc::default(),
            stream: None,
            error: None,
        }
    }
}

impl Sound {
    /// Call once per frame, after the UI: `levels` is the thrust and torque (as fractions of
    /// their maxima) while the replay plays, `None` while it doesn't.
    pub fn update(&mut self, ctx: &egui::Context, levels: Option<(f32, f32)>) {
        let test_ion = std::mem::take(&mut self.test_ion);
        let test_rcs = std::mem::take(&mut self.test_rcs);
        if !self.enabled {
            self.stream = None;
            return;
        }
        let (mut thrust, mut torque) = levels.unwrap_or_default();
        if test_ion {
            thrust = 1.0;
        }
        if test_rcs {
            torque = 1.0;
        }
        if test_ion || test_rcs {
            // Keep the heartbeat going while a button is held still.
            ctx.request_repaint();
        }
        let wanted = levels.is_some() || test_ion || test_rcs;
        if wanted && self.stream.is_none() && self.error.is_none() {
            match open(Arc::clone(&self.shared)) {
                Ok(stream) => self.stream = Some(stream),
                Err(e) => {
                    log::warn!("No sound: {e}");
                    self.error = Some(e);
                }
            }
        }
        // A browser keeps audio suspended until the page is interacted with: should the
        // output have been opened before that, resume it on the next click or key press.
        if cfg!(target_arch = "wasm32")
            && let Some(stream) = &self.stream
            && ctx.input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::PointerButton { pressed: true, .. }
                            | egui::Event::Key { pressed: true, .. }
                            | egui::Event::Touch { .. }
                    )
                })
            })
            && let Err(e) = stream.play()
        {
            log::warn!("Audio output: {e}");
        }
        // Knob positions to gains: squaring is close to how loudness is heard.
        let [master, ion, rcs] = self.volumes.map(|v| v * v);
        self.shared.publish(&Controls {
            thrust,
            torque,
            ion: self.ion,
            ion_gain: ion,
            rcs_gain: rcs,
            master_gain: master,
        });
    }

    /// The "Sound" section of the controls panel.
    pub fn ui(&mut self, ui: &mut Ui) {
        ui.heading("Sound");
        if ui.checkbox(&mut self.enabled, "Thruster sounds").changed() {
            self.error = None;
        }
        ui.add_enabled_ui(self.enabled, |ui| {
            egui::ComboBox::from_label("ion thruster")
                .selected_text(self.ion.name())
                .show_ui(ui, |ui| {
                    for sound in IonSound::ALL {
                        ui.selectable_value(&mut self.ion, sound, sound.name())
                            .on_hover_text(sound.description());
                    }
                })
                .response
                .on_hover_text(self.ion.description());
            ui.horizontal(|ui| {
                let [master, ion, rcs] = &mut self.volumes;
                knob(ui, master, DEFAULT_VOLUMES[0], "Master");
                knob(ui, ion, DEFAULT_VOLUMES[1], "Ion");
                knob(ui, rcs, DEFAULT_VOLUMES[2], "RCS");
            });
            ui.horizontal(|ui| {
                self.test_ion = ui
                    .button("Test ion")
                    .on_hover_text("Hold to fire the ion thruster at full thrust")
                    .is_pointer_button_down_on();
                self.test_rcs = ui
                    .button("Test RCS")
                    .on_hover_text("Hold to fire the reaction control thrusters at full torque")
                    .is_pointer_button_down_on();
            });
        });
        if let Some(e) = &self.error {
            ui.colored_label(palette::RED, format!("No sound: {e}"));
        }
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
                if quiet_frames > HEARTBEAT_TIMEOUT * rate {
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

/// A rotary knob for a volume from 0 to 1, labeled underneath. Drag up or right to turn it
/// up; double-click to reset it to `default`.
fn knob(ui: &mut Ui, value: &mut f32, default: f32, label: &str) -> Response {
    const SIZE: f32 = 34.0;
    /// The knob turns from 7:30 (zero) clockwise through 12:00 to 4:30 (full).
    const START: f32 = 0.75 * std::f32::consts::PI;
    const SWEEP: f32 = 1.5 * std::f32::consts::PI;

    let column = vec2(SIZE + 20.0, SIZE + 18.0);
    ui.allocate_ui_with_layout(column, egui::Layout::top_down(egui::Align::Center), |ui| {
        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::splat(SIZE), Sense::click_and_drag());
        if response.dragged() {
            let delta = response.drag_delta();
            *value = (*value + (delta.x - delta.y) / 120.0).clamp(0.0, 1.0);
            response.mark_changed();
        }
        if response.double_clicked() {
            *value = default;
            response.mark_changed();
        }
        let enabled = ui.is_enabled();
        response.widget_info(|| egui::WidgetInfo::slider(enabled, f64::from(*value), label));

        let visuals = ui.style().interact(&response);
        let painter = ui.painter();
        let center = rect.center();
        let radius = 0.5 * SIZE - 2.0;
        let point = |angle: f32, r: f32| center + r * vec2(angle.cos(), angle.sin());
        let arc = |from: f32, to: f32, r: f32| -> Vec<Pos2> {
            let steps = ((to - from).abs() * 12.0).ceil().max(1.0) as usize;
            (0..=steps)
                .map(|i| point(from + (to - from) * i as f32 / steps as f32, r))
                .collect()
        };
        let accent = if enabled {
            palette::MAUVE
        } else {
            palette::OVERLAY0
        };
        painter.add(egui::Shape::line(
            arc(START, START + SWEEP, radius),
            Stroke::new(3.0, palette::SURFACE2),
        ));
        if *value > 0.0 {
            painter.add(egui::Shape::line(
                arc(START, START + SWEEP * *value, radius),
                Stroke::new(3.0, accent),
            ));
        }
        painter.circle(center, radius - 5.0, visuals.bg_fill, visuals.bg_stroke);
        let angle = START + SWEEP * *value;
        painter.line_segment(
            [point(angle, radius - 13.0), point(angle, radius - 6.0)],
            Stroke::new(2.0, visuals.fg_stroke.color),
        );
        ui.add(egui::Label::new(egui::RichText::new(label).small()).selectable(false));
        response.on_hover_text(format!(
            "{label} volume: {:.0}%. Drag to change, double-click to reset.",
            100.0 * *value
        ))
    })
    .inner
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
