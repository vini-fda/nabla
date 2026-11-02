//! Thruster sounds: plays the [`nabla_synth`] synthesizer on the default audio output,
//! following the replay's thrust and torque while it plays, and the "Sound" section of the
//! controls panel: volume knobs, and buttons to hear each thruster on its own.
//!
//! Natively, cpal calls the synthesizer on the operating system's audio thread
//! (`sound/native.rs`). In the browser, an `AudioWorklet` runs it on the browser's audio
//! rendering thread (`sound/web.rs`): the page's main thread, which also draws the frames,
//! only posts it the controls, so slow frames cannot starve the audio. See
//! `docs/audio_worklet.typ`.
//!
//! Everything it logs begins with `audio:`, so that `RUST_LOG=nabla_client::sound=debug` and a
//! `grep audio:` show what the sound is doing: changes at `debug` (sound on or off, the output
//! opened or closed, a thruster starting or stopping, the knobs), a status line every two
//! seconds from the backend, and at `trace` the controls published every frame.
//!
//! The audio output is opened the first time there is something to play, which in a browser
//! is after the page has been interacted with, as autoplay rules require. The app publishes
//! the controls every frame; when the frames stop for a second (a hidden tab, a minimized
//! window), the thrusters go quiet on their own. `index.html` sets the page's audio session
//! to "playback", so iOS plays it in silent mode.

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

use egui::{Pos2, Response, Sense, Stroke, Ui, Vec2, vec2};
use nabla_synth::Controls;

#[cfg(not(target_arch = "wasm32"))]
use self::native::Output;
#[cfg(target_arch = "wasm32")]
use self::web::Output;
use crate::theme::palette;

/// Default knob positions: master, ion thruster, reaction control thrusters.
const DEFAULT_VOLUMES: [f32; 3] = [0.7, 0.8, 0.25];

/// How often the backend's status is logged, in seconds of the UI's time.
const STATUS_PERIOD: f64 = 2.0;

/// What was last logged, so that only changes are.
#[derive(Default)]
struct Logged {
    wanted: bool,
    thrusting: bool,
    torquing: bool,
    volumes: [f32; 3],
    /// UI time of the last status line.
    status_at: f64,
}

/// Sound settings, and the audio output once opened.
pub struct Sound {
    enabled: bool,
    /// Knob positions (0 to 1) for the master, ion thruster and RCS volumes.
    volumes: [f32; 3],
    /// Whether a "test" button is held this frame.
    test_ion: bool,
    test_rcs: bool,
    output: Option<Output>,
    /// Why the output could not be opened; not retried until sound is turned back on.
    error: Option<String>,
    logged: Logged,
}

impl Default for Sound {
    fn default() -> Self {
        Self {
            enabled: true,
            volumes: DEFAULT_VOLUMES,
            test_ion: false,
            test_rcs: false,
            output: None,
            error: None,
            logged: Logged::default(),
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
            if self.output.take().is_some() {
                log::debug!("audio: sound off, output closed");
            }
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
            // Keep publishing while a button is held still, or the sound goes quiet.
            ctx.request_repaint();
        }
        let wanted = levels.is_some() || test_ion || test_rcs;
        if wanted != self.logged.wanted {
            log::debug!(
                "audio: {} (levels {levels:?}, test ion {test_ion}, test rcs {test_rcs})",
                if wanted { "playing" } else { "nothing to play" }
            );
            self.logged.wanted = wanted;
        }
        if wanted && self.output.is_none() && self.error.is_none() {
            log::debug!("audio: opening the output");
            match Output::open() {
                Ok(output) => {
                    log::debug!("audio: output opened");
                    self.output = Some(output);
                }
                Err(e) => self.fail(e),
            }
        }
        #[cfg(target_arch = "wasm32")]
        self.browser_events(ctx);
        let Some(output) = &self.output else {
            return;
        };
        // Knob positions to gains: squaring is close to how loudness is heard.
        let [master, ion, rcs] = self.volumes.map(|v| v * v);
        let controls = Controls {
            thrust,
            torque,
            ion_gain: ion,
            rcs_gain: rcs,
            master_gain: master,
        };
        output.publish(&controls);
        log::trace!(
            "audio: publish thrust {thrust:.3} torque {torque:.3} gains master {master:.3} ion {ion:.3} rcs {rcs:.3}"
        );
        let time = ctx.input(|i| i.time);
        self.log_changes(thrust, torque, time);
    }

    /// Logs what changed since the last frame, and the backend's status every
    /// [`STATUS_PERIOD`] seconds.
    fn log_changes(&mut self, thrust: f32, torque: f32, time: f64) {
        let logged = &mut self.logged;
        if (thrust > 0.0) != logged.thrusting {
            logged.thrusting = thrust > 0.0;
            log::debug!(
                "audio: ion thruster {} (thrust {thrust:.3})",
                if logged.thrusting { "on" } else { "off" }
            );
        }
        if (torque != 0.0) != logged.torquing {
            logged.torquing = torque != 0.0;
            log::debug!(
                "audio: rcs thrusters {} (torque {torque:.3})",
                if logged.torquing { "on" } else { "off" }
            );
        }
        if self.volumes != logged.volumes {
            logged.volumes = self.volumes;
            let [master, ion, rcs] = self.volumes;
            log::debug!("audio: knobs master {master:.2} ion {ion:.2} rcs {rcs:.2}");
        }
        if time - logged.status_at >= STATUS_PERIOD {
            logged.status_at = time;
            if let Some(output) = &self.output {
                log::debug!("audio: status {}", output.status());
            }
        }
    }

    /// Closes the output, and shows why until sound is turned back on.
    fn fail(&mut self, e: String) {
        log::warn!("audio: no sound: {e}");
        self.error = Some(e);
        self.output = None;
    }

    /// In the browser, the output loads in the background, and may fail then. And audio stays
    /// suspended until the page is interacted with: should the output have been opened before
    /// that, resume it on the next click or key press.
    #[cfg(target_arch = "wasm32")]
    fn browser_events(&mut self, ctx: &egui::Context) {
        let Some(output) = &self.output else {
            return;
        };
        if let Some(e) = output.take_error() {
            self.fail(e);
            return;
        }
        let gesture = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::PointerButton { pressed: true, .. }
                        | egui::Event::Key { pressed: true, .. }
                        | egui::Event::Touch { .. }
                )
            })
        });
        if gesture {
            log::debug!("audio: page interaction, resuming the context");
            output.resume();
        }
    }

    /// The "Sound" section of the controls panel.
    pub fn ui(&mut self, ui: &mut Ui) {
        ui.heading("Sound");
        if ui.checkbox(&mut self.enabled, "Thruster sounds").changed() {
            self.error = None;
            log::debug!(
                "audio: thruster sounds {}",
                if self.enabled { "enabled" } else { "disabled" }
            );
        }
        ui.add_enabled_ui(self.enabled, |ui| {
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
