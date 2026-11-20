//! The game: pick a test case and fly the ship to its goal, on a pannable, zoomable canvas,
//! next to plots of the state and control signals. An autopilot can fly it instead, with one of
//! the control policies.

use log::Level;
use nabla_core::State;
use nabla_core::problem::{Case, Outcome};
use nabla_core::solutions::{SOLUTIONS, StoredSolution};
use nabla_core::suite;
use web_time::Instant;

use crate::camera::Camera;
use crate::game::{Game, Objective, Steering};
use crate::gamepad::{self, Gamepads};
use crate::gizmo::ViewGizmo;
use crate::notifications::Notifications;
use crate::plume::Plume;
use crate::replay::Replay;
use crate::ship3d::{SHIP_GLB, ShipModel, ShipRenderer};
use crate::sound::Sound;
use crate::theme::{self, palette};
use crate::version::build_version;
use nabla_sim::span;

mod canvas;
mod flight;
mod game_bar;
mod help;
mod online;
mod orbits;
mod scene;
mod settings;
mod side_panel;
mod state_plots;
mod title;
mod worlds;
use help::HelpTab;
use side_panel::{Dock, SidePanel};
use title::{Screen, TitlePage};

const CONTROLS_PANEL: SidePanel = SidePanel {
    dock: Dock::Left,
    id: "config_panel",
    titles: &["Settings"],
    // Narrowest the contents can be dragged to; below this their rows would be cut off.
    min_width: 320.0,
    default_width: 320.0,
};
const RIGHT_PANEL: SidePanel = SidePanel {
    dock: Dock::Right,
    id: "plots_panel",
    // In the order of `RightView::ALL`.
    titles: &["State plots", "Objective"],
    min_width: 220.0,
    default_width: 300.0,
};

const DEFAULT_ALPHA: f32 = 0.3;

/// The thrust cone's default half-angle, and its range, in degrees.
const DEFAULT_AIM_CONE: f64 = 10.0;
const AIM_CONE_RANGE: std::ops::RangeInclusive<f64> = 1.0..=90.0;

/// A frame longer than this, in seconds, is logged as slow: the flight falls behind real time, as
/// the game simulates no more than this at once.
const SLOW_FRAME: f64 = 0.1;
/// Slow frames are logged at most once this often, in seconds, not to add to the slowness.
const SLOW_FRAME_LOG_PERIOD: f64 = 1.0;

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
    Objective,
}

/// What the right panel of the wide layout shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RightView {
    #[default]
    Plots,
    Objective,
}

impl RightView {
    const ALL: [Self; 2] = [Self::Plots, Self::Objective];
}

/// Case listed first in the presets.
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

pub struct ViewerApp {
    world_setup: crate::world::World,
    /// `world_setup` came from a file, rather than from a [`Preset`](crate::world::Preset).
    custom_world: bool,
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
    /// Overlay the bodies' gravity field: potential bands and contour lines.
    show_field: bool,
    /// The gravity field's contour lines' width, in points.
    field_line_width: f32,
    /// Cached contour lines of the gravity field, recomputed as the view moves.
    field_contours: Option<crate::field::Contours>,
    /// Overlay the gravity field's lines: glowing streamlines with arrowheads.
    show_streamlines: bool,
    /// The field lines of the world, traced again when it changes.
    streamlines: Option<crate::streamlines::Streamlines>,
    follow_ship: bool,
    /// Show the keyboard and mouse help over the canvas.
    show_help: bool,
    /// When the last slow frame was logged, in the UI's time.
    slow_frame_logged_at: f64,
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
    /// The tab of the User inputs panel the player picked, if any.
    help_tab: Option<HelpTab>,
    /// The scale the title screen is drawn at to fit the window, part of egui's zoom factor. 1
    /// during the flight.
    title_fit: f32,
    /// The height of what's below the title on the title screen, in points, last time it showed.
    title_rest: f32,
    /// The width of the column below the title, in points, last time it showed: wider than it
    /// was laid out at when a row in it can't wrap, as the page's buttons can't in a narrow
    /// window.
    title_wide: f32,
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
    /// Show what each control does in the controller's tab of the User inputs panel.
    show_guides: bool,
    cases: Vec<CaseEntry>,
    fit_pending: bool,
    /// Wide layout: show the left (controls) and right (plots or objective) side panels.
    show_controls: bool,
    show_plots: bool,
    right_view: RightView,
    compact_view: CompactView,
    view: CanvasView,
    /// Draws the 3D ship and its plumes; `None` where OpenGL can't (then the canvas shows the
    /// debug view).
    ship3d: Option<(ShipRenderer, Plume)>,
    /// The flight on a server, while the online screen shows.
    online: Option<crate::online::Online>,
    /// The page's address asked for a server to be connected to at once.
    connect_on_start: bool,
    /// The server's address as typed on the title screen.
    server_address: String,
    /// The password of that server, as typed when it asked for one, until the pilot goes back to
    /// the title screen.
    server_password: String,
    online_error: Option<String>,
    /// The ship the pilot asks a server for, in the lobby.
    join_request: nabla_sim::world::ShipRequest,
    /// The name the pilot asks a server for, in the lobby, as typed.
    username: String,
    /// Frame the camera around the ship, the next time the canvas shows.
    online_fit: bool,
    /// How hard the player holds thrust this frame, for the log.
    input_thrust: f64,
    /// The orbit controls have been set for the ship that is flying.
    online_ready: bool,
    /// How hard the own ship's thrusters fire, for the sound.
    online_levels: Option<(f32, f32)>,
}

impl Default for ViewerApp {
    fn default() -> Self {
        let world = crate::world::Preset::default().world();
        let case = world.case();
        let policy = &SOLUTIONS[0];
        Self {
            circumradius: world.players[0].radius,
            h: case.task.params.h,
            world_setup: world,
            custom_world: false,
            world_file: None,
            world_error: None,
            alpha: DEFAULT_ALPHA,
            camera: Camera::default(),
            gizmo: ViewGizmo::default(),
            show_gizmo: true,
            drag_orbits: false,
            show_velocity: true,
            show_acceleration: true,
            show_path: false,
            show_prediction: false,
            prediction: orbits::Forecast::default(),
            show_collision_meshes: false,
            show_grid: true,
            show_field: false,
            field_line_width: 2.0,
            field_contours: None,
            show_streamlines: false,
            streamlines: None,
            follow_ship: true,
            show_help: false,
            slow_frame_logged_at: f64::NEG_INFINITY,
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
            help_tab: None,
            title_fit: 1.0,
            title_rest: 0.0,
            title_wide: 0.0,
            setup: case,
            show_code: false,
            sound: Sound::default(),
            gamepads: Gamepads::default(),
            show_guides: true,
            cases: all_cases(),
            fit_pending: true,
            show_controls: true,
            show_plots: true,
            right_view: RightView::default(),
            compact_view: CompactView::default(),
            view: CanvasView::default(),
            ship3d: None,
            online: None,
            connect_on_start: false,
            server_address: online::DEFAULT_SERVER.to_owned(),
            server_password: String::new(),
            online_error: None,
            join_request: world_default_request(),
            username: String::new(),
            online_fit: false,
            input_thrust: 0.0,
            online_ready: false,
            online_levels: None,
        }
    }
}

/// The bundled ship, as the request the lobby starts from.
fn world_default_request() -> nabla_sim::world::ShipRequest {
    crate::world::Preset::default().world().players[0]
        .task
        .params
        .into()
}

impl ViewerApp {
    /// Called once before the first frame: a flight on the default case.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        log::info!("Nabla {}", build_version!());
        let _span = span!(Level::Debug, "app setup");
        theme::apply(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);

        let ship3d = cc
            .wgpu_render_state
            .as_ref()
            .ok_or_else(|| "not running on wgpu".to_owned())
            .and_then(|render_state| {
                let _span = span!(Level::Debug, "3D view setup");
                let model = ShipModel::load(SHIP_GLB)?;
                // egui's render pass, into which the ship is drawn: see `MULTISAMPLING`.
                let samples = if cfg!(target_arch = "wasm32") {
                    1
                } else {
                    u32::from(crate::MULTISAMPLING)
                };
                let depth = eframe::egui_wgpu::depth_format_from_bits(crate::DEPTH_BUFFER, 0);
                crate::planets::init(render_state, samples, depth);
                crate::field::init(render_state, samples, depth);
                crate::overlays::init(render_state, samples, depth);
                crate::streamlines::init(render_state, samples, depth);
                Ok((
                    ShipRenderer::new(render_state, &model, samples, depth),
                    Plume::new(render_state, samples, depth),
                ))
            })
            .inspect_err(|e| log::warn!("No 3D view: {e}"))
            .ok();
        let mut app = Self {
            ship3d,
            ..Self::default()
        };
        if let Some((server, connect)) = online::server_from_page() {
            app.server_address = server;
            app.title_page = TitlePage::Online;
            app.connect_on_start = connect;
        }
        app
    }

    /// The starting conditions, with the sampling period set by the slider.
    fn selected_case(&self) -> Case {
        let mut case = self.setup.clone();
        case.task.params.h = self.h;
        case
    }

    /// Score the policy on the starting conditions, for comparison with the flight.
    fn score_policy(&mut self) {
        let _span = span!(Level::Debug, "scoring {}", self.policy.name);
        self.policy_outcome = (self.policy.simulate)(&self.selected_case()).outcome;
        log::debug!("{}: {:?}", self.policy.name, self.policy_outcome);
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
        let _span = span!(Level::Debug, "starting a flight");
        let restarting = self.screen == Screen::Flight;
        let previous = self.game.objective();
        let engage = if restarting {
            previous == Objective::ReachTarget && self.assisted()
        } else {
            self.autopilot_on_start
        };
        self.game = Game::in_world(self.selected_case(), self.selected_world());
        log::info!("Flight started in {}", self.game.world.name);
        self.reset_orbit_controls();
        if restarting && previous == Objective::ReachTarget {
            self.choose_objective(Objective::ReachTarget);
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
            format!("The autopilot landed at the target in {n} steps ({time:.2} s)")
        } else {
            format!("Landed at the target in {n} steps ({time:.2} s)")
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

    /// Whether a frame that `took` seconds, at the UI's time `now`, is one to log as slow.
    fn slow_frame_to_log(&mut self, took: f64, now: f64) -> bool {
        let log = took > SLOW_FRAME && now - self.slow_frame_logged_at >= SLOW_FRAME_LOG_PERIOD;
        if log {
            self.slow_frame_logged_at = now;
        }
        log
    }
}

impl eframe::App for ViewerApp {
    /// Called each time the UI needs repainting, which may be many times per second.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let started = Instant::now();
        let _span = span!(Level::Trace, "frame");
        self.poll_world_file();
        self.notifications.tick(ui.input(|i| i.time));
        match self.gamepads.poll(ui.ctx()) {
            Some(gamepad::Connection::Connected(name)) => {
                log::info!("{name} connected");
                self.notifications
                    .post(format!("{name} connected"), palette::GREEN);
            }
            Some(gamepad::Connection::Disconnected(name)) => {
                log::info!("{name} disconnected");
                self.notifications
                    .post(format!("{name} disconnected"), palette::YELLOW);
            }
            None => {}
        }
        if std::mem::take(&mut self.connect_on_start) {
            let ctx = ui.ctx().clone();
            self.connect_online(&ctx);
        }
        match self.screen {
            Screen::Title => self.title_ui(ui),
            Screen::Flight => self.flight_ui(ui),
            Screen::Online => self.online_ui(ui),
        }

        let ctx = ui.ctx().clone();
        self.code_window(&ctx);

        // The thrusters heard at present while the flight runs.
        let game = &self.game;
        let heard =
            (self.screen == Screen::Flight && !game.paused).then(|| (&game.replay, game.time()));
        let levels = if self.screen == Screen::Online {
            self.online_levels
        } else {
            heard.map(|(r, t)| (r.thrust_fraction(t) as f32, r.torque_fraction(t) as f32))
        };
        // Where the sound's levels come from, for the log: an engaged autopilot flies in place of
        // the player, whose input then does nothing, and a paused flight is silent.
        let source = match self.screen {
            Screen::Title => "title screen".to_owned(),
            Screen::Flight if self.game.paused => "flight, paused".to_owned(),
            Screen::Flight | Screen::Online => {
                let place = if self.screen == Screen::Online {
                    "online flight"
                } else {
                    "flight"
                };
                if self.assisted() {
                    let objective = self.flight_objective().label();
                    format!("{place}, autopilot ({objective}); the input is ignored")
                } else if self.input_thrust > 0.0 {
                    format!("{place}, by hand, thrust held ({:.2})", self.input_thrust)
                } else {
                    format!("{place}, by hand, no thrust held")
                }
            }
        };
        self.sound.note_source(&source);
        self.sound.update(&ctx, levels);

        let took = started.elapsed().as_secs_f64();
        if self.slow_frame_to_log(took, ctx.input(|i| i.time)) {
            log::warn!(
                "Slow frame: {:.0} ms, {:?}, policy {}",
                took * 1e3,
                self.screen,
                self.policy.name
            );
        }
    }

    /// Closing the window ends the connection, so the server frees the ship at once and not after
    /// its idle timeout: eframe exits the process without dropping the app. The network's thread
    /// ends with the process, so this waits for it to say goodbye.
    fn on_exit(&mut self) {
        if let Some(mut online) = self.online.take() {
            online.leave();
            drop(online);
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::sleep(nabla_net::native::GOODBYE);
        }
    }
}

/// The other way of steering: the stability assist on or off.
fn toggled(steering: Steering) -> Steering {
    match steering {
        Steering::Raw => Steering::Sas,
        Steering::Sas => Steering::Raw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_frames_are_logged_at_most_once_a_period() {
        let mut app = ViewerApp::default();
        assert!(!app.slow_frame_to_log(SLOW_FRAME, 0.0), "not slow");
        assert!(app.slow_frame_to_log(2.0 * SLOW_FRAME, 0.0));
        assert!(!app.slow_frame_to_log(2.0 * SLOW_FRAME, 0.5), "too soon");
        assert!(app.slow_frame_to_log(2.0 * SLOW_FRAME, SLOW_FRAME_LOG_PERIOD));
    }

    #[test]
    fn a_new_step_size_restarts_the_flight_with_plots_sized_for_it() {
        let mut app = ViewerApp::default();
        for (h, steps) in [(0.1, 600), (0.01, 6000), (0.05, 1200)] {
            app.h = h;
            app.start_game();
            assert_eq!(app.game.replay.h(), h);
            assert_eq!(app.game.plots.steps(), steps);
            assert_eq!(app.game.plots.x.points().len(), 1, "only the start");
        }
    }
}
