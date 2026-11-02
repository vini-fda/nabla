//! The game: pick a test case and fly the ship to its goal, on a pannable, zoomable canvas,
//! next to plots of the state and control signals. An autopilot can fly it instead, with one of
//! the control policies.

use egui_plot::{AxisHints, GridInput, GridMark, Legend, Line, Plot, PlotPoint, VLine};
use nabla_core::State;
use nabla_core::problem::{Case, Outcome};
use nabla_core::solutions::{SOLUTIONS, StoredSolution};
use nabla_core::suite;

use crate::camera::{Camera, Vec3};
use crate::canvas_buttons::{self, Button};
use crate::controller_view;
use crate::formula::{self, Formula};
use crate::game::{self, Aim, Game, Input, Objective, Steering};
use crate::gamepad::{self, Gamepads};
use crate::gizmo::ViewGizmo;
use crate::grid::{GridStyle, draw_grid};
use crate::keys;
use crate::notifications::Notifications;
use crate::plume::Plume;
use crate::replay::{Replay, Series};
use crate::ship3d::{self, DepthMap, SHIP_GLB, ShipModel, ShipRenderer};
use crate::sound::Sound;
use crate::theme::{self, icons, palette};

mod orbits;
mod title;
mod worlds;
use title::{Screen, TitlePage};

const TRIANGLE_STROKE_WIDTH: f32 = 2.0;

const PLOT_HEIGHT: f32 = 110.0;
/// Pixels between neighboring tick labels at which the plots' axes start showing them, dim,
/// and at which they show them at full strength. Each axis has grid lines at three steps
/// (see [`plot_grid`]), so the labels of the finer steps show dimmed between those of the
/// coarser ones, in the text color.
const X_LABEL_SPACING: (f32, f32) = (24.0, 75.0);
const Y_LABEL_SPACING: (f32, f32) = (12.0, 45.0);
const CONTROLS_PANEL: SidePanel = SidePanel {
    dock: Dock::Left,
    id: "config_panel",
    title: "Settings",
    // Narrowest the contents can be dragged to; below this their rows would be cut off.
    min_width: 320.0,
    default_width: 320.0,
};
const PLOTS_PANEL: SidePanel = SidePanel {
    dock: Dock::Right,
    id: "plots_panel",
    title: "State plots",
    min_width: 220.0,
    default_width: 300.0,
};
/// Width of the vertical title bar along a side panel's window edge: all that is left of the
/// panel when it is collapsed.
const TITLE_BAR_WIDTH: f32 = 30.0;
/// Seconds the pointer must stay away from a collapsed panel's title bar before the bar
/// slides out of the window, leaving only its triangle.
const TITLE_BAR_HIDE_DELAY: f64 = 0.5;
/// How close (in points) the pointer must come to a hidden title bar to bring it back.
const TITLE_BAR_REVEAL_MARGIN: f32 = 16.0;
/// Seconds the title bar takes to slide out of or back into the window.
const TITLE_BAR_SLIDE_TIME: f32 = 0.3;

const POS_X_COLOR: egui::Color32 = palette::RED;
const POS_Y_COLOR: egui::Color32 = palette::BLUE;
const VEL_X_COLOR: egui::Color32 = palette::PEACH;
const VEL_Y_COLOR: egui::Color32 = palette::TEAL;
const THETA_COLOR: egui::Color32 = palette::MAUVE;
const OMEGA_COLOR: egui::Color32 = palette::YELLOW;
const THRUST_COLOR: egui::Color32 = palette::RED;
const TORQUE_COLOR: egui::Color32 = palette::SAPPHIRE;
const CURSOR_COLOR: egui::Color32 = palette::OVERLAY1;

const DEFAULT_CIRCUMRADIUS: f32 = 1.0;
const DEFAULT_ALPHA: f32 = 0.3;

// HUD-style vector arrows: fixed pixel-per-magnitude scales so arrow length
// stays readable regardless of camera zoom.
const VELOCITY_ARROW_PX_PER_UPS: f32 = 20.0;
const ACCELERATION_ARROW_PX_PER_UPS2: f32 = 20.0;
const ARROW_STROKE_WIDTH: f32 = 2.0;
const ARROW_MIN_PIXELS: f32 = 4.0;

const VELOCITY_COLOR: egui::Color32 = palette::GREEN;
const ACCELERATION_COLOR: egui::Color32 = palette::RED;

const PATH_COLOR: egui::Color32 = palette::OVERLAY0;
const TRAVELED_COLOR: egui::Color32 = palette::BLUE;
const PATH_MIN_SEGMENT_PX: f32 = 0.5;

const GOAL_COLOR: egui::Color32 = palette::YELLOW;
/// The heading the stability assist holds, or would.
const SAS_HEADING_COLOR: egui::Color32 = palette::SAPPHIRE;
/// The line from the ship to the pointer, in pointing mode: green while the ship faces the
/// pointer, within the cone, and so thrusts.
const AIM_COLOR: egui::Color32 = palette::OVERLAY1;
const AIM_THRUST_COLOR: egui::Color32 = palette::GREEN;
/// How near the ship (in points) the pointer commands nothing: the heading toward it is not
/// defined at the ship itself.
const AIM_DEAD_ZONE_PX: f32 = 12.0;
/// How far the controller's right stick must be pushed (of its full travel) to aim, and how far
/// from the ship (in points) its aim is drawn.
const STICK_AIM_MIN: f32 = 0.5;
const STICK_AIM_REACH_PX: f32 = 90.0;
/// How fast the controller's R1 and L1 zoom while held: the zoom's logarithm per second, so
/// about 4x a second.
const CONTROLLER_ZOOM_RATE: f32 = 1.4;
/// The thrust cone's default half-angle, and its range, in degrees.
const DEFAULT_AIM_CONE: f64 = 10.0;
const AIM_CONE_RANGE: std::ops::RangeInclusive<f64> = 1.0..=90.0;
const GOAL_RADIUS_PX: f32 = 10.0;
const GOAL_LABEL_OFFSET_PX: f32 = 12.0;
const START_RADIUS_PX: f32 = 5.0;

/// Below this width (in points), e.g. on phones, the panels are shown one at a time.
const COMPACT_WIDTH: f32 = 800.0;

/// How the canvas shows the ship.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CanvasView {
    /// The textured 3D model seen from above, with the path, goal and a faint grid.
    #[default]
    Ship3d,
    /// A flat triangle with velocity and acceleration arrows, on a labeled grid.
    Debug,
}

/// What the compact (narrow-screen) layout shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CompactView {
    #[default]
    Canvas,
    Controls,
    Plots,
}

/// Fraction of the canvas left empty around the ship and the goal by "Fit view".
const FIT_MARGIN: f32 = 0.15;

/// Range of sampling periods offered by the slider (the suites use `suite::H`).
const H_MIN: f64 = 0.01;
const H_MAX: f64 = 0.1;

/// Case selected at startup.
const DEFAULT_CASE: &str = "agile";

/// A test case and the suite it comes from.
struct CaseEntry {
    suite: &'static str,
    case: Case,
}

fn all_cases() -> Vec<CaseEntry> {
    let standard = suite::standard().into_iter().map(|case| CaseEntry {
        suite: "standard",
        case,
    });
    let holdout = suite::holdout().into_iter().map(|case| CaseEntry {
        suite: "holdout",
        case,
    });
    let mut cases: Vec<CaseEntry> = standard.chain(holdout).collect();
    // The default case goes first in the list.
    if let Some(i) = cases.iter().position(|e| e.case.name == DEFAULT_CASE) {
        cases[..=i].rotate_right(1);
    }
    cases
}

/// Plot series of the flight.
struct Plots {
    x: Vec<PlotPoint>,
    y: Vec<PlotPoint>,
    vx: Vec<PlotPoint>,
    vy: Vec<PlotPoint>,
    theta: Vec<PlotPoint>,
    omega: Vec<PlotPoint>,
    thrust: Vec<PlotPoint>,
    torque: Vec<PlotPoint>,
}

fn points(series: &Series) -> Vec<PlotPoint> {
    series.iter().map(|&[t, v]| PlotPoint::new(t, v)).collect()
}

impl Plots {
    fn new(r: &Replay) -> Self {
        Self {
            x: points(&r.state_series(|s| s.x)),
            y: points(&r.state_series(|s| s.y)),
            vx: points(&r.state_series(|s| s.vx)),
            vy: points(&r.state_series(|s| s.vy)),
            theta: points(&r.state_series(|s| s.theta)),
            omega: points(&r.state_series(|s| s.omega)),
            thrust: points(&r.control_series(|u| u.thrust)),
            torque: points(&r.control_series(|u| u.torque)),
        }
    }
}

pub struct ViewerApp {
    world_setup: crate::world::World,
    world_file: Option<std::sync::mpsc::Receiver<crate::world_io::FileResult>>,
    world_error: Option<String>,
    circumradius: f32,
    alpha: f32,
    camera: Camera,
    gizmo: ViewGizmo,
    /// Show the navigation gizmo.
    show_gizmo: bool,
    /// Dragging the canvas orbits the camera, rather than panning.
    drag_orbits: bool,
    show_velocity: bool,
    show_acceleration: bool,
    show_path: bool,
    show_prediction: bool,
    prediction: orbits::Forecast,
    /// Overlay the circular collision shapes in either render mode.
    show_collision_meshes: bool,
    /// Draw the background grid.
    show_grid: bool,
    /// Show the thrusters' plumes in the 3D view.
    show_plume: bool,
    follow_ship: bool,
    /// Show the keyboard and mouse help over the canvas.
    show_help: bool,
    /// The flight on the selected case.
    game: Game,
    /// What letting go of the turn does: kept from flight to flight.
    steering: Steering,
    /// Title-screen preference, not the active flight's controller state.
    autopilot_on_start: bool,
    orbit_controls: orbits::Controls,
    /// The control policy the autopilot flies with.
    policy: &'static StoredSolution,
    /// How the policy does on the selected case, flown from its start, for comparison.
    policy_outcome: Outcome,
    /// Why the autopilot last switched itself off, if it failed.
    autopilot_fault: Option<String>,
    /// What just happened, over the canvas.
    notifications: Notifications,
    /// Pointing mode: while the camera follows the ship, the mouse or a finger held on the
    /// canvas turns the ship toward it, and thrusts once it faces it.
    pointing: bool,
    /// Half-angle of the cone around the pointer within which the ship thrusts, in degrees.
    aim_cone: f64,
    /// Where the player points, as the canvas last saw it: on screen, and the heading toward
    /// it from the ship.
    aiming: Option<(egui::Pos2, f64)>,
    /// The canvas's areas that other widgets take (the buttons, the gizmo), as last drawn: a
    /// touch there never points.
    reserved: Vec<egui::Rect>,
    /// Which screen shows: the title screen, or the flight.
    screen: Screen,
    /// What the title screen shows below its buttons.
    title_page: TitlePage,
    /// The starting conditions picked on the title screen: a preset's, named after it, or the
    /// player's own.
    setup: Case,
    /// Sampling period the selected case runs with, in place of its own.
    h: f64,
    /// Show the policy's source code window.
    show_code: bool,

    sound: Sound,
    /// Game controllers, read every frame.
    gamepads: Gamepads,
    /// Show the controller's wireframe, lit by its state, under the flight.
    show_controller: bool,
    cases: Vec<CaseEntry>,
    fit_pending: bool,
    /// Wide layout: show the left (controls) and right (plots) side panels.
    show_controls: bool,
    show_plots: bool,
    compact_view: CompactView,
    view: CanvasView,
    /// Draws the 3D ship and its plumes; `None` where OpenGL can't (then the canvas shows the
    /// debug view).
    ship3d: Option<(ShipRenderer, Plume)>,
}

impl Default for ViewerApp {
    fn default() -> Self {
        let case = suite::find(DEFAULT_CASE).expect("the default case exists");
        let policy = &SOLUTIONS[0];
        Self {
            world_setup: crate::world::World::vacuum(&case, DEFAULT_CIRCUMRADIUS),
            world_file: None,
            world_error: None,
            circumradius: DEFAULT_CIRCUMRADIUS,
            alpha: DEFAULT_ALPHA,
            camera: Camera::default(),
            gizmo: ViewGizmo::default(),
            show_gizmo: true,
            drag_orbits: false,
            show_velocity: true,
            show_acceleration: true,
            show_path: true,
            show_prediction: true,
            prediction: orbits::Forecast::default(),
            show_collision_meshes: false,
            show_grid: true,
            show_plume: true,
            follow_ship: true,
            show_help: false,
            policy_outcome: (policy.simulate)(&case).outcome,
            game: Game::new(case.clone()),
            steering: Steering::default(),
            autopilot_on_start: false,
            orbit_controls: orbits::Controls::default(),
            policy,
            autopilot_fault: None,
            notifications: Notifications::default(),
            pointing: false,
            aim_cone: DEFAULT_AIM_CONE,
            aiming: None,
            reserved: Vec::new(),
            screen: Screen::default(),
            title_page: TitlePage::default(),
            setup: case,
            h: suite::H,
            show_code: false,
            sound: Sound::default(),
            gamepads: Gamepads::default(),
            show_controller: false,
            cases: all_cases(),
            fit_pending: true,
            show_controls: true,
            show_plots: true,
            compact_view: CompactView::default(),
            view: CanvasView::default(),
            ship3d: None,
        }
    }
}

impl ViewerApp {
    /// Called once before the first frame: a flight on the default case.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let ship3d = cc
            .wgpu_render_state
            .as_ref()
            .ok_or_else(|| "not running on wgpu".to_owned())
            .and_then(|render_state| {
                let model = ShipModel::load(SHIP_GLB)?;
                // egui's render pass, into which the ship is drawn: see `MULTISAMPLING`.
                let samples = if cfg!(target_arch = "wasm32") {
                    1
                } else {
                    u32::from(crate::MULTISAMPLING)
                };
                let depth = eframe::egui_wgpu::depth_format_from_bits(crate::DEPTH_BUFFER, 0);
                crate::planets::init(render_state, samples, depth);
                Ok((
                    ShipRenderer::new(render_state, &model, samples, depth),
                    Plume::new(render_state, samples, depth),
                ))
            })
            .inspect_err(|e| log::warn!("No 3D view: {e}"))
            .ok();
        Self {
            ship3d,
            ..Self::default()
        }
    }

    /// The starting conditions, with the sampling period set by the slider.
    fn selected_case(&self) -> Case {
        let mut case = self.setup.clone();
        case.task.params.h = self.h;
        case
    }

    /// Score the policy on the starting conditions, for comparison with the flight.
    fn score_policy(&mut self) {
        self.policy_outcome = (self.policy.simulate)(&self.selected_case()).outcome;
    }

    /// The flight, and its present as `(k, tau)`.
    fn shown(&self) -> (&Replay, usize, f64) {
        let (k, offset) = self.game.moment();
        (&self.game.replay, k, offset)
    }

    /// Sample index `k`, offset `tau` into the step, and the exact state at present.
    fn now(&self) -> (usize, f64, State) {
        let (replay, k, offset) = self.shown();
        (k, offset, replay.state_at(k, offset))
    }

    /// Start a flight on the starting conditions, from their start, following the ship.
    fn start_game(&mut self) {
        let restarting = self.screen == Screen::Flight;
        let previous = self.game.objective();
        let engage = if restarting {
            previous == Objective::ReachB && self.assisted()
        } else {
            self.autopilot_on_start
        };
        self.game = Game::in_world(self.selected_case(), self.selected_world());
        self.reset_orbit_controls();
        if restarting && previous == Objective::ReachB {
            self.choose_objective(Objective::ReachB);
        }
        if engage {
            self.engage_objective();
        }
        self.screen = Screen::Flight;
        self.follow_ship = true;
        self.fit_pending = true;
        self.compact_view = CompactView::Canvas;
    }

    /// The same switch serves every objective; switching off preserves the objective.
    fn toggle_autopilot(&mut self) {
        if self.assisted() {
            self.manual_flight();
        } else {
            self.engage_objective();
        }
    }

    /// Tell of the landing, at sample `n`: by the autopilot if it flew all the way.
    fn notify_landing(&mut self, n: usize) {
        let time = self.game.replay.time(n);
        let text = if self.game.autopiloted() {
            format!("The autopilot landed at B in {n} steps ({time:.2} s)")
        } else {
            format!("Landed at B in {n} steps ({time:.2} s)")
        };
        self.notifications.post(text, palette::GREEN);
    }

    /// Pause the flight, or resume it.
    fn toggle_pause(&mut self) {
        self.game.paused = !self.game.paused;
        let text = if self.game.paused {
            "Paused"
        } else {
            "Resumed"
        };
        self.notifications.post(text, palette::WHITE);
    }

    /// Turn the stability assist on or off.
    fn toggle_steering(&mut self) {
        self.steering = toggled(self.steering);
        let text = match self.steering {
            Steering::Sas => "Stability assist on",
            Steering::Raw => "Stability assist off",
        };
        self.notifications.post(text, palette::WHITE);
    }

    /// The flight's controls held on the keyboard: Space (or Up) thrusts, Left and Right turn,
    /// and Shift scales them down for fine control.
    fn keyboard_input(ui: &egui::Ui) -> Input {
        if ui.ctx().memory(|m| m.focused().is_some()) {
            return Input::default();
        }
        ui.input(|i| {
            // A key pressed and released between two frames still counts, for that frame.
            let held = |key| i.key_down(key) || i.key_pressed(key);
            let left = f64::from(u8::from(held(egui::Key::ArrowLeft)));
            let right = f64::from(u8::from(held(egui::Key::ArrowRight)));
            Input {
                thrust: held(egui::Key::Space) || held(egui::Key::ArrowUp),
                turn: left - right,
                fine: i.modifiers.shift,
                aim: None,
            }
        })
    }

    /// The controller's objective/preview/engage/manual commands. Options pauses and Create
    /// toggles the controller view.
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
            self.show_controller = !self.show_controller;
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

    /// The autopilot: the policy it flies with, its switch, its source, and its sampling period.
    fn autopilot_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Autopilot");
        if self.game.objective() == Objective::ReachB {
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
            if self.game.objective() == Objective::ReachB {
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
    fn policy_ui(&mut self, ui: &mut egui::Ui) {
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
            if self.assisted() && self.game.objective() == Objective::ReachB {
                self.engage_objective();
            }
        }
    }

    /// Optional window showing the policy's source with syntax highlighting and line numbers.
    fn code_window(&mut self, ctx: &egui::Context) {
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
            if self.game.objective() == Objective::ReachB {
                row(
                    &formula::GOAL,
                    "goal",
                    format!("({:.2}, {:.2}) m", task.goal.x, task.goal.y),
                );
            }
        });
        if self.game.objective() != Objective::ReachB {
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
            row(
                &formula::POSITION,
                "position",
                format!("({:.3}, {:.3}) m", s.x, s.y),
            );
            row(
                &formula::VELOCITY,
                "velocity",
                format!("({:.3}, {:.3}) m/s", s.vx, s.vy),
            );
            row(
                &formula::THETA,
                "heading angle",
                format!("{:.3} rad", s.theta),
            );
            row(
                &formula::OMEGA,
                "angular velocity",
                format!("{:.3} rad/s", s.omega),
            );
            if self.game.objective() == Objective::ReachB {
                row(
                    &formula::DISTANCE,
                    "distance to the goal B",
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
                |u| format!("({:.3} N, {:.3} N m)", u.thrust, u.torque),
            );
            row(&formula::CONTROL, "control: thrust and torque", control);
            if let Objective::Orbit(target) = self.game.objective() {
                let body = &self.game.world.bodies[target.body];
                let altitude = (s.x - f64::from(body.position[0]))
                    .hypot(s.y - f64::from(body.position[1]))
                    - f64::from(body.radius);
                ui.label("Altitude")
                    .on_hover_text("Altitude above the selected body's surface");
                ui.monospace(format!("{altitude:.4} m"));
                ui.end_row();
            }
        });
    }

    /// The display options, and `in_flight`, the buttons that fit or reset the camera.
    fn display_ui(&mut self, ui: &mut egui::Ui, in_flight: bool) {
        ui.heading("Display options");
        ui.add_enabled_ui(self.ship3d.is_some(), |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.view, CanvasView::Ship3d, "3D");
                ui.selectable_value(&mut self.view, CanvasView::Debug, "Debug")
                    .on_hover_text("Flat ship with velocity and acceleration arrows");
            });
        });
        ui.add_enabled(
            !in_flight,
            egui::Slider::new(&mut self.circumradius, 0.05..=10.0)
                .logarithmic(true)
                .text("ship size (world units)"),
        )
        .on_hover_text("Ship size and collision radius; set before starting a flight");
        ui.checkbox(&mut self.show_grid, "Grid");
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
        ui.checkbox(&mut self.show_controller, "Controller view")
            .on_hover_text(
                "A wireframe of the game controller under the flight, lit as you use it. \
                 The controller's Create button shows or hides it too",
            );
        ui.add_enabled_ui(!self.debug_view(), |ui| {
            ui.checkbox(&mut self.show_plume, "Thruster plumes");
        });
        ui.add_enabled_ui(self.debug_view(), |ui| {
            ui.checkbox(&mut self.show_velocity, "Velocity (green)");
            ui.checkbox(&mut self.show_acceleration, "Acceleration (red)");
        });
        if !in_flight {
            return;
        }
        ui.horizontal(|ui| {
            if ui.button("Fit view").clicked() {
                self.fit_pending = true;
            }
            if ui.button("Reset view").clicked() {
                self.camera = Camera::default();
                self.gizmo.stop();
            }
        });
    }

    /// The user inputs help: touch gestures on a phone or tablet, keyboard and mouse elsewhere.
    /// With a `controller` connected, its controls too.
    fn user_inputs_ui(ui: &mut egui::Ui, controller: bool) {
        let k = keys::Keys::new(ui.ctx());
        ui.heading("User inputs");
        if k.touch() {
            Self::touch_inputs_ui(ui);
        } else {
            Self::keyboard_mouse_inputs_ui(ui, k);
        }
        if controller {
            Self::controller_flight_ui(ui);
        }
    }

    /// The flight's controls on a game controller, named as on a Sony one.
    fn controller_flight_ui(ui: &mut egui::Ui) {
        use keys::{Key, caps, joiner};

        input_group(ui, "Controller", |ui| {
            input_row(
                ui,
                |ui| caps(ui, &[Key::Word("left stick")]),
                |ui| ui.label("Turn, harder the further"),
            );
            input_row(
                ui,
                |ui| {
                    caps(ui, &[Key::Word("Cross")]);
                    joiner(ui, "/");
                    caps(ui, &[Key::Word("R2")]);
                },
                |ui| ui.label("Thrust"),
            );
            input_row(
                ui,
                |ui| {
                    caps(ui, &[Key::Word("L2")]);
                    joiner(ui, "held");
                },
                |ui| ui.label(format!("Fine control ({}x power)", game::FINE)),
            );
            input_row(
                ui,
                |ui| {
                    caps(ui, &[Key::Word("R1")]);
                    joiner(ui, "/");
                    caps(ui, &[Key::Word("L1")]);
                },
                |ui| ui.label("Zoom in / out"),
            );
            input_row(
                ui,
                |ui| caps(ui, &[Key::Word("right stick")]),
                |ui| ui.label("Aim: turn that way; thrust when facing it"),
            );
            input_row(
                ui,
                |ui| caps(ui, &[Key::Word("Triangle")]),
                |ui| ui.label("SAS: hold the heading when not turning"),
            );
            input_row(
                ui,
                |ui| caps(ui, &[Key::Word("Square")]),
                |ui| ui.label("Engage the selected objective"),
            );
            input_row(
                ui,
                |ui| caps(ui, &[Key::Word("Circle")]),
                |ui| ui.label("Return to manual flight"),
            );
            input_row(
                ui,
                |ui| caps(ui, &[Key::Word("D-pad left / right")]),
                |ui| ui.label("Select Reach B / Orbit objective"),
            );
            input_row(
                ui,
                |ui| caps(ui, &[Key::Word("D-pad up/down")]),
                |ui| ui.label("Orbit altitude; hold L2 for fine adjustment"),
            );
            input_row(
                ui,
                |ui| caps(ui, &[Key::Word("R3")]),
                |ui| ui.label("Select orbit target"),
            );
            input_row(
                ui,
                |ui| caps(ui, &[Key::Word("Options")]),
                |ui| ui.label("Pause / resume"),
            );
            input_row(
                ui,
                |ui| caps(ui, &[Key::Word("Create")]),
                |ui| ui.label("Show / hide the controller view"),
            );
        });
    }

    /// The flight's keys.
    fn keyboard_flight_ui(ui: &mut egui::Ui, k: keys::Keys) {
        use icons::outline::ROCKET_LAUNCH;
        use keys::{A, F, LEFT, P, R, Run, SPACE, T, caps, joiner, label_with_icons};

        input_group(ui, "Flight", |ui| {
            input_row(ui, |ui| caps(ui, &[SPACE]), |ui| ui.label("Thrust"));
            input_row(
                ui,
                |ui| caps(ui, &[LEFT, keys::RIGHT]),
                |ui| ui.label("Turn left / right"),
            );
            input_row(
                ui,
                |ui| {
                    caps(ui, &[k.shift()]);
                    joiner(ui, "held");
                },
                |ui| ui.label(format!("Fine control ({}x power)", game::FINE)),
            );
            input_row(
                ui,
                |ui| {
                    caps(ui, &[P]);
                    joiner(ui, "/");
                    caps(ui, &[R]);
                },
                |ui| ui.label("Pause / restart"),
            );
            input_row(
                ui,
                |ui| caps(ui, &[T]),
                |ui| ui.label("SAS: hold the heading when not turning"),
            );
            input_row(
                ui,
                |ui| caps(ui, &[A]),
                |ui| ui.label("Autopilot: the policy flies"),
            );
            input_row(
                ui,
                |ui| keys::gesture(ui, icons::outline::MOUSE_LEFT_CLICK, "hold (Point)"),
                |ui| ui.label("Turn toward the pointer; thrust when facing it"),
            );
            input_row(
                ui,
                |ui| caps(ui, &[F]),
                |ui| {
                    label_with_icons(
                        ui,
                        &[
                            Run::Text("Follow the ship (or "),
                            Run::Icon(ROCKET_LAUNCH),
                            Run::Text(")"),
                        ],
                    );
                },
            );
        });
    }

    /// The keyboard and mouse inputs, in groups, the keys drawn as keycaps left of what they do.
    fn keyboard_mouse_inputs_ui(ui: &mut egui::Ui, k: keys::Keys) {
        use icons::outline::{MOUSE_LEFT_CLICK, MOUSE_SCROLL};
        use keys::gesture;

        Self::keyboard_flight_ui(ui, k);
        input_group(ui, "View", |ui| {
            input_row(
                ui,
                |ui| gesture(ui, MOUSE_LEFT_CLICK, "drag"),
                drag_action_ui,
            );
            input_row(
                ui,
                |ui| gesture(ui, MOUSE_SCROLL, "scroll"),
                |ui| ui.label("Zoom"),
            );
            input_row(
                ui,
                |ui| gesture(ui, MOUSE_LEFT_CLICK, "drag axes"),
                |ui| ui.label("Orbit; click one to view along it"),
            );
        });
    }

    /// The flight's on-screen controls.
    fn touch_flight_ui(ui: &mut egui::Ui) {
        use icons::outline::{FIRE, HAND_SWIPE_RIGHT, HAND_TAP, ROCKET_LAUNCH};
        use keys::gesture;

        input_group(ui, "Flight", |ui| {
            input_row(ui, |ui| gesture(ui, FIRE, "hold"), |ui| ui.label("Thrust"));
            input_row(
                ui,
                |ui| gesture(ui, HAND_SWIPE_RIGHT, "drag stick"),
                |ui| ui.label("Turn, harder the further"),
            );
            input_row(
                ui,
                |ui| gesture(ui, HAND_TAP, "tap fine"),
                |ui| ui.label(format!("Fine control ({}x power)", game::FINE)),
            );
            input_row(
                ui,
                |ui| gesture(ui, HAND_TAP, "tap SAS"),
                |ui| ui.label("Hold the heading when not turning"),
            );
            input_row(
                ui,
                |ui| gesture(ui, HAND_TAP, "tap Autopilot"),
                |ui| ui.label("The policy flies"),
            );
            input_row(
                ui,
                |ui| gesture(ui, HAND_TAP, "hold (Point)"),
                |ui| ui.label("Turn toward the finger; thrust when facing it"),
            );
            input_row(
                ui,
                |ui| outline_glyph(ui, ROCKET_LAUNCH),
                |ui| ui.label("Follow the ship"),
            );
        });
    }

    /// The touch gestures, and what the flight's controls do, since a touch screen shows no
    /// tooltips.
    fn touch_inputs_ui(ui: &mut egui::Ui) {
        use icons::outline::{ARROWS_OUT_SIMPLE, HAND_SWIPE_RIGHT, HAND_TAP};
        use keys::gesture;

        Self::touch_flight_ui(ui);
        input_group(ui, "View", |ui| {
            input_row(
                ui,
                |ui| gesture(ui, HAND_SWIPE_RIGHT, "drag"),
                drag_action_ui,
            );
            input_row(
                ui,
                |ui| gesture(ui, ARROWS_OUT_SIMPLE, "pinch"),
                |ui| ui.label("Zoom"),
            );
            input_row(
                ui,
                |ui| gesture(ui, HAND_SWIPE_RIGHT, "drag axes"),
                |ui| ui.label("Orbit"),
            );
            input_row(
                ui,
                |ui| gesture(ui, HAND_TAP, "tap an axis"),
                |ui| ui.label("View along it"),
            );
        });
    }

    /// The bar along the bottom: pause and restart, the switches for the stability assist, the
    /// autopilot and pointing (with its cone), and the time. `compact`, for narrow screens,
    /// shortens the autopilot's, the cone's and the time's labels so the row fits a phone.
    fn game_bar_ui(&mut self, ui: &mut egui::Ui, compact: bool) {
        self.objective_ui(ui);
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

    /// The plots of the flight so far, which grows every frame.
    fn plots_ui(&self, ui: &mut egui::Ui) {
        let plots = Plots::new(&self.game.replay);
        let t = self.game.time();
        ui.heading("State plots");
        egui::ScrollArea::vertical().show(ui, |ui| {
            time_plot(
                ui,
                "Position",
                "m",
                &formula::POSITION,
                t,
                &[("x", &plots.x, POS_X_COLOR), ("y", &plots.y, POS_Y_COLOR)],
            );
            time_plot(
                ui,
                "Velocity",
                "m/s",
                &formula::VELOCITY,
                t,
                &[
                    ("vx", &plots.vx, VEL_X_COLOR),
                    ("vy", &plots.vy, VEL_Y_COLOR),
                ],
            );
            time_plot(
                ui,
                "Heading angle",
                "rad",
                &formula::THETA,
                t,
                &[("theta", &plots.theta, THETA_COLOR)],
            );
            time_plot(
                ui,
                "Angular velocity",
                "rad/s",
                &formula::OMEGA,
                t,
                &[("omega", &plots.omega, OMEGA_COLOR)],
            );
            time_plot(
                ui,
                "Thrust",
                "N",
                &formula::THRUST,
                t,
                &[("F", &plots.thrust, THRUST_COLOR)],
            );
            time_plot(
                ui,
                "Torque",
                "N m",
                &formula::TORQUE,
                t,
                &[("tau", &plots.torque, TORQUE_COLOR)],
            );
        });
    }

    /// The canvas. `inset` is how much of its right edge is taken, at the top, by a collapsed
    /// side panel's floating triangle: the gizmo and the buttons in the top right corner
    /// move left of it.
    fn canvas_ui(&mut self, ui: &mut egui::Ui, inset: f32) {
        let (rect, response) =
            ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        // Where the gizmo and the buttons go, in its top right corner.
        let corner = rect.with_max_x(rect.right() - inset);
        self.gizmo
            .input(ui, corner, &mut self.camera, self.show_gizmo);

        // The flight's pads, over the canvas, take the pointer from it where they are.
        let touch_device = keys::Keys::new(ui.ctx()).touch();
        let pointing = self.pointing_now();
        let visible = self.pads_visible(touch_device);
        let pads = &mut self.game.pads;
        pads.update(ui, rect, visible, &self.reserved);
        let on_pads = pads.touching() || (pointing && pads.aim_touch().is_some());

        self.drag_and_zoom(ui, &response, rect, on_pads);

        if self.fit_pending {
            fit_camera(&mut self.camera, self.game.view_bounds(), rect);
            self.fit_pending = false;
        }

        let painter = ui.painter_at(rect);
        // The scene first, which centers the camera on the ship when following it.
        self.draw_scene(ui, &painter, rect);
        self.aiming = self.read_aim(&response, rect);
        self.draw_game_overlay(&painter, rect, touch_device);
        if self.show_gizmo {
            self.gizmo.paint(ui, &painter, corner, &self.camera);
        }
        let buttons = canvas_buttons::show(
            ui,
            &painter,
            corner,
            &mut [
                Button::toggle(
                    icons::outline::SPHERE,
                    &mut self.show_gizmo,
                    "Show the navigation gizmo",
                ),
                Button::toggle(
                    icons::outline::ROCKET_LAUNCH,
                    &mut self.follow_ship,
                    "Follow the ship (F)",
                ),
                Button::toggle(
                    icons::outline::PATH,
                    &mut self.show_path,
                    "Show the path flown so far",
                ),
                Button::switch(
                    [
                        icons::outline::ARROWS_OUT_CARDINAL,
                        icons::outline::ARROW_COUNTER_CLOCKWISE,
                    ],
                    &mut self.drag_orbits,
                    [
                        "Dragging the canvas pans. Click to rotate instead",
                        "Dragging the canvas rotates the camera. Click to pan instead",
                    ],
                ),
                Button::switch(
                    [icons::outline::GRID_FOUR, icons::outline::PERSPECTIVE],
                    &mut self.camera.perspective,
                    [
                        "Orthographic view. Click for perspective",
                        "Perspective view. Click for orthographic",
                    ],
                ),
                Button::toggle(
                    icons::outline::SEAL_QUESTION,
                    &mut self.show_help,
                    "Keyboard and mouse help",
                )
                .color(palette::YELLOW),
            ],
        );
        self.reserved = std::iter::once(buttons)
            .chain(ViewGizmo::area(corner).filter(|_| self.show_gizmo))
            .collect();
        if self.show_help {
            self.help_ui(ui, rect, buttons);
        }
    }

    /// Dragging and zooming the canvas, whose `response` this is, in `rect`. `on_pads`: a finger
    /// is on the flight's controls, or points, so another finger is not a pinch.
    fn drag_and_zoom(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        on_pads: bool,
    ) {
        // Primary drag orbits the camera or pans, as the pan/rotate button says. Panning is
        // disabled while following the ship, since the center is locked to the ship's
        // position each frame. In pointing mode, the drag points instead.
        let pointing = self.pointing_now();
        if pointing && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if response.dragged_by(egui::PointerButton::Primary) && !pointing {
            if self.drag_orbits {
                self.gizmo.orbit(&mut self.camera, response.drag_delta());
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if !self.follow_ship {
                self.camera.pan_by_screen(response.drag_delta());
            }
        }

        // Zoom with scroll/pinch while the cursor is over the canvas. A finger on a pad, or
        // pointing, and another on the canvas are not a pinch.
        if let Some(hover_pos) = response.hover_pos() {
            let (scroll_y, zoom_delta) = ui.ctx().input(|i| {
                let zoom = if on_pads { 1.0 } else { i.zoom_delta() };
                (i.smooth_scroll_delta.y, zoom)
            });
            let factor = zoom_delta * (scroll_y / 200.0).exp();
            if factor.is_finite() && factor > 0.0 && (factor - 1.0).abs() > f32::EPSILON {
                self.camera.zoom_at(factor, hover_pos, rect);
            }
        }

        // The controller's R1 zooms in and L1 out while held, about the canvas's center: the
        // ship, while following it.
        let zoom = self.gamepads.state().map_or(0.0, gamepad::State::zoom);
        if zoom != 0.0 {
            let dt = ui.input(|i| i.stable_dt).min(0.1);
            let factor = (zoom * CONTROLLER_ZOOM_RATE * dt).exp();
            self.camera.zoom_at(factor, rect.center(), rect);
        }
    }

    /// Whether the pointer flies the ship now: with pointing on and the camera following the
    /// ship, without which a still pointer would not hold a direction, and while the player
    /// flies rather than the autopilot.
    fn pointing_now(&self) -> bool {
        self.pointing && self.follow_ship && !self.assisted()
    }

    /// Whether the on-screen controls show: on a touch screen, unless the player points, when
    /// the whole canvas points, or the autopilot flies.
    fn pads_visible(&self, touch_device: bool) -> bool {
        self.game.pads.visible(touch_device) && !self.pointing_now() && !self.assisted()
    }

    /// Where the player points, in pointing mode: the mouse pressed on the canvas, or on a
    /// touch screen the touch that points, on screen, and the heading toward it from the ship,
    /// unless it is too near the ship for one.
    ///
    /// Without a pointer, the controller's right stick aims, while pushed far enough, toward
    /// where it points on screen: a direction, which needs neither Point nor the camera
    /// following the ship. Its aim is drawn at a fixed distance from the ship.
    fn read_aim(&self, response: &egui::Response, rect: egui::Rect) -> Option<(egui::Pos2, f64)> {
        if self.assisted() {
            return None;
        }
        let game = &self.game;
        let s = game.state();
        let ship = world(s.x, s.y);
        let on_screen = self.camera.world_to_screen(ship, rect)?;
        let pointer = if !self.pointing_now() {
            None
        } else if game.pads.touched() {
            game.pads.aim_touch()
        } else {
            response
                .is_pointer_button_down_on()
                .then(|| response.interact_pointer_pos())
                .flatten()
        };
        let target = pointer.or_else(|| {
            let [x, y] = self.gamepads.state()?.right_stick;
            // Screen y points down; the stick's y up.
            let push = egui::vec2(x, -y);
            (push.length() >= STICK_AIM_MIN)
                .then(|| on_screen + push.normalized() * STICK_AIM_REACH_PX)
        })?;
        if on_screen.distance(target) < AIM_DEAD_ZONE_PX {
            return None;
        }
        let offset = self.camera.screen_to_world(target, rect)? - ship;
        Some((target, f64::from(offset.y).atan2(f64::from(offset.x))))
    }

    /// The keyboard and mouse help, over the `canvas`, left of its `buttons` and level with
    /// their top. Where the canvas is too small for it, it shrinks, keeping its proportions, so
    /// it always shows whole. Escape closes it, as does its button.
    fn help_ui(&mut self, ui: &egui::Ui, canvas: egui::Rect, buttons: egui::Rect) {
        const GAP: f32 = 8.0;
        let controller = self.gamepads.state().is_some();
        // Laid out at its full size at the layer's origin, unclipped, then scaled and moved into
        // place by the layer's transform, which applies to its input too.
        let area = egui::Area::new(ui.scope_id().with("help"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::Pos2::ZERO)
            .constrain_to(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::splat(1.0e4),
            ))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| Self::user_inputs_ui(ui, controller));
            });
        let size = area.response.rect.size();
        let top_right = egui::pos2(buttons.left() - GAP, buttons.top());
        let room = egui::vec2(
            top_right.x - canvas.left() - GAP,
            canvas.bottom() - top_right.y - GAP,
        );
        let scale = (room / size).min_elem().clamp(0.1, 1.0);
        ui.ctx().set_transform_layer(
            area.response.layer_id,
            egui::emath::TSTransform::new(
                egui::vec2(top_right.x - scale * size.x, top_right.y),
                scale,
            ),
        );
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_help = false;
        }
    }

    /// Everything on the canvas but the gizmo.
    fn draw_scene(&mut self, ui: &egui::Ui, painter: &egui::Painter, rect: egui::Rect) {
        let debug = self.debug_view();
        let style = if debug {
            GridStyle::from_visuals(ui.visuals())
        } else {
            GridStyle::faint(ui.visuals())
        };
        let (_, _, state) = self.now();

        // Follow mode: keep the ship centered. Done after pan/zoom so zoom
        // still adjusts scale around the (now ship-centered) view.
        if self.follow_ship {
            self.camera.center = Vec3::plane(world(state.x, state.y));
        }

        if self.show_grid {
            draw_grid(painter, &self.camera, rect, &style);
        }
        let (replay, k, offset) = self.shown();
        if self.show_path {
            self.draw_path(painter, rect, replay, k, &state);
        }
        if self.game.objective() == Objective::ReachB {
            self.draw_goal(painter, rect, replay, &state);
        }
        if debug {
            crate::planets::draw_debug(painter, &self.camera, rect, &self.game.world.bodies);
        }
        match &self.ship3d {
            Some((renderer, plume)) if !debug => {
                let position = world(state.x, state.y);
                let depth = DepthMap::for_world(
                    renderer,
                    &self.camera,
                    rect,
                    position,
                    self.circumradius,
                    &self.game.world.bodies,
                );
                painter.add(crate::planets::paint(
                    &self.camera,
                    rect,
                    &depth,
                    &self.game.world.bodies,
                    renderer.gamma_output,
                ));
                if let Some(callback) = ship3d::paint_callback(
                    renderer,
                    &self.camera,
                    rect,
                    position,
                    state.theta as f32,
                    self.circumradius,
                    &depth,
                ) {
                    painter.add(callback);
                }
                // After the ship, whose depth they are tested against.
                if self.show_plume {
                    let t = replay.time(k) + offset;
                    if let Some(callback) =
                        plume.paint(&self.camera, rect, replay, t, self.circumradius, &depth)
                    {
                        painter.add(callback);
                    }
                }
            }
            _ => {
                self.draw_ship(painter, rect, &state);
                self.draw_arrows(painter, rect, replay, k, &state);
            }
        }
        self.draw_orbits(painter, rect);
        self.draw_prediction(painter, rect);
        if self.show_collision_meshes {
            // Use the same radii as the colliders, at the displayed (between-sample)
            // position, and draw last so meshes remain visible through the 3D models.
            let bodies = self
                .game
                .world
                .bodies
                .iter()
                .map(|body| (egui::vec2(body.position[0], body.position[1]), body.radius));
            let player = (world(state.x, state.y), self.game.world.players[0].radius);
            crate::planets::draw_circles(
                painter,
                &self.camera,
                rect,
                bodies.chain([player]),
                egui::Stroke::new(2.0, palette::PEACH),
            );
        }
    }

    /// Whether the canvas shows the debug view: when chosen, or when there is no 3D view.
    fn debug_view(&self) -> bool {
        self.view == CanvasView::Debug || self.ship3d.is_none()
    }

    /// Full path, and the path up to the ship's current (possibly between-samples) position.
    fn draw_path(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        replay: &Replay,
        k: usize,
        now: &State,
    ) {
        let states = &replay.trajectory.states;
        let point = |s: &State| Vec3::plane(world(s.x, s.y));
        let full = screen_path(&self.camera, rect, states.iter().map(point));
        let traveled = screen_path(
            &self.camera,
            rect,
            states[..=k].iter().chain([now]).map(point),
        );
        for run in full {
            painter.add(egui::Shape::line(
                run,
                egui::Stroke::new(1.0_f32, PATH_COLOR),
            ));
        }
        for run in traveled {
            painter.add(egui::Shape::line(
                run,
                egui::Stroke::new(2.0_f32, TRAVELED_COLOR),
            ));
        }

        if let Some(start) = self
            .camera
            .world_to_screen(world(states[0].x, states[0].y), rect)
        {
            painter.circle_stroke(
                start,
                START_RADIUS_PX,
                egui::Stroke::new(1.5_f32, TRAVELED_COLOR),
            );
        }
    }

    fn draw_goal(&self, painter: &egui::Painter, rect: egui::Rect, replay: &Replay, now: &State) {
        let task = &replay.case.task;
        let reached = task.reached(now);
        let color = GOAL_COLOR.gamma_multiply(if reached { 0.35 } else { 1.0 });

        // Position tolerance eps_p, drawn to scale once it is visible: a circle in the plane,
        // an ellipse once the view is tilted.
        let tolerance = task.tolerance.position as f32;
        if tolerance * self.camera.scale > 1.5 {
            let center = world(task.goal.x, task.goal.y);
            let circle = (0..=64).map(|i| {
                let angle = i as f32 / 64.0 * std::f32::consts::TAU;
                Vec3::plane(center + egui::Vec2::angled(angle) * tolerance)
            });
            for run in self.camera.project_polyline(circle, rect) {
                painter.add(egui::Shape::line(run, egui::Stroke::new(1.0_f32, color)));
            }
        }

        let Some(goal) = self
            .camera
            .world_to_screen(world(task.goal.x, task.goal.y), rect)
        else {
            return;
        };
        painter.circle_stroke(goal, GOAL_RADIUS_PX, egui::Stroke::new(2.0_f32, color));
        painter.circle_filled(goal, 2.0, color);
        painter.text(
            goal + egui::vec2(GOAL_LABEL_OFFSET_PX, -GOAL_LABEL_OFFSET_PX),
            egui::Align2::LEFT_BOTTOM,
            if reached { "B (reached)" } else { "B" },
            egui::FontId::monospace(12.0),
            color,
        );
    }

    /// Over the flight: where the goal is while it is off the canvas, the pads (where they
    /// show), and the notifications.
    fn draw_game_overlay(&self, painter: &egui::Painter, rect: egui::Rect, touch_device: bool) {
        let game = &self.game;
        let pads = self.pads_visible(touch_device);
        // Above the pads, where they show.
        let marker_area = if pads {
            rect.with_max_y(rect.bottom() - game::PADS_HEIGHT)
        } else {
            rect
        };
        let state = game.state();
        if let Some((pointer, heading)) = self.aiming {
            self.draw_aim(painter, rect, &state, pointer, heading);
        } else if !self.assisted()
            && let Some(heading) = game.sas_heading(self.steering)
        {
            self.draw_sas_heading(painter, rect, &state, heading);
        }
        if game.objective() == Objective::ReachB {
            self.draw_goal_marker(painter, rect, marker_area, &game.replay, &state);
        }
        if pads {
            game.pads.paint(painter, rect);
        }
        self.notifications.paint(painter, rect);
        if self.show_controller {
            let panel = controller_view::panel_rect(rect);
            controller_view::paint(painter, panel, self.gamepads.state());
        }
    }

    /// Where the player points, from the ship in state `s`: a line to the `pointer`, which
    /// points at `heading`, and the edges of the thrust cone around it, as long. Green while
    /// the ship faces the pointer within the cone, and so thrusts.
    fn draw_aim(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        s: &State,
        pointer: egui::Pos2,
        heading: f64,
    ) {
        let cone = self.aim_cone.to_radians();
        let color = if game::wrap(heading - s.theta).abs() <= cone {
            AIM_THRUST_COLOR
        } else {
            AIM_COLOR
        };
        let center = world(s.x, s.y);
        let Some(ship) = self.camera.world_to_screen(center, rect) else {
            return;
        };
        let reach = self
            .camera
            .screen_to_world(pointer, rect)
            .map_or(0.0, |p| (p - center).length());
        for side in [-1.0, 1.0] {
            let edge = center + egui::Vec2::angled((heading + side * cone) as f32) * reach;
            if let Some(edge) = self.camera.world_to_screen(edge, rect) {
                painter.line_segment(
                    [ship, edge],
                    egui::Stroke::new(1.0, color.gamma_multiply(0.35)),
                );
            }
        }
        painter.line_segment([ship, pointer], egui::Stroke::new(1.5, color));
        painter.circle_stroke(pointer, 6.0, egui::Stroke::new(1.5, color));
    }

    /// The heading the stability assist holds, or would hold if the player let go now: a faint
    /// line from the ship in state `s` out past its nose, ending in a ring.
    fn draw_sas_heading(&self, painter: &egui::Painter, rect: egui::Rect, s: &State, heading: f64) {
        const LENGTH: f32 = 2.5;
        const RING_RADIUS_PX: f32 = 4.0;
        let color = SAS_HEADING_COLOR.gamma_multiply(0.7);
        let center = world(s.x, s.y);
        let tip = center + egui::Vec2::angled(heading as f32) * LENGTH * self.circumradius;
        let ends = [center, tip].map(|p| self.camera.world_to_screen(p, rect));
        if let [Some(from), Some(to)] = ends {
            painter.line_segment([from, to], egui::Stroke::new(1.0, color));
            painter.circle_stroke(to, RING_RADIUS_PX, egui::Stroke::new(1.5, color));
        }
    }

    /// Where the goal is while it is off the canvas: a triangle at the edge of `area` (of the
    /// canvas) pointing at it, with the distance to it from the ship, in state `s`.
    fn draw_goal_marker(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        area: egui::Rect,
        replay: &Replay,
        s: &State,
    ) {
        const INSET: f32 = 24.0;
        const SIZE: f32 = 9.0;
        let task = &replay.case.task;
        let inner = area.shrink(INSET);
        let Some(goal) = self
            .camera
            .world_to_screen(world(task.goal.x, task.goal.y), rect)
        else {
            return;
        };
        if inner.contains(goal) {
            return;
        }
        let center = inner.center();
        let direction = goal - center;
        // How far along the direction the inner rectangle's edge is.
        let reach =
            (inner.width() / 2.0 / direction.x.abs()).min(inner.height() / 2.0 / direction.y.abs());
        if !reach.is_finite() {
            return;
        }
        let tip = center + direction * reach;
        let along = direction.normalized();
        let across = along.rot90();
        painter.add(egui::Shape::convex_polygon(
            vec![
                tip,
                tip - along * 1.6 * SIZE + across * SIZE,
                tip - along * 1.6 * SIZE - across * SIZE,
            ],
            GOAL_COLOR,
            egui::Stroke::NONE,
        ));
        let distance = (task.goal.x - s.x).hypot(task.goal.y - s.y);
        let label = tip - along * (2.6 * SIZE);
        painter.text(
            label,
            egui::Align2::CENTER_CENTER,
            format!("B {distance:.1} m"),
            egui::FontId::monospace(12.0),
            GOAL_COLOR,
        );
    }

    fn draw_ship(&self, painter: &egui::Painter, rect: egui::Rect, state: &State) {
        let center = world(state.x, state.y);
        let (sin_theta, cos_theta) = (state.theta as f32).sin_cos();
        let (sin_2alpha, cos_2alpha) = (2.0 * self.alpha).sin_cos();

        // Columns of P: three unit-circle vertices of the base isosceles triangle.
        // alpha = angle between the midline and the main side.
        let base_vertices = [
            (1.0_f32, 0.0_f32),
            (-cos_2alpha, sin_2alpha),
            (-cos_2alpha, -sin_2alpha),
        ];

        let Some(vertices) = base_vertices
            .iter()
            .map(|&(px, py)| {
                let x = cos_theta * px - sin_theta * py;
                let y = sin_theta * px + cos_theta * py;
                let vertex = center + egui::vec2(x, y) * self.circumradius;
                self.camera.world_to_screen(vertex, rect)
            })
            .collect::<Option<Vec<egui::Pos2>>>()
        else {
            return;
        };

        // Seen from below, the triangle is wound the other way; `convex_polygon` takes either.
        painter.add(egui::Shape::convex_polygon(
            vertices,
            palette::WHITE,
            egui::Stroke::new(TRIANGLE_STROKE_WIDTH, palette::BLACK),
        ));
    }

    fn draw_arrows(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        replay: &Replay,
        k: usize,
        state: &State,
    ) {
        let position = Vec3::plane(world(state.x, state.y));
        let (Some(center), Some(scale)) = (
            self.camera.project(position, rect),
            self.camera.scale_at(position, rect),
        ) else {
            return;
        };

        // World vectors -> screen vectors through the camera's view at the ship. Use fixed
        // pixel scales (decoupled from camera zoom) so arrows behave like HUD indicators.
        let arrow = |v: [f64; 2], px_per_unit: f32, color: egui::Color32| {
            let v = Vec3::plane(world(v[0], v[1]));
            let Some(screen) = self.camera.screen_vector_at(position, v, rect) else {
                return;
            };
            let screen = screen / scale * px_per_unit;
            if screen.length() > ARROW_MIN_PIXELS {
                painter.arrow(center, screen, egui::Stroke::new(ARROW_STROKE_WIDTH, color));
            }
        };
        if self.show_velocity {
            arrow(
                [state.vx, state.vy],
                VELOCITY_ARROW_PX_PER_UPS,
                VELOCITY_COLOR,
            );
        }
        if self.show_acceleration {
            let thrust = replay.acceleration(k, state);
            let gravity = crate::physics::gravity_acceleration(
                &self.game.world.bodies,
                self.game.world.gravity_constant,
                [state.x, state.y],
            );
            arrow(
                [thrust[0] + gravity[0], thrust[1] + gravity[1]],
                ACCELERATION_ARROW_PX_PER_UPS2,
                ACCELERATION_COLOR,
            );
        }
    }
}

impl ViewerApp {
    /// The left panel's content: case, autopilot, current state and display options.
    fn controls_ui(&mut self, ui: &mut egui::Ui) {
        // Scrolls sideways too, so rows are never cut off if the space is narrower than they are.
        egui::ScrollArea::both().show(ui, |ui| {
            ui.heading(&self.game.world.name);
            ui.label(format!(
                "Gravitational constant G: {}",
                self.game.world.gravity_constant
            ));
            self.world_files_ui(ui);
            if !self.game.world.bodies.is_empty() {
                ui.label(
                    "Orbit assist accounts for gravity. Reach B policies use the vacuum model.",
                );
            }
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

    /// Desktop layout: the game's bar along the bottom, controls left and plots right above it.
    fn wide_layout(&mut self, ui: &mut egui::Ui) {
        // The bar first, across the whole window, so neither side panel (nor its collapsed
        // title bar, which slides back in when the pointer comes near) reaches it.
        egui::Panel::bottom("game_bar").show(ui, |ui| self.game_bar_ui(ui, false));
        let mut show_controls = self.show_controls;
        CONTROLS_PANEL.show(ui, &mut show_controls, |ui| self.controls_ui(ui));
        self.show_controls = show_controls;
        let mut show_plots = self.show_plots;
        PLOTS_PANEL.show(ui, &mut show_plots, |ui| self.plots_ui(ui));
        self.show_plots = show_plots;
        // The plots panel's triangle floats over the canvas's corner while it's collapsed.
        let inset = if show_plots { 0.0 } else { TITLE_BAR_WIDTH };
        egui::CentralPanel::default().show(ui, |ui| self.canvas_ui(ui, inset));
    }

    /// Narrow-screen layout: one full-size view at a time, chosen in the top bar; the game's
    /// bar stays available below the canvas and the plots.
    fn compact_layout(&mut self, ui: &mut egui::Ui) {
        if self.compact_view != CompactView::Controls {
            egui::Panel::bottom("game_bar").show(ui, |ui| self.game_bar_ui(ui, true));
        }
        // No heading above the controls or plots: the selected tab already names the view.
        egui::CentralPanel::default().show(ui, |ui| match self.compact_view {
            CompactView::Canvas => self.canvas_ui(ui, 0.0),
            CompactView::Controls => self.controls_ui(ui),
            CompactView::Plots => self.plots_ui(ui),
        });
    }
}

impl eframe::App for ViewerApp {
    /// Called each time the UI needs repainting, which may be many times per second.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_world_file();
        self.gamepads.poll(ui.ctx());
        self.notifications.tick(ui.input(|i| i.time));
        match self.screen {
            Screen::Title => self.title_ui(ui),
            Screen::Flight => self.flight_ui(ui),
        }

        let ctx = ui.ctx().clone();
        self.code_window(&ctx);

        // The thrusters heard at present while the flight runs.
        let game = &self.game;
        let heard =
            (self.screen == Screen::Flight && !game.paused).then(|| (&game.replay, game.time()));
        let levels = heard.map(|(r, t)| (r.thrust_fraction(t) as f32, r.torque_fraction(t) as f32));
        self.sound.update(&ctx, levels);
    }
}

impl ViewerApp {
    /// The flight's screen: the canvas, with the controls and the plots beside it.
    fn flight_ui(&mut self, ui: &mut egui::Ui) {
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
        let dt = f64::from(ui.input(|i| i.stable_dt));
        let was_landed = self.game.landed().is_some();
        self.game.advance(dt, input, self.steering);
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
        self.refresh_prediction();

        let compact = ui.available_width() < COMPACT_WIDTH;
        // The tab bar only exists in the compact layout; the wide layout shows everything.
        if compact {
            egui::Panel::top("top_panel").show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (view, label) in [
                        (CompactView::Canvas, "View"),
                        (CompactView::Controls, CONTROLS_PANEL.title),
                        (CompactView::Plots, "Plots"),
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
}

/// The window edge a collapsible side panel is docked to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dock {
    Left,
    Right,
}

/// A side panel with a vertical title bar along its window edge: an egui window's title bar
/// turned on its side, with the collapse triangle and the panel's title. Clicking the bar
/// (or dragging the panel's edge past its minimum width) slides the panel away until only
/// the title bar is left, the way an egui window collapses to its title bar.
struct SidePanel {
    dock: Dock,
    id: &'static str,
    title: &'static str,
    /// The contents' widths, not counting the title bar.
    min_width: f32,
    default_width: f32,
}

impl SidePanel {
    fn show(
        &self,
        ui: &mut egui::Ui,
        expanded: &mut bool,
        add_contents: impl FnOnce(&mut egui::Ui),
    ) {
        let Self {
            dock,
            id,
            title,
            min_width,
            default_width,
        } = *self;
        let id = egui::Id::unique(id);
        let ctx = ui.ctx().clone();
        let openness = ctx.animate_bool(id.with("openness"), *expanded);

        // Where the title bar is when it is in the window, and where its triangle is: there
        // also while the collapsed bar has slid out, so it stays visible and clickable.
        let area = ui.available_rect_before_wrap();
        let bar = match dock {
            Dock::Left => area.with_max_x(area.left() + TITLE_BAR_WIDTH),
            Dock::Right => area.with_min_x(area.right() - TITLE_BAR_WIDTH),
        };
        let icon_center = egui::pos2(bar.center().x, title_bar_icon_y(ui, bar));

        let reveal = title_bar_reveal(ui, id, bar, *expanded);

        let mut toggled = false;
        if !*expanded {
            toggled |= egui::Area::new(id.with("collapsed_icon"))
                .order(egui::Order::Foreground)
                .fixed_pos(icon_center - egui::vec2(TITLE_BAR_WIDTH, TITLE_BAR_WIDTH) / 2.0)
                .show(&ctx, |ui| {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::Vec2::splat(TITLE_BAR_WIDTH),
                        egui::Sense::click(),
                    );
                    paint_collapse_icon(ui, &response, rect.center(), dock, openness);
                    response.on_hover_text("Show this panel").clicked()
                })
                .inner;
        }

        // Fully collapsed with the bar slid out: nothing left of the panel but that triangle.
        if *expanded || openness > 0.0 || reveal > 0.0 {
            let side = |id: egui::Id| match dock {
                Dock::Left => egui::Panel::left(id),
                Dock::Right => egui::Panel::right(id),
            };
            // No margin, so the title bar runs flush along the panel; the contents get the
            // usual one.
            let frame = egui::Frame::side_top_panel(ui.style()).inner_margin(0);
            let collapsed_panel = side(id.with("collapsed"))
                .frame(frame)
                .exact_size(TITLE_BAR_WIDTH * reveal);
            let expanded_panel = side(id)
                .frame(frame)
                .min_size(min_width + TITLE_BAR_WIDTH)
                .default_size(default_width + TITLE_BAR_WIDTH);
            let icon_in_bar = *expanded;

            toggled |= egui::Panel::show_switched(
                ui,
                expanded,
                collapsed_panel,
                expanded_panel,
                |ui, show_contents| {
                    let rect = ui.available_rect_before_wrap();
                    // The bar keeps its width as the collapsed panel narrows, so it slides out
                    // past the window edge instead of being squeezed.
                    let bar_rect = if (dock == Dock::Left) == show_contents {
                        rect.with_max_x(rect.left() + TITLE_BAR_WIDTH)
                    } else {
                        rect.with_min_x(rect.right() - TITLE_BAR_WIDTH)
                    };
                    let icon = icon_in_bar.then_some(openness);
                    let toggled = title_bar(ui, bar_rect, dock, title, icon, show_contents);
                    if show_contents {
                        let (x, content_rect) = match dock {
                            Dock::Left => (bar_rect.right(), rect.with_min_x(bar_rect.right())),
                            Dock::Right => (bar_rect.left(), rect.with_max_x(bar_rect.left())),
                        };
                        let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
                        ui.painter().vline(x, rect.y_range(), stroke);
                        let mut content =
                            ui.new_child(egui::UiBuilder::new().max_rect(content_rect));
                        egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(8, 2))
                            .show(&mut content, add_contents);
                    }
                    ui.advance_cursor_after_rect(rect);
                    toggled
                },
            )
            .inner;
        }
        if toggled {
            *expanded = !*expanded;
        }
    }
}

/// How far a side panel's title bar is in the window, from 0 (slid out) to 1. Once the
/// panel is collapsed, the bar slides out after the pointer has kept away from `bar` (where
/// the bar is when in the window) for a while, and slides back in as soon as the pointer
/// comes near.
fn title_bar_reveal(ui: &egui::Ui, id: egui::Id, bar: egui::Rect, expanded: bool) -> f32 {
    let ctx = ui.ctx();
    let now = ui.input(|i| i.time);
    let near = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| bar.expand(TITLE_BAR_REVEAL_MARGIN).contains(p));
    let last_near_id = id.with("last_near");
    if expanded || near {
        ctx.data_mut(|d| d.insert_temp(last_near_id, now));
    }
    let away = now - ctx.data_mut(|d| *d.get_temp_mut_or(last_near_id, now));
    let tucked = !expanded && away > TITLE_BAR_HIDE_DELAY;
    if !expanded && !tucked {
        ctx.request_repaint_after_secs((TITLE_BAR_HIDE_DELAY - away) as f32);
    }
    ctx.animate_bool_with_time_and_easing(
        id.with("reveal"),
        !tucked,
        TITLE_BAR_SLIDE_TIME,
        egui::emath::easing::cubic_in_out,
    )
}

/// Height of a title bar's triangle in `bar`: level with the panel contents' first row.
fn title_bar_icon_y(ui: &egui::Ui, bar: egui::Rect) -> f32 {
    bar.top() + 2.0 + ui.spacing().interact_size.y / 2.0
}

/// Paints a side panel's vertical title bar in `rect`: the collapse triangle at the top
/// (when `icon` gives its openness), then the title, reading away from the triangle toward
/// the bottom on the right and toward the top on the left, so the letters always stand on
/// the panel's contents. Returns whether the bar was clicked.
fn title_bar(
    ui: &egui::Ui,
    rect: egui::Rect,
    dock: Dock,
    title: &str,
    icon: Option<f32>,
    expanded: bool,
) -> bool {
    use std::f32::consts::TAU;

    let response = ui
        .interact(rect, ui.scope_id().with("title_bar"), egui::Sense::click())
        .on_hover_text(if expanded {
            "Hide this panel"
        } else {
            "Show this panel"
        });
    let icon_center = egui::pos2(rect.center().x, title_bar_icon_y(ui, rect));
    if let Some(openness) = icon {
        paint_collapse_icon(ui, &response, icon_center, dock, openness);
    }

    let color = ui.style().interact(&response).fg_stroke.color;
    let font = egui::TextStyle::Heading.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(title.to_owned(), font, color);
    let size = galley.size();
    let top = icon_center.y + ui.spacing().icon_width / 2.0 + ui.spacing().item_spacing.y;
    let (pos, angle) = match dock {
        Dock::Left => (
            egui::pos2(rect.center().x - size.y / 2.0, top + size.x),
            -TAU / 4.0,
        ),
        Dock::Right => (egui::pos2(rect.center().x + size.y / 2.0, top), TAU / 4.0),
    };
    ui.painter()
        .add(egui::epaint::TextShape::new(pos, galley, color).with_angle(angle));

    response.clicked()
}

/// The collapse triangle of a side panel's title bar, centered on `center`, in the colors
/// of `response`. As in `egui::collapsing_header::paint_default_icon`: a triangle pointing
/// down, turned with the openness. Here it turns half a turn instead of a quarter, from
/// pointing into the window (collapsed) to pointing at the window edge (expanded).
fn paint_collapse_icon(
    ui: &egui::Ui,
    response: &egui::Response,
    center: egui::Pos2,
    dock: Dock,
    openness: f32,
) {
    use std::f32::consts::TAU;

    let visuals = ui.style().interact(response);
    let r = egui::Rect::from_center_size(center, egui::Vec2::splat(ui.spacing().icon_width * 0.75))
        .expand(visuals.expansion);
    let (closed, open) = match dock {
        Dock::Left => (-TAU / 4.0, TAU / 4.0),
        Dock::Right => (TAU / 4.0, -TAU / 4.0),
    };
    let rotation = egui::emath::Rot2::from_angle(egui::remap(openness, 0.0..=1.0, closed..=open));
    let points = [r.left_top(), r.right_top(), r.center_bottom()]
        .map(|p| r.center() + rotation * (p - r.center()))
        .to_vec();
    ui.painter().add(egui::Shape::convex_polygon(
        points,
        visuals.fg_stroke.color,
        egui::Stroke::NONE,
    ));
}

/// A button with an outline `icon`, at the size of a button's text.
fn outline_button(ui: &mut egui::Ui, icon: &str) -> egui::Response {
    let size = egui::TextStyle::Button.resolve(ui.style()).size;
    ui.button(egui::RichText::new(icon).font(icons::outline::font(size)))
}

/// The other way of steering: the stability assist on or off.
fn toggled(steering: Steering) -> Steering {
    match steering {
        Steering::Raw => Steering::Sas,
        Steering::Sas => Steering::Raw,
    }
}

fn world(x: f64, y: f64) -> egui::Vec2 {
    egui::vec2(x as f32, y as f32)
}

/// The polyline through world points `points` on screen, as the runs of it in front of the
/// eye, each skipping points closer than `PATH_MIN_SEGMENT_PX` but its last.
fn screen_path(
    camera: &Camera,
    rect: egui::Rect,
    points: impl IntoIterator<Item = Vec3>,
) -> Vec<Vec<egui::Pos2>> {
    camera
        .project_polyline(points, rect)
        .into_iter()
        .map(|run| {
            let mut path: Vec<egui::Pos2> = Vec::with_capacity(run.len());
            for &p in &run {
                if path
                    .last()
                    .is_none_or(|q| q.distance(p) >= PATH_MIN_SEGMENT_PX)
                {
                    path.push(p);
                }
            }
            if let (Some(&p), Some(&q)) = (run.last(), path.last())
                && p != q
            {
                path.push(p);
            }
            path
        })
        .collect()
}

/// Center the camera on `bounds` (in the plane) and zoom so they fill the canvas, as seen
/// from the camera's side, with a margin.
fn fit_camera(camera: &mut Camera, bounds: ([f64; 2], [f64; 2]), rect: egui::Rect) {
    let (lo, hi) = bounds;
    camera.center = Vec3::plane(world(
        f64::midpoint(lo[0], hi[0]),
        f64::midpoint(lo[1], hi[1]),
    ));
    // The bounds' extent across and up the screen, in world units.
    let [right, up, _] = camera.orbit.basis();
    let corners = [
        [lo[0], lo[1]],
        [hi[0], lo[1]],
        [lo[0], hi[1]],
        [hi[0], hi[1]],
    ]
    .map(|[x, y]| Vec3::plane(world(x, y)));
    let extent = |axis: Vec3| {
        let along = corners.map(|c| c.dot(axis));
        along.iter().copied().fold(f32::MIN, f32::max)
            - along.iter().copied().fold(f32::MAX, f32::min)
    };
    let size = egui::vec2(extent(right), extent(up)).max(egui::vec2(1.0, 1.0));
    let usable = rect.size() * (1.0 - 2.0 * FIT_MARGIN);
    let scale = (usable.x / size.x).min(usable.y / size.y);
    if scale.is_finite() && scale > 0.0 {
        camera.set_scale(scale);
    }
}

fn make_plot(id: &str) -> Plot<'static> {
    let (x_min, x_max) = X_LABEL_SPACING;
    let (y_min, y_max) = Y_LABEL_SPACING;
    Plot::new(id.to_owned())
        .height(PLOT_HEIGHT)
        .legend(Legend::default())
        .x_grid_spacer(plot_grid)
        .y_grid_spacer(plot_grid)
        .custom_x_axes(vec![
            AxisHints::new_x()
                .label_spacing(x_min..=x_max)
                .formatter(tick_label),
        ])
        .custom_y_axes(vec![
            AxisHints::new_y()
                .label_spacing(y_min..=y_max)
                .formatter(tick_label),
        ])
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
}

/// Grid lines (and tick labels) at three steps, alternating between ones and fives: the
/// smallest of `1 * 10^k` and `5 * 10^k` at least the plot's base step apart, and the next
/// two above it (1, 5, 10 or 5, 10, 50). The labels of the finer steps fill in, dimmed,
/// between those of the round numbers.
fn plot_grid(input: GridInput) -> Vec<GridMark> {
    let base = input.base_step_size.abs();
    // A degenerate transform, as egui_plot's own spacers handle it.
    if base < f64::EPSILON {
        return Vec::new();
    }
    let decade = 10f64.powf(base.log10().floor());
    let steps = if base <= decade {
        [decade, 5.0 * decade, 10.0 * decade]
    } else if base <= 5.0 * decade {
        [5.0 * decade, 10.0 * decade, 50.0 * decade]
    } else {
        [10.0 * decade, 50.0 * decade, 100.0 * decade]
    };
    egui_plot::uniform_grid_spacer(|_| steps)(input)
}

/// A tick label, with as many decimals as its step needs: `0.5` for a step of 0.5, where
/// `egui_plot`'s own labels would round it to `0`.
fn tick_label(mark: GridMark, _range: &std::ops::RangeInclusive<f64>) -> String {
    // The epsilon keeps `log10(0.1)` from landing just below -1.
    let decimals = (-(mark.step_size.log10() + 1e-9).floor()).max(0.0) as usize;
    egui::emath::format_with_decimals_in_range(mark.value, decimals..=decimals)
}

/// A group of the User inputs panel: its name, then its rows, the inputs in a column wide
/// enough that every group's actions line up.
fn input_group(ui: &mut egui::Ui, name: &str, rows: impl FnOnce(&mut egui::Ui)) {
    const INPUTS_COLUMN_WIDTH: f32 = 130.0;
    ui.add_space(4.0);
    ui.label(egui::RichText::new(name).small().color(palette::OVERLAY1));
    egui::Grid::new(name)
        .num_columns(2)
        .min_col_width(INPUTS_COLUMN_WIDTH)
        .show(ui, rows);
}

/// What dragging the canvas does, pointing at the pan/rotate canvas button's two icons.
fn drag_action_ui(ui: &mut egui::Ui) {
    use icons::outline::{ARROW_COUNTER_CLOCKWISE, ARROWS_OUT_CARDINAL};
    use keys::Run;
    keys::label_with_icons(
        ui,
        &[
            Run::Text("Pan (when "),
            Run::Icon(ARROWS_OUT_CARDINAL),
            Run::Text(") or rotate (when "),
            Run::Icon(ARROW_COUNTER_CLOCKWISE),
            Run::Text(")"),
        ],
    );
}

/// An outline icon in the User inputs panel, as large as a gesture's.
fn outline_glyph(ui: &mut egui::Ui, glyph: &str) {
    ui.label(
        egui::RichText::new(glyph)
            .font(icons::outline::font(keys::GESTURE_GLYPH_SIZE))
            .color(palette::WHITE),
    );
}

/// A row of an [`input_group`]: the `inputs`, then what they do.
fn input_row<A, B>(
    ui: &mut egui::Ui,
    inputs: impl FnOnce(&mut egui::Ui) -> A,
    action: impl FnOnce(&mut egui::Ui) -> B,
) {
    ui.horizontal(inputs);
    action(ui);
    ui.end_row();
}

/// Plot `lines` against time with a cursor at `t`, titled "`title` `symbol` [`unit`]".
fn time_plot(
    ui: &mut egui::Ui,
    title: &str,
    unit: &str,
    symbol: &Formula,
    t: f64,
    lines: &[(&str, &Vec<PlotPoint>, egui::Color32)],
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(format!("{title} "));
        symbol.ui(ui, &egui::TextStyle::Body.resolve(ui.style()));
        ui.label(format!(" [{unit}]"));
    });
    make_plot(title).show(ui, |plot_ui| {
        for &(name, series, color) in lines {
            plot_ui.line(
                Line::new(name, series.as_slice())
                    .color(color)
                    .allow_band(false),
            );
        }
        plot_ui.vline(VLine::new("", t).color(CURSOR_COLOR));
    });
}
