//! The flight's screen: the canvas with the settings and the plots beside it, laid out for wide
//! and narrow windows, and the flight's keys and controller buttons.

use super::side_panel::canvas_inset;
use super::{CONTROLS_PANEL, CompactView, RIGHT_PANEL, RightView, ViewerApp};
use crate::game::{Aim, Input};
use crate::gamepad;
use crate::theme::palette;

/// Below this width (in points), e.g. on phones, the panels are shown one at a time.
const COMPACT_WIDTH: f32 = 800.0;

impl ViewerApp {
    /// The flight's screen: the canvas, with the controls and the plots beside it.
    pub(super) fn flight_ui(&mut self, ui: &mut egui::Ui) {
        // Advance the flight first, so every panel shows the same instant.
        self.handle_keys(ui);
        self.handle_controller_buttons();
        let controller = self
            .gamepads
            .state()
            .map_or_else(Input::default, gamepad::State::input);
        let mut input = Self::keyboard_input(ui)
            .or(self.game.pads.held())
            .or(controller);
        input.aim = self.aiming.map(|(_, heading)| Aim {
            heading,
            cone: self.aim_cone.to_radians(),
        });
        self.input_thrust = input.thrust;
        let frame_time = f64::from(ui.input(|i| i.stable_dt));
        let was_landed = self.game.landed().is_some();
        self.game.advance(frame_time, input, self.steering);
        if let Some(fault) = self.game.take_fault() {
            self.notifications
                .post(format!("Flight assist switched off: {fault}"), palette::RED);
            self.autopilot_fault = Some(fault);
        }
        if let Some(n) = self.game.landed()
            && !was_landed
        {
            self.notify_landing(n);
        }
        if !self.game.paused || self.notifications.any() {
            ui.ctx().request_repaint();
        }
        self.refresh_orbit_prediction();
        if self.orbit_check_running() {
            ui.ctx().request_repaint();
        }
        self.refresh_prediction();

        let compact = ui.available_width() < COMPACT_WIDTH;
        // The tab bar only exists in the compact layout; the wide layout shows everything.
        if compact {
            egui::Panel::top("top_panel").show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (view, label) in [
                        (CompactView::Canvas, "View"),
                        (CompactView::Controls, CONTROLS_PANEL.titles[0]),
                        (CompactView::Plots, "Plots"),
                        (CompactView::Objective, RIGHT_PANEL.titles[1]),
                    ] {
                        ui.selectable_value(&mut self.compact_view, view, label);
                    }
                });
            });
        }

        if compact {
            self.compact_layout(ui);
        } else {
            self.wide_layout(ui);
        }
    }

    /// F: follow the ship or stop following it. P: pause/resume, R: restart, T: the stability
    /// assist on or off (as in KSP) and A: the autopilot on or off, the other keys flying the
    /// ship (see [`Self::keyboard_input`]).
    fn handle_keys(&mut self, ui: &egui::Ui) {
        if ui.ctx().memory(|m| m.focused().is_some()) {
            return;
        }
        // Not with a modifier, which is the browser's or the system's (Cmd+F: find).
        let plain = |key| ui.input(|i| i.key_pressed(key) && i.modifiers.is_none());
        if plain(egui::Key::F) {
            self.follow_ship = !self.follow_ship;
        }
        if plain(egui::Key::P) {
            self.toggle_pause();
        }
        if plain(egui::Key::R) {
            self.start_game();
        }
        if plain(egui::Key::T) {
            self.toggle_steering();
        }
        if plain(egui::Key::A) {
            self.toggle_autopilot();
        }
    }

    /// The flight's controls held on the keyboard: Space (or Up) thrusts, Left and Right turn,
    /// and Shift scales them down for fine control.
    pub(super) fn keyboard_input(ui: &egui::Ui) -> Input {
        if ui.ctx().memory(|m| m.focused().is_some()) {
            return Input::default();
        }
        ui.input(|i| {
            // A key pressed and released between two frames still counts, for that frame.
            let held = |key| i.key_down(key) || i.key_pressed(key);
            let left = f64::from(u8::from(held(egui::Key::ArrowLeft)));
            let right = f64::from(u8::from(held(egui::Key::ArrowRight)));
            Input {
                thrust: f64::from(u8::from(held(egui::Key::Space) || held(egui::Key::ArrowUp))),
                turn: left - right,
                fine: i.modifiers.shift,
                aim: None,
            }
        })
    }

    /// The controller's objective/preview/engage/manual commands. Options pauses and Create
    /// shows or hides its guide.
    fn handle_controller_buttons(&mut self) {
        let Some(pad) = self.gamepads.state().cloned() else {
            return;
        };
        let pressed = |button| pad.just_pressed(button);
        let (sas, autopilot, pause, view) = (
            pressed(gamepad::Button::North),
            pressed(gamepad::Button::West),
            pressed(gamepad::Button::Start),
            pressed(gamepad::Button::Select),
        );
        if sas {
            self.toggle_steering();
        }
        let orbit_handled = self.orbit_buttons(&pad);
        if autopilot && !orbit_handled {
            self.engage_objective();
        }
        if pause {
            self.toggle_pause();
        }
        if view {
            self.toggle_controller_guide();
        }
    }

    /// Desktop layout: the game's bar along the bottom, controls left and the plots or the
    /// objective right above it.
    fn wide_layout(&mut self, ui: &mut egui::Ui) {
        // The bar first, across the whole window, so neither side panel (nor its collapsed
        // title bar, which slides back in when the pointer comes near) reaches it.
        egui::Panel::bottom("game_bar").show(ui, |ui| self.game_bar_ui(ui, false));
        let mut show_controls = self.show_controls;
        CONTROLS_PANEL.show(ui, &mut show_controls, &mut 0, |ui, _| self.controls_ui(ui));
        self.show_controls = show_controls;
        let mut show_plots = self.show_plots;
        let mut view = self.right_view as usize;
        let reveal =
            RIGHT_PANEL.show(
                ui,
                &mut show_plots,
                &mut view,
                |ui, view| match RightView::ALL[view] {
                    RightView::Plots => self.plots_ui(ui),
                    RightView::Objective => self.objective_ui(ui),
                },
            );
        self.right_view = RightView::ALL[view];
        self.show_plots = show_plots;
        egui::CentralPanel::default().show(ui, |ui| self.canvas_ui(ui, canvas_inset(reveal)));
    }

    /// Narrow-screen layout: one full-size view at a time, chosen in the top bar; the game's
    /// bar stays available below the canvas and the plots.
    fn compact_layout(&mut self, ui: &mut egui::Ui) {
        if self.compact_view != CompactView::Controls {
            egui::Panel::bottom("game_bar").show(ui, |ui| self.game_bar_ui(ui, true));
        }
        // No heading above the controls: the selected tab already names the view.
        egui::CentralPanel::default().show(ui, |ui| match self.compact_view {
            CompactView::Canvas => self.canvas_ui(ui, 0.0),
            CompactView::Controls => self.controls_ui(ui),
            CompactView::Plots => self.plots_ui(ui),
            CompactView::Objective => self.objective_ui(ui),
        });
    }
}
