//! Orbit commands: preview edits never change an engaged target until confirmed.

use std::time::Duration;

use log::Level;

use super::{RightView, Vec3, ViewerApp, input_row, palette, world};
use crate::controller_guide::{self, Icon};
use crate::formula::{self, Formula};
use crate::game::Objective;
use crate::gamepad::{Button, State};
use crate::keys;
use crate::orbit::{self, Check, Phase, Pilot, Prediction, Target};
use crate::theme::icons::outline;
use nabla_core::State as State2;
use nabla_sim::span;

/// How long a frame gives the preview's capture check. At once, the check makes a frame late
/// enough to lose commands to the server (`docs/multiplayer.typ`, "Autopilots").
const CHECK_BUDGET: Duration = Duration::from_millis(2);

#[derive(Default)]
pub(super) struct Controls {
    target: Option<Target>,
    prediction: Option<Prediction>,
    error: Option<String>,
    /// Simulated time at the last prediction's start; edits invalidate it even while paused.
    checked_at: Option<f64>,
    /// The check under way, which leaves the last one's verdict showing until it has its own.
    check: Option<Check>,
}

#[derive(Default)]
pub(super) struct Forecast {
    checked_at: Option<f64>,
    starts_at: f64,
    path: Vec<[f64; 2]>,
}

impl ViewerApp {
    /// The flight on a server, while that is the screen.
    fn online_flight(&self) -> Option<&crate::online::Online> {
        self.online
            .as_ref()
            .filter(|_| self.screen == super::Screen::Online)
    }

    /// What the orbit controls and the forecast work on is the flight now showing: a single-player
    /// flight's [`Game`](crate::game::Game), or a flight on a server.
    pub(super) fn flight_world(&self) -> &crate::world::World {
        self.online_flight()
            .and_then(crate::online::Online::world)
            .unwrap_or(&self.game.world)
    }

    pub(super) fn flight_state(&self) -> State2 {
        self.online_flight()
            .and_then(crate::online::Online::next_state)
            .unwrap_or_else(|| self.game.state())
    }

    pub(super) fn flight_next_state(&self) -> State2 {
        self.flight_state()
    }

    pub(super) fn flight_time(&self) -> f64 {
        self.online_flight()
            .map_or_else(|| self.game.time(), crate::online::Online::time)
    }

    /// The time the forecast starts at: of the next sample to be flown.
    pub(super) fn flight_next_time(&self) -> f64 {
        self.online_flight().map_or_else(
            || self.game.replay.trajectory.controls.len() as f64 * self.game.replay.h(),
            crate::online::Online::time,
        )
    }

    pub(super) fn flight_h(&self) -> f64 {
        self.online_flight()
            .and_then(crate::online::Online::h)
            .unwrap_or_else(|| self.game.replay.h())
    }

    pub(super) fn flight_objective(&self) -> Objective {
        self.online_flight()
            .map_or_else(|| self.game.objective(), crate::online::Online::objective)
    }

    pub(super) fn flight_orbit_pilot(&self) -> Option<&Pilot> {
        match self.online_flight() {
            Some(online) => online.orbit_pilot(),
            None => self.game.orbit_pilot(),
        }
    }

    pub(super) fn flight_forecast(&self) -> Vec<[f64; 2]> {
        self.online_flight()
            .map_or_else(|| self.game.prediction(), crate::online::Online::forecast)
    }

    fn flight_select_objective(&mut self, objective: Objective) -> Result<(), String> {
        if self.screen == super::Screen::Online
            && let Some(online) = &mut self.online
        {
            return online.select_objective(objective);
        }
        self.game.select_objective(objective)
    }

    fn flight_engage(&mut self, objective: Objective) -> Result<Option<Prediction>, String> {
        let policy = self.policy;
        if self.screen == super::Screen::Online
            && let Some(online) = &mut self.online
        {
            return online.engage(objective, policy);
        }
        self.game.engage(objective, policy)
    }

    fn flight_disengage(&mut self) {
        if self.screen == super::Screen::Online
            && let Some(online) = &mut self.online
        {
            online.disengage();
        } else {
            self.game.disengage();
        }
    }

    pub(super) fn assisted(&self) -> bool {
        if self.screen == super::Screen::Online {
            return self
                .online
                .as_ref()
                .is_some_and(crate::online::Online::assisted);
        }
        self.game.assisted()
    }

    pub(super) fn reset_orbit_controls(&mut self) {
        let state = self.flight_state();
        let nearest = self
            .flight_world()
            .bodies
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let distance = |body: &crate::world::CelestialBody| {
                    (state.x - f64::from(body.position[0]))
                        .hypot(state.y - f64::from(body.position[1]))
                        - f64::from(body.radius)
                };
                distance(a).total_cmp(&distance(b))
            })
            .map(|(i, _)| i);
        self.orbit_controls = Controls {
            target: nearest
                .and_then(|i| Target::at_current_altitude(self.flight_world(), &state, i)),
            ..Controls::default()
        };
        let objective = self
            .orbit_controls
            .target
            .map_or(Objective::ReachB, Objective::Orbit);
        self.choose_objective(objective);
    }

    pub(super) fn choose_objective(&mut self, objective: Objective) {
        match self.flight_select_objective(objective) {
            Ok(()) => {
                self.autopilot_fault = None;
                self.aiming = None;
                self.show_code = false;
                self.invalidate_orbit_preview();
            }
            Err(error) => self.notifications.post(error, palette::YELLOW),
        }
    }

    fn invalidate_orbit_preview(&mut self) {
        self.orbit_controls.prediction = None;
        self.orbit_controls.error = None;
        self.orbit_controls.checked_at = None;
        self.orbit_controls.check = None;
        self.prediction = Forecast::default();
    }

    fn select_orbit_body(&mut self, body: usize) {
        self.orbit_controls.target =
            Target::at_current_altitude(self.flight_world(), &self.flight_state(), body);
        self.invalidate_orbit_preview();
    }

    fn adjust_altitude(&mut self, amount: f64, fine: bool) {
        let lowest = Target::min_altitude(self.flight_world());
        if let Some(target) = &mut self.orbit_controls.target {
            let step = if fine {
                0.1
            } else {
                (target.altitude * 0.1).max(1.0)
            };
            target.altitude = (target.altitude + amount * step).clamp(lowest, orbit::MAX_ALTITUDE);
            self.invalidate_orbit_preview();
        }
    }

    pub(super) fn engage_objective(&mut self) {
        let objective = match self.flight_objective() {
            Objective::ReachB => Objective::ReachB,
            Objective::Orbit(_) => {
                let Some(target) = self.orbit_controls.target else {
                    return;
                };
                Objective::Orbit(target)
            }
        };
        self.orbit_controls.check = None;
        match self.flight_engage(objective) {
            Ok(prediction) => {
                self.autopilot_fault = None;
                self.aiming = None;
                self.orbit_controls.prediction = prediction;
                self.orbit_controls.error = None;
                self.orbit_controls.checked_at = Some(self.flight_time());
                self.prediction = Forecast::default();
                self.notifications.post(
                    format!("Autopilot engaged: {}", objective.label()),
                    palette::SAPPHIRE,
                );
            }
            Err(error) => {
                self.notifications.post(&error, palette::YELLOW);
                self.orbit_controls.error = Some(error);
                self.orbit_controls.prediction = None;
                self.orbit_controls.checked_at = Some(self.flight_time());
            }
        }
    }

    pub(super) fn manual_flight(&mut self) {
        self.flight_disengage();
        self.autopilot_fault = None;
        self.notifications.post("Manual flight", palette::WHITE);
        self.invalidate_orbit_preview();
    }

    /// Returns whether Square was consumed. Left/right selects the objective, and says so: the
    /// panel with its details may be closed or hidden. Circle always wins.
    pub(super) fn orbit_buttons(&mut self, pad: &State) -> bool {
        if pad.just_pressed(Button::East) {
            self.manual_flight();
            return true; // Cancel wins over a simultaneous engage.
        }
        let objective = self.flight_objective();
        if pad.just_pressed(Button::DPadLeft) {
            self.choose_objective(Objective::ReachB);
        } else if pad.just_pressed(Button::DPadRight)
            && self.flight_objective() == Objective::ReachB
            && let Some(target) = self.orbit_controls.target
        {
            self.choose_objective(Objective::Orbit(target));
        }
        if self.flight_objective() != objective {
            self.right_view = RightView::Objective;
            self.notifications.post(
                format!("Objective: {}", self.flight_objective().label()),
                palette::WHITE,
            );
        }
        if self.flight_objective() == Objective::ReachB {
            return false;
        }
        let Some(target) = self.orbit_controls.target else {
            return false;
        };
        if pad.just_pressed(Button::RightThumb) {
            self.select_orbit_body((target.body + 1) % self.flight_world().bodies.len());
        }
        let delta = i32::from(pad.just_pressed(Button::DPadUp))
            - i32::from(pad.just_pressed(Button::DPadDown));
        if delta != 0 {
            self.adjust_altitude(f64::from(delta), pad.is_held(Button::LeftTrigger2));
        }
        if pad.just_pressed(Button::West) {
            self.engage_objective();
        }
        true
    }

    /// Start a capture check of the selected orbit when one is due, and give the one under way
    /// this frame's share of time.
    pub(super) fn refresh_orbit_prediction(&mut self) {
        let target = self
            .orbit_controls
            .target
            .filter(|_| self.flight_objective() != Objective::ReachB);
        let Some(target) = target else {
            self.orbit_controls.check = None;
            return;
        };
        if self.orbit_controls.check.is_none() {
            if self.orbit_controls.checked_at.is_some_and(|time| {
                self.flight_time() - time < 1.0
                    || self
                        .flight_orbit_pilot()
                        .is_some_and(|p| p.target == target)
            }) {
                return;
            }
            self.orbit_controls.checked_at = Some(self.flight_time());
            match Check::new(self.flight_world(), self.flight_state(), target) {
                Ok(check) => self.orbit_controls.check = Some(check),
                Err(error) => {
                    self.orbit_controls.prediction = None;
                    self.orbit_controls.error = Some(error);
                    return;
                }
            }
        }
        let verdict = self
            .orbit_controls
            .check
            .as_mut()
            .and_then(|check| check.run_for(CHECK_BUDGET));
        match verdict {
            Some(Ok(prediction)) => {
                self.orbit_controls.check = None;
                self.orbit_controls.prediction = Some(prediction);
                self.orbit_controls.error = None;
            }
            Some(Err(error)) => {
                self.orbit_controls.check = None;
                self.orbit_controls.prediction = None;
                self.orbit_controls.error = Some(error);
            }
            None => {}
        }
    }

    /// Whether a capture check needs more frames, which a paused flight would not otherwise draw.
    pub(super) fn orbit_check_running(&self) -> bool {
        self.orbit_controls.check.is_some()
    }

    /// The flight's objective and, for an orbit, its target, preview and engagement.
    pub(super) fn objective_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Objective");
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut objective = self.flight_objective();
            egui::ComboBox::from_id_salt("flight-objective")
                .selected_text(objective.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut objective, Objective::ReachB, "Reach B");
                    if let Some(target) = self.orbit_controls.target {
                        let selected = matches!(objective, Objective::Orbit(_));
                        if ui.selectable_label(selected, "Orbit").clicked() && !selected {
                            objective = Objective::Orbit(target);
                        }
                    }
                });
            if objective != self.flight_objective() {
                self.choose_objective(objective);
            }
            match self.flight_objective() {
                Objective::ReachB => {
                    ui.label("Reach B and stop.");
                    controls_grid(ui, |ui| {
                        objective_control_row(ui);
                        input_row(
                            ui,
                            |ui| {
                                controller_guide::icon_ui(ui, &controller_guide::SQUARE);
                                if !keys::Keys::new(ui.ctx()).touch() {
                                    keys::joiner(ui, "/");
                                    keys::caps(ui, &[keys::A]);
                                }
                            },
                            |ui| ui.small("Autopilot"),
                        );
                        control_row(ui, &[&controller_guide::CIRCLE], "Manual");
                    });
                }
                Objective::Orbit(_) => self.orbit_ui(ui),
            }
        });
    }

    fn orbit_ui(&mut self, ui: &mut egui::Ui) {
        let Some(mut target) = self.orbit_controls.target else {
            return;
        };
        let body = target.body;
        let mut engage = false;
        let mut manual = false;
        group_heading(ui, "Selected orbit", self.selected_orbit_color(), true);
        icon_row(ui, outline::PLANET, "Target", None, |ui| {
            egui::ComboBox::from_id_salt("orbit-body")
                .width(100.0)
                .selected_text(&self.flight_world().bodies[body].name)
                .show_ui(ui, |ui| {
                    for (i, body) in self.flight_world().bodies.iter().enumerate() {
                        ui.selectable_value(&mut target.body, i, &body.name);
                    }
                });
        });
        formula_row(ui, &formula::ALTITUDE, "Altitude", |ui| {
            ui.add(
                egui::DragValue::new(&mut target.altitude)
                    .range(Target::min_altitude(self.flight_world())..=orbit::MAX_ALTITUDE)
                    .speed(0.1)
                    .max_decimals(1)
                    .suffix(" m"),
            );
        });
        direction_row(ui, target);
        if let Some(error) = &self.orbit_controls.error {
            icon_row(
                ui,
                outline::WARNING,
                "Preview",
                Some(palette::YELLOW),
                |ui| {
                    let text = egui::RichText::new(error).color(palette::YELLOW);
                    ui.add(egui::Label::new(text).wrap());
                },
            );
        } else if let Some(prediction) = &self.orbit_controls.prediction {
            let capture = prediction.capture_seconds;
            formula_row(ui, &formula::CAPTURE_TIME, "Predicted capture time", |ui| {
                ui.label(format!("{capture:.0} s"));
            });
        } else {
            icon_row(ui, outline::HOURGLASS_MEDIUM, "Preview", None, |ui| {
                ui.label("Checking capture...");
            });
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let square = controller_guide::SQUARE.inline(ui);
            engage = ui.button(("Engage orbit", square)).clicked();
            let circle = controller_guide::CIRCLE.inline(ui);
            manual = ui
                .add_enabled(self.assisted(), egui::Button::new(("Manual", circle)))
                .clicked();
        });
        if target.body != body {
            self.select_orbit_body(target.body);
        } else if self.orbit_controls.target != Some(target) {
            self.orbit_controls.target = Some(target);
            self.invalidate_orbit_preview();
        }
        if manual {
            self.manual_flight();
        } else if engage {
            self.engage_objective();
        }
        if let Some(pilot) = self.flight_orbit_pilot() {
            group_heading(ui, "Active orbit", palette::GREEN, false);
            icon_row(ui, outline::PLANET, "Target", None, |ui| {
                ui.label(&self.flight_world().bodies[pilot.target.body].name);
            });
            formula_row(ui, &formula::ALTITUDE, "Altitude", |ui| {
                ui.label(format!("{:.1} m", pilot.target.altitude));
            });
            direction_row(ui, pilot.target);
            let color = (pilot.phase == Phase::Holding).then_some(palette::GREEN);
            icon_row(ui, phase_icon(pilot.phase), "Phase", color, |ui| {
                let mut text = egui::RichText::new(pilot.phase.label());
                if let Some(color) = color {
                    text = text.color(color);
                }
                ui.label(text);
            });
        }
        controls_grid(ui, |ui| {
            objective_control_row(ui);
            control_row(
                ui,
                &[&controller_guide::DPAD_UP, &controller_guide::DPAD_DOWN],
                "Altitude",
            );
            control_row(ui, &[&controller_guide::L2], "Hold for fine altitude");
            control_row(ui, &[&controller_guide::RIGHT_STICK_CLICK], "Next target");
        });
    }

    /// The color of the selected orbit's dashed ring: a warning while it can't be captured.
    fn selected_orbit_color(&self) -> egui::Color32 {
        if self.orbit_controls.error.is_some() {
            palette::YELLOW
        } else {
            palette::SKY
        }
    }

    pub(super) fn draw_orbits(&self, painter: &egui::Painter, rect: egui::Rect) {
        if self.flight_objective() == Objective::ReachB {
            return;
        }
        let draw = |target: Target, color, dashed| {
            let body = &self.flight_world().bodies[target.body];
            let radius = target.radius(self.flight_world()) as f32;
            let center = egui::Vec2::from(body.position);
            for i in 0..128 {
                if dashed && i % 4 >= 2 {
                    continue;
                }
                let points = [i, i + 1].map(|j| {
                    let angle = j as f32 * std::f32::consts::TAU / 128.0;
                    Vec3::plane(center + egui::Vec2::angled(angle) * radius)
                });
                for run in self.camera.project_polyline(points, rect) {
                    painter.add(egui::Shape::line(run, egui::Stroke::new(1.5, color)));
                }
            }
        };
        if let Some(pilot) = self.flight_orbit_pilot() {
            draw(pilot.target, palette::GREEN, false);
        }
        if let Some(target) = self.orbit_controls.target {
            draw(target, self.selected_orbit_color(), true);
        }
    }

    pub(super) fn refresh_prediction(&mut self) {
        if !self.show_prediction {
            self.prediction = Forecast::default();
            return;
        }
        if self
            .prediction
            .checked_at
            .is_some_and(|t| self.flight_time() - t < 0.25)
        {
            return;
        }
        let _span = span!(Level::Trace, "forecast");
        let path = if self.assisted() {
            self.flight_forecast()
        } else {
            match self.flight_objective() {
                Objective::ReachB => Vec::new(),
                Objective::Orbit(_) => self.orbit_controls.target.map_or_else(Vec::new, |target| {
                    orbit::forecast(
                        self.flight_world(),
                        self.flight_next_state(),
                        orbit::Pilot::new(target),
                    )
                }),
            }
        };
        self.prediction = Forecast {
            checked_at: Some(self.flight_time()),
            starts_at: self.flight_next_time(),
            path,
        };
    }

    pub(super) fn draw_prediction(&self, painter: &egui::Painter, rect: egui::Rect) {
        if !self.show_prediction || self.prediction.path.len() < 2 {
            return;
        }
        // Drop elapsed forecast samples so the red line always starts at the visible ship.
        let first = ((self.flight_time() - self.prediction.starts_at) / self.flight_h())
            .max(0.0)
            .ceil() as usize;
        let state = self.flight_state();
        let points = std::iter::once([state.x, state.y])
            .chain(self.prediction.path.iter().skip(first).copied())
            .map(|p| Vec3::plane(world(p[0], p[1])));
        for run in self.camera.project_polyline(points, rect) {
            painter.add(egui::Shape::line(run, egui::Stroke::new(2.0, palette::RED)));
        }
    }
}

/// Width of the column of icons and symbols in the rows of the objective view, and of the sample
/// of an orbit's ring in the heading of its group, with the size of the icons.
const SYMBOL_COLUMN_WIDTH: f32 = 28.0;
const ICON_SIZE: f32 = 16.0;

/// The heading of a group of the objective view: a sample of its orbit's ring on the canvas, in
/// `color` and maybe `dashed`, then its name.
fn group_heading(ui: &mut egui::Ui, name: &str, color: egui::Color32, dashed: bool) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let size = egui::vec2(SYMBOL_COLUMN_WIDTH, ui.spacing().interact_size.y);
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let ends = [
            rect.left_center(),
            rect.right_center() - egui::vec2(6.0, 0.0),
        ];
        let stroke = egui::Stroke::new(1.5, color);
        if dashed {
            ui.painter()
                .add(egui::Shape::dashed_line(&ends, stroke, 4.0, 3.0));
        } else {
            ui.painter().line_segment(ends, stroke);
        }
        ui.label(group_name(name));
    });
}

/// The name of a group of the objective view.
fn group_name(name: &str) -> egui::RichText {
    egui::RichText::new(name).small().color(palette::OVERLAY1)
}

/// A row of the objective view: `symbol`, captioned by `caption` when hovered, then `value`.
fn symbol_row(
    ui: &mut egui::Ui,
    symbol: impl FnOnce(&mut egui::Ui) -> egui::Response,
    caption: &str,
    value: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        let size = egui::vec2(SYMBOL_COLUMN_WIDTH, ui.spacing().interact_size.y);
        let layout = egui::Layout::left_to_right(egui::Align::Center);
        ui.allocate_ui_with_layout(size, layout, |ui| {
            ui.set_min_size(size);
            symbol(ui).on_hover_text(caption);
        });
        value(ui);
    });
}

/// A [`symbol_row`] with a Phosphor icon, in `color` if given.
fn icon_row(
    ui: &mut egui::Ui,
    icon: &str,
    caption: &str,
    color: Option<egui::Color32>,
    value: impl FnOnce(&mut egui::Ui),
) {
    let symbol = |ui: &mut egui::Ui| {
        let mut text = egui::RichText::new(icon).font(outline::font(ICON_SIZE));
        if let Some(color) = color {
            text = text.color(color);
        }
        ui.add(egui::Label::new(text).selectable(false))
    };
    symbol_row(ui, symbol, caption, value);
}

/// A [`symbol_row`] with a typeset symbol.
fn formula_row(
    ui: &mut egui::Ui,
    formula: &Formula,
    caption: &str,
    value: impl FnOnce(&mut egui::Ui),
) {
    let symbol = |ui: &mut egui::Ui| {
        let font = egui::TextStyle::Body.resolve(ui.style());
        formula.ui(ui, &font)
    };
    symbol_row(ui, symbol, caption, value);
}

/// The row with the direction `target` is orbited in.
fn direction_row(ui: &mut egui::Ui, target: Target) {
    let icon = if target.clockwise {
        outline::ARROW_CLOCKWISE
    } else {
        outline::ARROW_COUNTER_CLOCKWISE
    };
    icon_row(ui, icon, "Direction", None, |ui| {
        ui.label(target.direction());
    });
}

/// The icon of what the orbit assist is doing.
fn phase_icon(phase: Phase) -> &'static str {
    match phase {
        Phase::Turning => outline::COMPASS,
        Phase::Burning => outline::FIRE,
        Phase::Coasting => outline::ROCKET,
        Phase::Holding => outline::CHECK_CIRCLE,
    }
}

/// The objective view's list of controls: a column of icons, then what they do.
fn controls_grid(ui: &mut egui::Ui, rows: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(8.0);
    ui.label(group_name("Controls"));
    egui::Grid::new("objective-controls")
        .num_columns(2)
        .show(ui, rows);
}

/// A row of a [`controls_grid`].
fn control_row(ui: &mut egui::Ui, icons: &[&Icon], action: &str) {
    input_row(
        ui,
        |ui| {
            for icon in icons {
                controller_guide::icon_ui(ui, icon);
            }
        },
        |ui| ui.small(action),
    );
}

/// The row of a [`controls_grid`] that both objectives share.
fn objective_control_row(ui: &mut egui::Ui) {
    control_row(
        ui,
        &[&controller_guide::DPAD_LEFT, &controller_guide::DPAD_RIGHT],
        "Objective",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Game, Input, Steering};
    use crate::world::Preset;

    fn planet_app() -> ViewerApp {
        let world = Preset::Planet.world();
        let mut app = ViewerApp {
            game: Game::in_world(world.case(), world),
            ..ViewerApp::default()
        };
        app.reset_orbit_controls();
        app
    }

    fn press(app: &mut ViewerApp, buttons: &[Button], fine: bool) -> bool {
        app.orbit_buttons(&State {
            pressed: buttons.to_vec(),
            held: if fine {
                vec![Button::LeftTrigger2]
            } else {
                vec![]
            },
            ..State::default()
        })
    }

    /// Frames until the capture check under way has its verdict.
    fn finish_check(app: &mut ViewerApp) {
        while app.orbit_check_running() {
            app.refresh_orbit_prediction();
        }
    }

    #[test]
    fn the_preview_checks_a_capture_over_several_frames() {
        let mut app = planet_app();
        let target = app.orbit_controls.target.expect("preview");
        let whole = orbit::predict(&app.game.world, app.game.state(), target).expect("a capture");

        // 6000 steps can't fit in one frame's share, however fast the machine.
        app.refresh_orbit_prediction();
        assert!(app.orbit_check_running());
        assert!(app.orbit_controls.prediction.is_none() && app.orbit_controls.error.is_none());
        finish_check(&mut app);
        let first = app.orbit_controls.prediction.as_ref().expect("a verdict");
        assert_eq!(first.capture_seconds, whole.capture_seconds);

        // The next check, a second on, leaves that verdict showing while it runs.
        app.refresh_orbit_prediction();
        assert!(!app.orbit_check_running(), "not due yet");
        for _ in 0..11 {
            app.game.advance(0.1, Input::default(), Steering::Raw);
        }
        app.refresh_orbit_prediction();
        assert!(app.orbit_check_running());
        assert!(app.orbit_controls.prediction.is_some());
        finish_check(&mut app);

        // An edit abandons the check for the old target, and its verdict with it.
        for _ in 0..11 {
            app.game.advance(0.1, Input::default(), Steering::Raw);
        }
        app.refresh_orbit_prediction();
        assert!(app.orbit_check_running());
        press(&mut app, &[Button::DPadUp], false);
        assert!(!app.orbit_check_running());
        assert!(app.orbit_controls.prediction.is_none());
        app.refresh_orbit_prediction();
        assert!(app.orbit_check_running(), "a new one, for the new target");

        // So does engaging, which checks for itself. Its verdict is the one left showing.
        press(&mut app, &[Button::West], false);
        assert!(app.assisted());
        assert!(!app.orbit_check_running());
        app.refresh_orbit_prediction();
        assert!(!app.orbit_check_running(), "the assist flies this target");
        assert!(app.orbit_controls.prediction.is_some());

        // And choosing B.
        app.manual_flight();
        app.refresh_orbit_prediction();
        assert!(app.orbit_check_running());
        app.choose_objective(Objective::ReachB);
        assert!(!app.orbit_check_running());
    }

    #[test]
    fn the_preview_refuses_a_target_at_once() {
        let mut app = planet_app();
        app.orbit_controls
            .target
            .as_mut()
            .expect("preview")
            .altitude = f64::NAN;
        app.refresh_orbit_prediction();
        assert!(!app.orbit_check_running());
        assert!(app.orbit_controls.error.is_some());
    }

    #[test]
    fn preview_edits_wait_for_square_and_circle_returns_to_manual() {
        let mut app = planet_app();
        app.game.paused = true;
        assert!(press(&mut app, &[Button::West], false));
        let original = app.game.orbit_pilot().expect("engaged while paused").target;
        press(&mut app, &[Button::DPadUp], true);
        let selected = app.orbit_controls.target.expect("preview");
        assert!((selected.altitude - original.altitude - 0.1).abs() < 1e-9);
        assert_eq!(
            app.game.orbit_pilot().expect("still engaged").target,
            original
        );
        press(&mut app, &[Button::West], false);
        assert_eq!(app.game.orbit_pilot().expect("confirmed").target, selected);
        let before = app.game.state();
        app.game.advance(0.1, Input::default(), Steering::Raw);
        assert_eq!(app.game.state(), before, "engaging does not unpause");
        press(&mut app, &[Button::East, Button::West], false);
        assert!(!app.assisted(), "cancel wins over a simultaneous engage");
        app.game.paused = false;
        app.game.advance(
            0.1,
            Input {
                thrust: 1.0,
                ..Input::default()
            },
            Steering::Raw,
        );
        assert_eq!(
            app.game.replay.trajectory.controls[0].thrust,
            app.game.world.players[0].task.params.f_max,
            "manual thrust resumes"
        );
    }

    #[test]
    fn rejected_request_keeps_active_orbit_and_goal_autopilot_can_take_over() {
        let mut app = planet_app();
        press(&mut app, &[Button::West], false);
        let active = app.game.orbit_pilot().expect("engaged").target;
        app.orbit_controls
            .target
            .as_mut()
            .expect("preview")
            .altitude = f64::NAN;
        press(&mut app, &[Button::West], false);
        assert!(app.orbit_controls.error.is_some());
        assert_eq!(app.game.orbit_pilot().expect("kept active").target, active);
        app.choose_objective(Objective::ReachB);
        app.toggle_autopilot();
        assert!(app.assisted() && app.game.objective() == Objective::ReachB);
        press(&mut app, &[Button::East], false);
        assert!(!app.assisted());
    }

    #[test]
    fn target_cycles_and_restart_and_vacuum_clear_orbit_state() {
        let mut app = planet_app();
        let world = Preset::PlanetAndMoon.world();
        app.game = Game::in_world(world.case(), world);
        app.reset_orbit_controls();
        assert_eq!(app.orbit_controls.target.expect("nearest").body, 0);
        press(&mut app, &[Button::RightThumb], false);
        assert_eq!(app.orbit_controls.target.expect("next").body, 1);
        press(&mut app, &[Button::RightThumb], false);
        assert_eq!(app.orbit_controls.target.expect("wrapped").body, 0);
        // Restart creates a fresh flight and selects the nearest body again.
        app.world_setup = Preset::Planet.world();
        app.start_game();
        assert!(!app.assisted());
        assert_eq!(app.orbit_controls.target.expect("reset").body, 0);
        app.world_setup = Preset::Vacuum.world();
        app.start_game();
        assert!(app.orbit_controls.target.is_none());
        assert!(
            !press(&mut app, &[Button::West], false),
            "vacuum delegates Square"
        );
    }

    /// The height of the game's bar, drawn on its own at a width that fits it on one row.
    fn game_bar_height(app: &mut ViewerApp) -> f32 {
        let ctx = egui::Context::default();
        let mut height = 0.0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            height = ui
                .scope(|ui| app.game_bar_ui(ui, false))
                .response
                .rect
                .height();
        });
        output.textures_delta.clear();
        height
    }

    #[test]
    fn switching_objectives_keeps_the_game_bar_height() {
        let mut app = planet_app();
        let orbit = game_bar_height(&mut app);
        press(&mut app, &[Button::DPadLeft], false);
        assert_eq!(app.game.objective(), Objective::ReachB);
        assert_eq!(game_bar_height(&mut app), orbit);
    }

    #[test]
    fn dpad_objective_switch_brings_up_the_objective_view() {
        let mut app = planet_app();
        assert_eq!(app.right_view, RightView::Plots);
        press(&mut app, &[Button::DPadUp], false);
        assert_eq!(app.right_view, RightView::Plots, "no change, no switch");
        press(&mut app, &[Button::DPadLeft], false);
        assert_eq!(app.right_view, RightView::Objective);
    }

    #[test]
    fn objective_selection_and_manual_control_preserve_exclusivity() {
        let mut app = planet_app();
        press(&mut app, &[Button::West], false);
        let objective = app.game.objective();
        press(&mut app, &[Button::East], false);
        assert_eq!(
            app.game.objective(),
            objective,
            "manual keeps the orbit objective"
        );
        assert!(!app.assisted());
        press(&mut app, &[Button::DPadLeft], false);
        assert_eq!(app.game.objective(), Objective::ReachB);
        app.toggle_autopilot();
        assert!(app.assisted());
        press(&mut app, &[Button::DPadRight], false);
        assert!(matches!(app.game.objective(), Objective::Orbit(_)));
        assert!(!app.assisted(), "a new objective waits for engagement");
        app.toggle_autopilot();
        assert!(
            app.game.orbit_pilot().is_some(),
            "A engages the selected objective too"
        );
    }

    #[test]
    fn forecast_refreshes_while_engaged_and_display_toggle_does_not_change_flight() {
        let mut app = planet_app();
        press(&mut app, &[Button::West], false);
        app.refresh_prediction();
        let initial = app.prediction.path.clone();
        assert!(initial.len() > 2 && initial.len() <= 501);
        for _ in 0..4 {
            app.game.advance(0.1, Input::default(), Steering::Raw);
        }
        app.refresh_prediction();
        assert_ne!(
            app.prediction.path, initial,
            "live forecast refreshes after engagement"
        );
        let objective = app.game.objective();
        let state = app.game.state();
        app.show_prediction = false;
        app.refresh_prediction();
        assert!(app.prediction.path.is_empty());
        assert_eq!(app.game.objective(), objective);
        assert_eq!(app.game.state(), state);
        assert!(app.assisted());
        app.show_prediction = true;
        app.refresh_prediction();
        assert!(
            !app.prediction.path.is_empty(),
            "re-enabled while paused or at the same time"
        );
        app.choose_objective(Objective::ReachB);
        app.refresh_prediction();
        assert!(
            app.prediction.path.is_empty(),
            "no stale orbit forecast for B"
        );
    }
}
