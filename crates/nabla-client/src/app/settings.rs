//! The settings panel: the world, the initial conditions, the autopilot, the current state and
//! the display options, and the window with the policy's source.

use nabla_core::problem::Outcome;
use nabla_core::solutions::SOLUTIONS;
use nabla_core::suite;

use super::{CanvasView, ViewerApp, orbits};
use crate::camera::Camera;
use crate::formula::{self, Formula};
use crate::game::Objective;
use crate::keys;
use crate::theme::palette;

/// Range of sampling periods offered by the slider (the suites use `suite::H`).
const H_MIN: f64 = 0.01;
const H_MAX: f64 = 0.1;

impl ViewerApp {
    /// The left panel's content: case, autopilot, current state and display options.
    pub(super) fn controls_ui(&mut self, ui: &mut egui::Ui) {
        // Scrolls sideways too, so rows are never cut off if the space is narrower than they are.
        egui::ScrollArea::both().show(ui, |ui| {
            self.world_selector_ui(ui);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let font = egui::TextStyle::Body.resolve(ui.style());
                formula::G
                    .ui(ui, &font)
                    .on_hover_text("gravitational constant");
                ui.label(format!(" = {}", self.game.world.gravity_constant));
            });
            self.world_files_ui(ui);
            ui.separator();
            ui.heading("Initial conditions");
            self.case_info_ui(ui);
            ui.separator();
            self.autopilot_ui(ui);
            ui.separator();
            self.frame_info_ui(ui);
            ui.separator();
            self.display_ui(ui, true);
            ui.separator();
            self.sound.ui(ui);
        });
    }

    /// The autopilot: the policy it flies with, its switch, its source, and its sampling period.
    fn autopilot_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Autopilot");
        if self.game.objective() == Objective::ReachTarget {
            self.policy_ui(ui);
        } else {
            ui.label("Circular-orbit capture and hold");
        }
        ui.horizontal(|ui| {
            let mut engaged = self.assisted();
            let label = if engaged { "Engaged" } else { "Engage" };
            let toggle = ui.toggle_value(&mut engaged, label).on_hover_ui(|ui| {
                keys::tooltip(
                    ui,
                    "Let the policy fly the ship, from wherever it is. Switch it off to take over",
                    &[keys::A],
                );
            });
            if toggle.changed() {
                self.toggle_autopilot();
            }
            if self.game.objective() == Objective::ReachTarget {
                ui.checkbox(&mut self.show_code, "Show code");
            }
        });
        if let Some(fault) = &self.autopilot_fault {
            ui.colored_label(palette::RED, format!("Switched off: {fault}"));
        }
        let h = ui
            .horizontal(|ui| {
                let font = egui::TextStyle::Body.resolve(ui.style());
                formula::H.ui(ui, &font).on_hover_text("sampling period");
                ui.add(egui::Slider::new(&mut self.h, H_MIN..=H_MAX).suffix(" s"))
            })
            .inner;
        if h.double_clicked() {
            self.h = suite::H;
        }
        h.on_hover_text("Sampling period. Double-click to reset. Changing it restarts the flight.");
        if self.game.replay.h() != self.h {
            self.score_policy();
            self.start_game();
        }
    }

    /// The policy the autopilot flies with, to pick.
    pub(super) fn policy_ui(&mut self, ui: &mut egui::Ui) {
        let mut picked = None;
        ui.horizontal(|ui| {
            ui.label("Policy:");
            egui::ComboBox::from_id_salt("policy")
                .selected_text(
                    egui::RichText::new(format!("{}.rs", self.policy.name))
                        .monospace()
                        .strong(),
                )
                .show_ui(ui, |ui| {
                    for solution in SOLUTIONS {
                        let label = format!("{}.rs", solution.name);
                        let selected = std::ptr::eq(solution, self.policy);
                        if ui
                            .selectable_label(selected, egui::RichText::new(label).monospace())
                            .clicked()
                        {
                            picked = Some(solution);
                        }
                    }
                });
        });
        if let Some(solution) = picked {
            self.policy = solution;
            self.score_policy();
            if self.assisted() && self.game.objective() == Objective::ReachTarget {
                self.engage_objective();
            }
        }
    }

    /// Optional window showing the policy's source with syntax highlighting and line numbers.
    pub(super) fn code_window(&mut self, ctx: &egui::Context) {
        let solution = self.policy;
        let mut open = self.show_code;
        egui::Window::new(egui::RichText::new(format!("{}.rs", solution.name)).monospace())
            .id(egui::Id::unique("source_code_window"))
            .open(&mut open)
            .default_size([640.0, 720.0])
            .show(ctx, |ui| {
                egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        let lines = solution.source.lines().count().max(1);
                        let numbers: Vec<String> = (1..=lines).map(|n| n.to_string()).collect();
                        let numbers = egui::RichText::new(numbers.join("\n"))
                            .monospace()
                            .color(ui.visuals().weak_text_color());
                        ui.add(egui::Label::new(numbers).selectable(false));
                        crate::highlight::code_view_ui(ui, solution.source);
                    });
                });
            });
        self.show_code = open;
    }

    /// The flight's case: the ship, its start and its goal, and how the policy does on it.
    fn case_info_ui(&self, ui: &mut egui::Ui) {
        let case = &self.game.replay.case;
        let task = &case.task;
        let p = &task.params;
        let s = &case.start;
        egui::Grid::new("case_info").num_columns(2).show(ui, |ui| {
            let font = egui::TextStyle::Monospace.resolve(ui.style());
            let mut row = |formula: &Formula, name: &str, value: String| {
                formula.ui(ui, &font).on_hover_text(name);
                ui.monospace(value);
                ui.end_row();
            };
            row(&formula::MASS, "mass", format!("{:.3} kg", p.mass));
            row(
                &formula::INERTIA,
                "moment of inertia",
                format!("{:.3} kg m^2", p.inertia),
            );
            row(
                &formula::F_MAX,
                "maximum thrust",
                format!("{:.3} N", p.f_max),
            );
            row(
                &formula::TAU_MAX,
                "maximum torque",
                format!("{:.3} N m", p.tau_max),
            );
            row(
                &formula::INITIAL_POSITION,
                "initial position",
                format!("({:.2}, {:.2}) m", s.x, s.y),
            );
            row(
                &formula::INITIAL_VELOCITY,
                "initial velocity",
                format!("({:.2}, {:.2}) m/s", s.vx, s.vy),
            );
            row(
                &formula::INITIAL_THETA,
                "initial heading angle",
                format!("{:.2} rad", s.theta),
            );
            row(
                &formula::INITIAL_OMEGA,
                "initial angular velocity",
                format!("{:.2} rad/s", s.omega),
            );
            if self.game.objective() == Objective::ReachTarget {
                row(
                    &formula::GOAL,
                    "target",
                    format!("({:.2}, {:.2}) m", task.goal.x, task.goal.y),
                );
            }
        });
        if self.game.objective() != Objective::ReachTarget {
            return;
        }
        let name = self.policy.name;
        let outcome = match self.policy_outcome {
            Outcome::Reached(n) => {
                format!("{name} lands in {n} steps ({:.2} s)", n as f64 * p.h)
            }
            Outcome::Timeout => format!("{name} fails: step budget exhausted"),
            Outcome::Inadmissible { step, .. } => {
                format!("{name} fails: inadmissible control at step {step}")
            }
            Outcome::Panic { step } => format!("{name} fails: panic at step {step}"),
        };
        ui.label(egui::RichText::new(outcome).strong())
            .on_hover_text("The autopilot's policy, flying this case from its start");
    }

    fn frame_info_ui(&self, ui: &mut egui::Ui) {
        let (replay, k, offset) = self.shown();
        let s = replay.state_at(k, offset);
        let goal = replay.case.task.goal;
        ui.heading("Current state");
        let font = egui::TextStyle::Body.resolve(ui.style());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            formula::T.ui(ui, &font).on_hover_text("time");
            ui.label(format!(" = {:.4} s", replay.time(k) + offset));
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label("sample ");
            formula::K.ui(ui, &font).on_hover_text("sample index");
            ui.label(format!(" = {k} + {offset:.4} s"));
        });
        egui::Grid::new("frame_info").num_columns(2).show(ui, |ui| {
            let font = egui::TextStyle::Monospace.resolve(ui.style());
            let mut row = |formula: &Formula, name: &str, value: String| {
                formula.ui(ui, &font).on_hover_text(name);
                ui.monospace(value);
                ui.end_row();
            };
            // The `+` in the formats keeps each number's width when its sign changes, so the
            // values hold still across frames.
            row(
                &formula::POSITION,
                "position",
                format!("({:+.3}, {:+.3}) m", s.x, s.y),
            );
            row(
                &formula::VELOCITY,
                "velocity",
                format!("({:+.3}, {:+.3}) m/s", s.vx, s.vy),
            );
            row(
                &formula::THETA,
                "heading angle",
                format!("{:+.3} rad", s.theta),
            );
            row(
                &formula::OMEGA,
                "angular velocity",
                format!("{:+.3} rad/s", s.omega),
            );
            if self.game.objective() == Objective::ReachTarget {
                row(
                    &formula::DISTANCE,
                    "distance to the target",
                    format!("{:.4} m", (s.x - goal.x).hypot(s.y - goal.y)),
                );
            }
            row(
                &formula::SPEED,
                "speed",
                format!("{:.4} m/s", s.vx.hypot(s.vy)),
            );
            let control = replay.control(k).map_or_else(
                || "- (final state)".to_owned(),
                |u| format!("({:+.3} N, {:+.3} N m)", u.thrust, u.torque),
            );
            row(&formula::CONTROL, "control: thrust and torque", control);
        });
    }

    /// The display options, and `in_flight`, the buttons that fit or reset the camera. The
    /// options are the same in a single-player flight and on a server; what the server decides,
    /// the ship's size, shows but is not configurable there.
    pub(super) fn display_ui(&mut self, ui: &mut egui::Ui, in_flight: bool) {
        let online = self.online_flight().is_some();
        ui.heading("Display options");
        ui.add_enabled_ui(self.ship3d.is_some(), |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.view, CanvasView::Ship3d, "3D");
                ui.selectable_value(&mut self.view, CanvasView::Debug, "Debug")
                    .on_hover_text("Flat ship with velocity and acceleration arrows");
            });
        });
        let mut size = if online {
            self.flight_world().hull_radius()
        } else {
            self.circumradius
        };
        ui.add_enabled(
            !in_flight && !online,
            egui::Slider::new(&mut size, 0.05..=10.0)
                .logarithmic(true)
                .text("ship size (world units)"),
        )
        .on_hover_text(if online {
            "Ship size, which its collision hull scales with; the server decides it"
        } else {
            "Ship size, which its collision hull scales with; set before starting a flight"
        });
        if !online {
            self.circumradius = size;
        }
        ui.checkbox(&mut self.show_grid, "Grid");
        if self.flight_world().gravity_constant > 0.0 {
            ui.checkbox(&mut self.show_field, "Gravity field")
                .on_hover_text(
                    "Bands and contour lines of the celestial bodies' gravitational potential",
                );
            if self.show_field {
                ui.add(
                    egui::Slider::new(&mut self.field_line_width, 0.5..=4.0)
                        .text("contour line width"),
                )
                .on_hover_text("Thickness of the contour lines, in points");
            }
            ui.checkbox(&mut self.show_streamlines, "Field lines (red)")
                .on_hover_text(
                    "Streamlines of the gravitational field, with arrowheads and a glow \
                     falling along them the way the field points",
                );
        }
        ui.checkbox(&mut self.show_path, "Path");
        if ui
            .checkbox(&mut self.show_prediction, "Controller prediction (red)")
            .on_hover_text("Upcoming motion predicted by the controller, up to 5 seconds ahead")
            .changed()
        {
            self.prediction = orbits::Forecast::default();
        }
        ui.checkbox(&mut self.show_collision_meshes, "Collision meshes")
            .on_hover_text("Orange outlines of the ship, planet and moon collision shapes");
        ui.add_enabled_ui(self.debug_view(), |ui| {
            ui.checkbox(&mut self.show_velocity, "Velocity (green)");
            ui.checkbox(&mut self.show_acceleration, "Acceleration (red)");
        });
        if !in_flight {
            return;
        }
        ui.horizontal(|ui| {
            if ui.button("Fit view").clicked() {
                if online {
                    self.online_fit = true;
                } else {
                    self.fit_pending = true;
                }
            }
            if ui.button("Reset view").clicked() {
                self.camera = Camera::default();
                self.gizmo.stop();
                if online {
                    self.online_fit = true;
                }
            }
        });
    }
}
