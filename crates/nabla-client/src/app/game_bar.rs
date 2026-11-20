//! The bar along the bottom of the flight: home, pause and restart, the stability assist, the
//! autopilot and pointing, the time, and whether a controller is connected.

use super::{AIM_CONE_RANGE, Screen, ViewerApp};
use crate::game::Steering;
use crate::gamepad::Gamepads;
use crate::keys;
use crate::theme::{icons, palette};

/// Size of the icon that shows whether a controller is connected, in points.
pub(super) const CONTROLLER_STATUS_SIZE: f32 = 20.0;

impl ViewerApp {
    /// The bar along the bottom: pause and restart, the switches for the stability assist, the
    /// autopilot and pointing (with its cone), and the time. `compact`, for narrow screens,
    /// shortens the autopilot's, the cone's and the time's labels so the row fits a phone.
    pub(super) fn game_bar_ui(&mut self, ui: &mut egui::Ui, compact: bool) {
        let assisted = self.assisted();
        let game = &self.game;
        let (mut pause, mut restart, mut sas, mut autopilot) = (false, false, false, false);
        let mut home = false;
        ui.horizontal(|ui| {
            home = outline_button(ui, icons::outline::HOUSE)
                .on_hover_text("Back to the title screen")
                .clicked();
            let pause_icon = if game.paused {
                icons::PLAY
            } else {
                icons::PAUSE
            };
            pause = ui
                .button(pause_icon)
                .on_hover_ui(|ui| keys::tooltip(ui, "Pause / resume", &[keys::P]))
                .clicked();
            restart = outline_button(ui, icons::outline::ARROW_CLOCKWISE)
                .on_hover_ui(|ui| keys::tooltip(ui, "Restart", &[keys::R]))
                .clicked();
            ui.separator();
            let mut on = self.steering == Steering::Sas;
            sas = ui
                .toggle_value(&mut on, "SAS")
                .on_hover_ui(|ui| {
                    keys::tooltip(
                        ui,
                        "Stability assist: when you stop turning, stop the spin and hold the \
                         heading",
                        &[keys::T],
                    );
                })
                .changed();
            let mut engaged = assisted;
            let label = if compact { "Auto" } else { "Autopilot" };
            autopilot = ui
                .toggle_value(&mut engaged, label)
                .on_hover_ui(|ui| {
                    let action = format!(
                        "Autopilot: {}. Switch it off to take over",
                        self.game.objective().label()
                    );
                    keys::tooltip(ui, &action, &[keys::A]);
                })
                .changed();
            ui.add_enabled_ui(self.follow_ship && !assisted, |ui| {
                ui.toggle_value(&mut self.pointing, "Point")
                    .on_hover_text(
                        "Hold the mouse or a finger on the canvas: the ship turns toward it, \
                         and thrusts once it faces it within the cone",
                    )
                    .on_disabled_hover_text(
                        "Follow the ship (F), with the autopilot off, to point",
                    );
                if self.pointing {
                    ui.add(
                        egui::DragValue::new(&mut self.aim_cone)
                            .range(AIM_CONE_RANGE)
                            .speed(0.5)
                            .max_decimals(0)
                            .prefix(if compact { "" } else { "cone " })
                            .suffix(" deg"),
                    )
                    .on_hover_text(
                        "Half-angle of the cone around the pointer within which the ship \
                         thrusts. Drag or click to change",
                    );
                }
            });
            ui.separator();
            let (k, _) = game.moment();
            let time = if compact {
                format!("{:.2} s", game.time())
            } else {
                format!("t = {:.2} s, k = {k}", game.time())
            };
            ui.monospace(time);
            // A narrow bar has no room left for it; the banners still tell.
            if !compact {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    controller_status_ui(ui, &self.gamepads);
                });
            }
        });
        if home {
            self.screen = Screen::Title;
            self.notifications.clear();
        }
        if pause {
            self.toggle_pause();
        }
        if restart {
            self.start_game();
        }
        if sas {
            self.toggle_steering();
        }
        if autopilot {
            self.toggle_autopilot();
        }
    }
}

/// A button with an outline `icon`, at the size of a button's text.
pub(super) fn outline_button(ui: &mut egui::Ui, icon: &str) -> egui::Response {
    let size = egui::TextStyle::Button.resolve(ui.style()).size;
    ui.button(egui::RichText::new(icon).font(icons::outline::font(size)))
}

/// A controller icon: green with one connected, gray without, saying which on hover.
pub(super) fn controller_status_ui(ui: &mut egui::Ui, gamepads: &Gamepads) {
    let color = if gamepads.state().is_some() {
        palette::GREEN
    } else {
        palette::OVERLAY0
    };
    let hint = if cfg!(target_arch = "wasm32") {
        "No controller: connect one and press a button on it"
    } else {
        "No controller connected"
    };
    let icon = egui::RichText::new(icons::outline::GAME_CONTROLLER)
        .font(icons::outline::font(CONTROLLER_STATUS_SIZE))
        .color(color);
    ui.add(egui::Label::new(icon).selectable(false))
        .on_hover_text(gamepads.state().map_or(hint, |pad| pad.name.as_str()));
}
