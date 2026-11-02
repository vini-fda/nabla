//! Orbit commands: preview edits never change an engaged target until confirmed.

use super::{RightView, Vec3, ViewerApp, input_row, palette, world};
use crate::controller_guide::{self, Icon};
use crate::formula::{self, Formula};
use crate::game::Objective;
use crate::gamepad::{Button, State};
use crate::keys;
use crate::orbit::{self, Phase, Prediction, Target};
use crate::theme::icons::outline;

#[derive(Default)]
pub(super) struct Controls {
    target: Option<Target>,
    prediction: Option<Prediction>,
    error: Option<String>,
    /// Simulated time at the last prediction; edits invalidate it even while paused.
    checked_at: Option<f64>,
}

#[derive(Default)]
pub(super) struct Forecast {
    checked_at: Option<f64>,
    starts_at: f64,
    path: Vec<[f64; 2]>,
}

impl ViewerApp {
    pub(super) fn assisted(&self) -> bool {
        self.game.assisted()
    }

    pub(super) fn reset_orbit_controls(&mut self) {
        let state = self.game.state();
        let nearest = self
            .game
            .world
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
            target: nearest.and_then(|i| Target::at_current_altitude(&self.game.world, &state, i)),
            ..Controls::default()
        };
        let objective = self
            .orbit_controls
            .target
            .map_or(Objective::ReachB, Objective::Orbit);
        self.choose_objective(objective);
    }

    pub(super) fn choose_objective(&mut self, objective: Objective) {
        match self.game.select_objective(objective) {
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
        self.prediction = Forecast::default();
    }

    fn select_orbit_body(&mut self, body: usize) {
        self.orbit_controls.target =
            Target::at_current_altitude(&self.game.world, &self.game.state(), body);
        self.invalidate_orbit_preview();
    }

    fn adjust_altitude(&mut self, amount: f64, fine: bool) {
        if let Some(target) = &mut self.orbit_controls.target {
            let step = if fine {
                0.1
            } else {
                (target.altitude * 0.1).max(1.0)
            };
            target.altitude = (target.altitude + amount * step)
                .clamp(Target::min_altitude(&self.game.world), orbit::MAX_ALTITUDE);
            self.invalidate_orbit_preview();
        }
    }

    pub(super) fn engage_objective(&mut self) {
        let objective = match self.game.objective() {
            Objective::ReachB => Objective::ReachB,
            Objective::Orbit(_) => {
                let Some(target) = self.orbit_controls.target else {
                    return;
                };
                Objective::Orbit(target)
            }
        };
        match self.game.engage(objective, self.policy) {
            Ok(prediction) => {
                self.autopilot_fault = None;
                self.aiming = None;
                self.orbit_controls.prediction = prediction;
                self.orbit_controls.error = None;
                self.orbit_controls.checked_at = Some(self.game.time());
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
                self.orbit_controls.checked_at = Some(self.game.time());
            }
        }
    }

    pub(super) fn manual_flight(&mut self) {
        self.game.disengage();
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
        let objective = self.game.objective();
        if pad.just_pressed(Button::DPadLeft) {
            self.choose_objective(Objective::ReachB);
        } else if pad.just_pressed(Button::DPadRight)
            && self.game.objective() == Objective::ReachB
            && let Some(target) = self.orbit_controls.target
        {
            self.choose_objective(Objective::Orbit(target));
        }
        if self.game.objective() != objective {
            self.right_view = RightView::Objective;
            self.notifications.post(
                format!("Objective: {}", self.game.objective().label()),
                palette::WHITE,
            );
        }
        if self.game.objective() == Objective::ReachB {
            return false;
        }
        let Some(target) = self.orbit_controls.target else {
            return false;
        };
        if pad.just_pressed(Button::RightThumb) {
            self.select_orbit_body((target.body + 1) % self.game.world.bodies.len());
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

    pub(super) fn refresh_orbit_prediction(&mut self) {
        if self.game.objective() == Objective::ReachB {
            return;
        }
        let Some(target) = self.orbit_controls.target else {
            return;
        };
        if self.orbit_controls.checked_at.is_some_and(|time| {
            self.game.time() - time < 1.0
                || self.game.orbit_pilot().is_some_and(|p| p.target == target)
        }) {
            return;
        }
        self.orbit_controls.checked_at = Some(self.game.time());
        match orbit::predict(&self.game.world, self.game.state(), target) {
            Ok(prediction) => {
                self.orbit_controls.prediction = Some(prediction);
                self.orbit_controls.error = None;
            }
            Err(error) => {
                self.orbit_controls.prediction = None;
                self.orbit_controls.error = Some(error);
            }
        }
    }

    /// The flight's objective and, for an orbit, its target, preview and engagement.
    pub(super) fn objective_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Objective");
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut objective = self.game.objective();
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
            if objective != self.game.objective() {
                self.choose_objective(objective);
            }
            match self.game.objective() {
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
                .selected_text(&self.game.world.bodies[body].name)
                .show_ui(ui, |ui| {
                    for (i, body) in self.game.world.bodies.iter().enumerate() {
                        ui.selectable_value(&mut target.body, i, &body.name);
                    }
                });
        });
        formula_row(ui, &formula::ALTITUDE, "Altitude", |ui| {
            ui.add(
                egui::DragValue::new(&mut target.altitude)
                    .range(Target::min_altitude(&self.game.world)..=orbit::MAX_ALTITUDE)
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
        if let Some(pilot) = self.game.orbit_pilot() {
            group_heading(ui, "Active orbit", palette::GREEN, false);
            icon_row(ui, outline::PLANET, "Target", None, |ui| {
                ui.label(&self.game.world.bodies[pilot.target.body].name);
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
        if self.game.objective() == Objective::ReachB {
            return;
        }
        let draw = |target: Target, color, dashed| {
            let body = &self.game.world.bodies[target.body];
            let radius = target.radius(&self.game.world) as f32;
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
        if let Some(pilot) = self.game.orbit_pilot() {
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
            .is_some_and(|t| self.game.time() - t < 0.25)
        {
            return;
        }
        let path = if self.assisted() {
            self.game.prediction()
        } else {
            match self.game.objective() {
                Objective::ReachB => Vec::new(),
                Objective::Orbit(_) => self.orbit_controls.target.map_or_else(Vec::new, |target| {
                    orbit::forecast(
                        &self.game.world,
                        self.game.next_state(),
                        orbit::Pilot::new(target),
                    )
                }),
            }
        };
        self.prediction = Forecast {
            checked_at: Some(self.game.time()),
            starts_at: self.game.replay.trajectory.controls.len() as f64 * self.game.replay.h(),
            path,
        };
    }

    pub(super) fn draw_prediction(&self, painter: &egui::Painter, rect: egui::Rect) {
        if !self.show_prediction || self.prediction.path.len() < 2 {
            return;
        }
        // Drop elapsed forecast samples so the red line always starts at the visible ship.
        let first = ((self.game.time() - self.prediction.starts_at) / self.game.replay.h())
            .max(0.0)
            .ceil() as usize;
        let state = self.game.state();
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
                thrust: true,
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
