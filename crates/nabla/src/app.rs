//! Trajectory viewer: open a solution's Rust source and pick a test case. The source is
//! compiled and run, and the run is replayed in continuous time on a pannable, zoomable canvas,
//! next to plots of the state and control signals.

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::IdSalt;
use egui_plot::{AxisHints, GridInput, GridMark, Legend, Line, Plot, PlotPoint, VLine};
use nabla_core::State;
use nabla_core::problem::{Case, Outcome, Trajectory};
use nabla_core::suite;
use nabla_solutions::db::Entry;
use nabla_solutions::registry::StoredSolution;

use crate::camera::{Camera, Vec3};
use crate::canvas_buttons::{self, Button};
use crate::formula::{self, Formula};
use crate::game::{self, Game, Input};
use crate::gizmo::ViewGizmo;
use crate::grid::{GridStyle, draw_grid};
use crate::keys;
use crate::plume::Plume;
use crate::replay::{MAX_SUBDIVISIONS, Playback, Replay, Series};
use crate::ship3d::{self, DepthMap, SHIP_GLB, ShipModel, ShipRenderer};
use crate::sound::Sound;
use crate::source::{self, Runner, SolutionSource, Status};
use crate::theme::{self, icons, palette};
use crate::timeline::{TimelineView, timeline_ui};

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

/// Fraction of the canvas left empty around the trajectory by "Fit view".
const FIT_MARGIN: f32 = 0.15;

/// How often to check the source file for changes.
const WATCH_INTERVAL: Duration = Duration::from_millis(500);

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

/// Plot series of the current replay, converted once.
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
    /// Draw the background grid.
    show_grid: bool,
    /// Show the thrusters' plumes in the 3D view.
    show_plume: bool,
    follow_ship: bool,
    /// Show the keyboard and mouse help over the canvas.
    show_help: bool,
    playback: Playback,
    /// Gameplay mode: the player's flight on the selected case, flown in place of the policy.
    game: Option<Game>,
    /// Name of the selected test case.
    case: Option<String>,
    /// Sampling period the selected case runs with, in place of its own.
    h: f64,
    /// Show the source code window.
    show_code: bool,
    /// Recompile when the source file changes on disk.
    watch_file: bool,

    source: Option<SolutionSource>,
    runner: Runner,
    sound: Sound,
    database: Vec<(Entry, &'static StoredSolution, PathBuf)>,
    cases: Vec<CaseEntry>,
    replay: Option<Replay>,
    plots: Option<Plots>,
    fit_pending: bool,
    timeline: TimelineView,
    /// Rewind and refit the view when the pending trajectory arrives.
    rewind_pending: bool,
    /// `egui` time of the last check for changes on disk.
    last_watch_check: f64,
    load_error: Option<String>,
    /// Wide layout: show the left (controls) and right (plots) side panels.
    show_controls: bool,
    show_plots: bool,
    compact_view: CompactView,
    view: CanvasView,
    /// Draws the 3D ship and its plumes; `None` where OpenGL can't (then the canvas shows the
    /// debug view).
    ship3d: Option<(ShipRenderer, Plume)>,
    /// Dropped files read by the browser.
    #[cfg(target_arch = "wasm32")]
    dropped_tx: std::sync::mpsc::Sender<DroppedFile>,
    #[cfg(target_arch = "wasm32")]
    dropped_rx: std::sync::mpsc::Receiver<DroppedFile>,
}

/// A dropped file read by the browser: `(name, contents)`.
#[cfg(target_arch = "wasm32")]
type DroppedFile = (String, Result<Vec<u8>, String>);

impl Default for ViewerApp {
    fn default() -> Self {
        #[cfg(target_arch = "wasm32")]
        let (dropped_tx, dropped_rx) = std::sync::mpsc::channel();
        Self {
            circumradius: DEFAULT_CIRCUMRADIUS,
            alpha: DEFAULT_ALPHA,
            camera: Camera::default(),
            gizmo: ViewGizmo::default(),
            show_gizmo: true,
            drag_orbits: false,
            show_velocity: true,
            show_acceleration: true,
            show_path: true,
            show_grid: true,
            show_plume: true,
            follow_ship: false,
            show_help: false,
            playback: Playback::default(),
            game: None,
            case: Some(DEFAULT_CASE.to_owned()),
            h: suite::H,
            show_code: false,
            watch_file: true,
            source: None,
            runner: Runner::default(),
            sound: Sound::default(),
            database: Vec::new(),
            cases: Vec::new(),
            replay: None,
            plots: None,
            fit_pending: false,
            timeline: TimelineView::default(),
            rewind_pending: false,
            last_watch_check: 0.0,
            load_error: None,
            show_controls: true,
            show_plots: true,
            compact_view: CompactView::default(),
            view: CanvasView::default(),
            ship3d: None,
            #[cfg(target_arch = "wasm32")]
            dropped_tx,
            #[cfg(target_arch = "wasm32")]
            dropped_rx,
        }
    }
}

impl ViewerApp {
    /// Called once before the first frame. Opens `source` if given, else the best stored one.
    pub fn new(cc: &eframe::CreationContext<'_>, source: Option<PathBuf>) -> Self {
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
                Ok((
                    ShipRenderer::new(render_state, &model, samples, depth),
                    Plume::new(render_state, samples, depth),
                ))
            })
            .inspect_err(|e| log::warn!("No 3D view: {e}"))
            .ok();
        let mut app = Self {
            runner: Runner::new(cc.egui_ctx.clone()),
            database: source::database(),
            cases: all_cases(),
            ship3d,
            ..Self::default()
        };
        match source.filter(|p| p.is_file()) {
            Some(path) => app.open_file(&path),
            None => {
                if let Some((_, solution, path)) = app.database.first().cloned() {
                    app.open_stored(solution, &path);
                }
            }
        }
        app
    }

    /// The selected case, with the sampling period set by the slider.
    fn selected_case(&self) -> Option<Case> {
        let name = self.case.as_deref()?;
        let mut case = self
            .cases
            .iter()
            .map(|e| &e.case)
            .find(|c| c.name == name)?
            .clone();
        case.task.params.h = self.h;
        Some(case)
    }

    fn open_file(&mut self, path: &Path) {
        match SolutionSource::from_file(path) {
            Ok(source) => {
                self.set_source(source, true);
            }
            Err(e) => self.load_error = Some(e),
        }
    }

    /// Open a stored solution: its file when available, else the copy embedded in the viewer.
    fn open_stored(&mut self, solution: &'static StoredSolution, path: &Path) {
        if path.is_file() {
            self.open_file(path);
        } else {
            let source = SolutionSource::from_text(solution.name, solution.source.to_owned());
            self.set_source(source, true);
        }
    }

    fn set_source(&mut self, source: SolutionSource, rewind: bool) {
        self.load_error = None;
        self.runner.set_source(&source);
        self.source = Some(source);
        self.run(rewind);
    }

    /// Run the current source on the selected case. Precompiled sources replay at once;
    /// others are compiled and traced in the background, see [`Self::poll_runner`].
    fn run(&mut self, rewind: bool) {
        let (Some(source), Some(case)) = (&self.source, self.selected_case()) else {
            return;
        };
        match self.runner.request(source, &case) {
            Some(trajectory) => self.show_trajectory(case, trajectory, rewind),
            None => self.rewind_pending |= rewind,
        }
    }

    fn poll_runner(&mut self) {
        let was_busy = self.runner.busy();
        if let Some((name, trajectory)) = self.runner.poll() {
            if let Some(case) = self.selected_case().filter(|c| c.name == name) {
                let rewind = self.rewind_pending;
                self.show_trajectory(case, trajectory, rewind);
            }
        } else if was_busy && matches!(self.runner.status, Status::Failed(_)) {
            // Do not keep showing a run of an older version of the source.
            self.replay = None;
            self.plots = None;
        }
    }

    /// Show a new run. Without `rewind`, keep the playhead time and the view, so that
    /// recompiling an edited source updates the picture in place.
    fn show_trajectory(&mut self, case: Case, trajectory: Trajectory, rewind: bool) {
        let name = self.source.as_ref().map_or("solution", |s| s.name.as_str());
        let previous_time = self.replay.as_ref().map(|r| self.playback.time(r.h()));
        let replay = Replay::new(name, case, trajectory);
        let (h, duration, last) = (replay.h(), replay.duration(), replay.last_frame());
        match previous_time {
            Some(t) if !rewind => {
                self.playback.seek_time(t, h, last);
                self.timeline.pan(0.0, duration, h);
            }
            _ => {
                self.playback.seek_start();
                self.playback.playing = false;
                self.timeline = TimelineView::full(duration, h);
                self.fit_pending = true;
            }
        }
        self.plots = Some(Plots::new(&replay));
        self.replay = Some(replay);
        self.rewind_pending = false;
    }

    /// Reload the source file when it changed on disk (checked twice a second).
    fn watch_source(&mut self, ui: &egui::Ui) {
        let Some(path) = self.source.as_ref().and_then(|s| s.path.clone()) else {
            return;
        };
        if !self.watch_file {
            return;
        }
        ui.ctx().request_repaint_after(WATCH_INTERVAL);
        let now = ui.input(|i| i.time);
        if now - self.last_watch_check < WATCH_INTERVAL.as_secs_f64() {
            return;
        }
        self.last_watch_check = now;
        if !self
            .source
            .as_ref()
            .is_some_and(SolutionSource::modified_on_disk)
        {
            return;
        }
        match SolutionSource::from_file(&path) {
            Ok(new) if self.source.as_ref().is_some_and(|old| old.hash == new.hash) => {
                self.source = Some(new); // touched, not changed
            }
            Ok(new) => self.set_source(new, false),
            Err(e) => self.load_error = Some(e),
        }
    }

    /// Open `.rs` files dropped on the window.
    #[cfg(not(target_arch = "wasm32"))]
    fn handle_dropped_files(&mut self, ui: &egui::Ui) {
        let dropped = ui.input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            self.open_file(file.path());
        }
    }

    /// Open `.rs` files dropped on the page. The browser reads them asynchronously, so the
    /// contents arrive on a later frame.
    #[cfg(target_arch = "wasm32")]
    fn handle_dropped_files(&mut self, ui: &egui::Ui) {
        let dropped = ui.input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            let tx = self.dropped_tx.clone();
            let ctx = ui.ctx().clone();
            wasm_bindgen_futures::spawn_local(async move {
                let name = file.path().display().to_string();
                // The receiver lives as long as the app.
                let _ignored = tx.send((name, file.bytes_async().await));
                ctx.request_repaint();
            });
        }
        while let Ok((name, bytes)) = self.dropped_rx.try_recv() {
            match bytes.map(String::from_utf8) {
                Ok(Ok(text)) => self.set_source(SolutionSource::from_text(&name, text), true),
                Ok(Err(_)) => self.load_error = Some(format!("{name} is not UTF-8 text")),
                Err(e) => self.load_error = Some(format!("cannot read {name}: {e}")),
            }
        }
    }

    fn last_frame(&self) -> usize {
        self.replay.as_ref().map_or(0, Replay::last_frame)
    }

    /// The run the canvas, plots and state show, and the moment of it shown as `(k, tau)`:
    /// the player's flight at present in gameplay mode, else the replay at the playhead.
    fn shown(&self) -> Option<(&Replay, usize, f64)> {
        if let Some(game) = &self.game {
            let (k, offset) = game.moment();
            return Some((&game.replay, k, offset));
        }
        let replay = self.replay.as_ref()?;
        let (k, offset) = self.playback.moment(replay.h());
        Some((replay, k, offset))
    }

    /// Sample index `k`, offset `tau` into the step, and the exact state shown.
    fn now(&self) -> Option<(usize, f64, State)> {
        let (replay, k, offset) = self.shown()?;
        Some((k, offset, replay.state_at(k, offset)))
    }

    /// Start a flight on the selected case, from its start, following the ship.
    fn start_game(&mut self) {
        let Some(case) = self.selected_case() else {
            return;
        };
        self.game = Some(Game::new(case));
        self.playback.playing = false;
        self.follow_ship = true;
        self.fit_pending = true;
        self.compact_view = CompactView::Canvas;
    }

    /// Switch to gameplay mode (a new flight), or back to replaying the policy.
    fn set_flying(&mut self, fly: bool) {
        if fly == self.game.is_some() {
            return;
        }
        if fly {
            self.start_game();
        } else {
            self.game = None;
            self.follow_ship = false;
            self.fit_pending = true;
        }
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
            }
        })
    }

    /// Space: play/pause. Left/Right: one dt. Shift+Left/Right: previous/next controller
    /// sample. Home/End: start/end. F: follow the ship or stop following it. In gameplay mode,
    /// P: pause/resume and R: restart, the other keys flying the ship (see
    /// [`Self::keyboard_input`]).
    fn handle_keys(&mut self, ui: &egui::Ui) {
        if ui.ctx().memory(|m| m.focused().is_some()) {
            return;
        }
        // Not with a modifier, which is the browser's or the system's (Cmd+F: find).
        let plain = |key| ui.input(|i| i.key_pressed(key) && i.modifiers.is_none());
        if plain(egui::Key::F) {
            self.follow_ship = !self.follow_ship;
        }
        if let Some(game) = &mut self.game {
            if plain(egui::Key::P) {
                game.paused = !game.paused;
            }
            if plain(egui::Key::R) {
                self.start_game();
            }
            return;
        }
        let last = self.last_frame();
        let (space, left, right, home, end, shift) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::Home),
                i.key_pressed(egui::Key::End),
                i.modifiers.shift,
            )
        });
        let p = &mut self.playback;
        if space {
            p.toggle(last);
        }
        match (left, right, shift) {
            (true, _, true) => p.previous_sample(last),
            (true, _, false) => p.step_ticks(-1, last),
            (_, true, true) => p.next_sample(last),
            (_, true, false) => p.step_ticks(1, last),
            _ => {}
        }
        if home {
            p.seek_start();
        }
        if end {
            p.seek_end(last);
        }
    }

    /// Replay the control policy, or fly the ship yourself on the selected case.
    fn mode_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Mode");
        let mut fly = self.game.is_some();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut fly, false, "Replay the policy")
                .on_hover_text("Watch the control policy fly the selected case");
            ui.selectable_value(&mut fly, true, "Fly it yourself")
                .on_hover_text("Fly the ship to B on the selected case, with no policy");
        });
        self.set_flying(fly);
    }

    fn source_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Control policy input");
        ui.horizontal(|ui| {
            #[cfg(not(target_arch = "wasm32"))]
            if ui.button("Open .rs...").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Rust source", &["rs"])
                    .set_directory(nabla_solutions::db::solutions_dir())
                    .pick_file()
            {
                self.open_file(&path);
            }
            let path = self.source.as_ref().and_then(|s| s.path.clone());
            if ui
                .add_enabled(path.is_some(), egui::Button::new("Reload"))
                .on_hover_text("Read the file again and recompile it if it changed")
                .clicked()
                && let Some(path) = path
            {
                self.open_file(&path);
            }
            ui.checkbox(&mut self.show_code, "Show code");
        });

        let mut stored = None;
        egui::ComboBox::from_id_salt("database")
            .selected_text("Open a stored control policy (rust code)...")
            .show_ui(ui, |ui| {
                for (entry, solution, path) in &self.database {
                    let label = format!("{}.rs  (score {})", solution.name, entry.score);
                    if ui
                        .selectable_label(false, egui::RichText::new(label).monospace())
                        .clicked()
                    {
                        stored = Some((*solution, path.clone()));
                    }
                }
            });
        if let Some((solution, path)) = stored {
            self.open_stored(solution, &path);
        }

        if let Some(source) = &self.source {
            let label = ui.label(
                egui::RichText::new(format!("{}.rs", source.name))
                    .monospace()
                    .strong(),
            );
            if let Some(path) = &source.path {
                label.on_hover_text(path.display().to_string());
            }
            if let Some(entry) = source.database_entry() {
                ui.label(format!("Stored in the database, score {}", entry.score));
            }
        }
        ui.add_enabled(
            self.source.as_ref().is_some_and(|s| s.path.is_some()),
            egui::Checkbox::new(&mut self.watch_file, "Recompile when the file is saved"),
        );
        self.status_ui(ui);
        ui.label(egui::RichText::new("Drop a .rs file on the window to open it.").weak());
    }

    fn status_ui(&self, ui: &mut egui::Ui) {
        let error_color = ui.visuals().error_fg_color;
        match &self.runner.status {
            Status::Idle => {}
            Status::Compiling => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Compiling...");
                });
            }
            Status::Tracing => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Running...");
                });
            }
            Status::Precompiled => {
                ui.label("Running the copy compiled into the viewer");
            }
            Status::Compiled => {
                ui.label("Compiled and running");
            }
            Status::Failed(message) => {
                ui.colored_label(error_color, "Failed:");
                egui::ScrollArea::both()
                    .id_salt("error_scroll")
                    .max_height(260.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(message).monospace().small())
                                .extend(),
                        );
                    });
            }
        }
        if let Some(error) = &self.load_error {
            ui.colored_label(error_color, error);
        }
    }

    fn case_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Problem input");
        let old_case = self.case.clone();
        ui.horizontal(|ui| {
            ui.label("Case:");
            egui::ComboBox::from_id_salt("case")
                .selected_text(egui::RichText::new(self.case.as_deref().unwrap_or("-")).monospace())
                .height(400.0)
                .show_ui(ui, |ui| {
                    for suite_name in ["standard", "holdout"] {
                        ui.label(egui::RichText::new(suite_name).monospace().strong());
                        for entry in self.cases.iter().filter(|e| e.suite == suite_name) {
                            let name = &entry.case.name;
                            ui.selectable_value(
                                &mut self.case,
                                Some(name.clone()),
                                egui::RichText::new(name).monospace(),
                            );
                        }
                    }
                });
        });
        if self.case != old_case {
            self.run(true);
            if self.game.is_some() {
                self.start_game();
            }
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
        h.on_hover_text("Sampling period. Double-click to reset.");
        if self.replay.as_ref().is_some_and(|r| r.h() != self.h) && !self.runner.busy() {
            // Keep the playhead time.
            self.run(false);
        }
        if self.game.as_ref().is_some_and(|g| g.replay.h() != self.h) {
            self.start_game();
        }
    }

    /// Optional window showing the source with syntax highlighting and line numbers.
    fn code_window(&mut self, ctx: &egui::Context) {
        let Some(source) = &self.source else {
            return;
        };
        let mut open = self.show_code;
        egui::Window::new(egui::RichText::new(format!("{}.rs", source.name)).monospace())
            .id(egui::Id::unique("source_code_window"))
            .open(&mut open)
            .default_size([640.0, 720.0])
            .show(ctx, |ui| {
                if let Some(path) = &source.path {
                    ui.label(
                        egui::RichText::new(path.display().to_string())
                            .weak()
                            .small(),
                    );
                    ui.separator();
                }
                egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        let lines = source.text.lines().count().max(1);
                        let numbers: Vec<String> = (1..=lines).map(|n| n.to_string()).collect();
                        let numbers = egui::RichText::new(numbers.join("\n"))
                            .monospace()
                            .color(ui.visuals().weak_text_color());
                        ui.add(egui::Label::new(numbers).selectable(false));
                        crate::highlight::code_view_ui(ui, &source.text);
                    });
                });
            });
        self.show_code = open;
    }

    fn case_info_ui(&self, ui: &mut egui::Ui) {
        let Some(case) = self.selected_case() else {
            return;
        };
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
            row(
                &formula::GOAL,
                "goal",
                format!("({:.2}, {:.2}) m", task.goal.x, task.goal.y),
            );
        });
        let Some(replay) = &self.replay else {
            return;
        };
        let outcome = match replay.outcome() {
            Outcome::Reached(n) => format!("Reached goal in {n} steps ({:.1} s)", replay.time(*n)),
            Outcome::Timeout => "Failed: step budget exhausted".to_owned(),
            Outcome::Inadmissible { step, .. } => {
                format!("Failed: inadmissible control at step {step}")
            }
            Outcome::Panic { step } => format!("Failed: panic at step {step}"),
        };
        ui.label(egui::RichText::new(outcome).strong());
    }

    fn frame_info_ui(&self, ui: &mut egui::Ui) {
        let Some((replay, k, offset)) = self.shown() else {
            return;
        };
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
            row(
                &formula::DISTANCE,
                "distance to the goal B",
                format!("{:.4} m", (s.x - goal.x).hypot(s.y - goal.y)),
            );
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
        });
    }

    fn display_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Display options");
        ui.add_enabled_ui(self.ship3d.is_some(), |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.view, CanvasView::Ship3d, "3D");
                ui.selectable_value(&mut self.view, CanvasView::Debug, "Debug")
                    .on_hover_text("Flat ship with velocity and acceleration arrows");
            });
        });
        ui.add(
            egui::Slider::new(&mut self.circumradius, 0.05..=10.0)
                .logarithmic(true)
                .text("ship size (world units)"),
        );
        ui.checkbox(&mut self.show_grid, "Grid");
        ui.checkbox(&mut self.show_path, "Path (grey: full, blue: so far)");
        ui.add_enabled_ui(!self.debug_view(), |ui| {
            ui.checkbox(&mut self.show_plume, "Thruster plumes");
        });
        ui.add_enabled_ui(self.debug_view(), |ui| {
            ui.checkbox(&mut self.show_velocity, "Velocity (green)");
            ui.checkbox(&mut self.show_acceleration, "Acceleration (red)");
        });
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

    /// The user inputs help: touch gestures on a phone or tablet, keyboard and mouse elsewhere;
    /// in gameplay mode, `flying`, the flight's controls in place of the playback's.
    fn user_inputs_ui(ui: &mut egui::Ui, flying: bool) {
        let k = keys::Keys::new(ui.ctx());
        ui.heading("User inputs");
        if k.touch() {
            Self::touch_inputs_ui(ui, flying);
        } else {
            Self::keyboard_mouse_inputs_ui(ui, k, flying);
        }
    }

    /// The flight's keys.
    fn keyboard_flight_ui(ui: &mut egui::Ui, k: keys::Keys) {
        use icons::outline::ROCKET_LAUNCH;
        use keys::{F, LEFT, P, R, Run, SPACE, caps, joiner, label_with_icons};

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
    fn keyboard_mouse_inputs_ui(ui: &mut egui::Ui, k: keys::Keys, flying: bool) {
        use icons::outline::{MOUSE_LEFT_CLICK, MOUSE_SCROLL};
        use keys::gesture;

        if flying {
            Self::keyboard_flight_ui(ui, k);
        } else {
            Self::keyboard_playback_ui(ui, k);
        }
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

    /// The playback's keys.
    fn keyboard_playback_ui(ui: &mut egui::Ui, k: keys::Keys) {
        use icons::outline::ROCKET_LAUNCH;
        use keys::{F, LEFT, RIGHT, Run, SPACE, caps, joiner, label_with_icons};

        input_group(ui, "Playback", |ui| {
            input_row(ui, |ui| caps(ui, &[SPACE]), |ui| ui.label("Play / pause"));
            input_row(
                ui,
                |ui| caps(ui, &[LEFT, RIGHT]),
                |ui| ui.label("Step one dt"),
            );
            input_row(
                ui,
                |ui| {
                    caps(ui, &[k.shift()]);
                    joiner(ui, "+");
                    caps(ui, &[LEFT, RIGHT]);
                },
                |ui| ui.label("Previous / next sample"),
            );
            input_row(
                ui,
                |ui| {
                    caps(ui, k.home());
                    joiner(ui, "/");
                    caps(ui, k.end());
                },
                |ui| ui.label("Start / end"),
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
                |ui| outline_glyph(ui, ROCKET_LAUNCH),
                |ui| ui.label("Follow the ship"),
            );
        });
    }

    /// The touch gestures, and what the playback buttons or the flight's controls do, since a
    /// touch screen shows no tooltips.
    fn touch_inputs_ui(ui: &mut egui::Ui, flying: bool) {
        use icons::outline::{ARROWS_OUT_SIMPLE, HAND_SWIPE_RIGHT, HAND_TAP, ROCKET_LAUNCH};
        use keys::{GESTURE_GLYPH_SIZE, gesture, joiner};

        let button = |ui: &mut egui::Ui, glyph: &str| {
            ui.label(
                egui::RichText::new(glyph)
                    .size(GESTURE_GLYPH_SIZE)
                    .color(palette::WHITE),
            );
        };
        if flying {
            Self::touch_flight_ui(ui);
        } else {
            input_group(ui, "Playback", |ui| {
                input_row(
                    ui,
                    |ui| button(ui, icons::PLAY),
                    |ui| ui.label("Play / pause"),
                );
                input_row(
                    ui,
                    |ui| {
                        button(ui, icons::CARET_LEFT);
                        joiner(ui, "/");
                        button(ui, icons::CARET_RIGHT);
                    },
                    |ui| ui.label("Previous / next sample"),
                );
                input_row(
                    ui,
                    |ui| {
                        button(ui, icons::SKIP_BACK);
                        joiner(ui, "/");
                        button(ui, icons::SKIP_FORWARD);
                    },
                    |ui| ui.label("Start / end"),
                );
                input_row(
                    ui,
                    |ui| outline_glyph(ui, ROCKET_LAUNCH),
                    |ui| ui.label("Follow the ship"),
                );
            });
        }
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
        if flying {
            return;
        }
        input_group(ui, "Timeline", |ui| {
            input_row(
                ui,
                |ui| gesture(ui, HAND_TAP, "tap / drag"),
                |ui| ui.label("Seek"),
            );
            input_row(
                ui,
                |ui| gesture(ui, ARROWS_OUT_SIMPLE, "pinch"),
                |ui| ui.label("Zoom"),
            );
            input_row(
                ui,
                |ui| gesture(ui, HAND_TAP, "double-tap"),
                |ui| ui.label("Show the whole run"),
            );
        });
    }

    /// The playback controls, on one row, and the timeline. `compact`, for narrow screens,
    /// keeps only the transport buttons and a playback speed menu.
    fn timeline_ui(&mut self, ui: &mut egui::Ui, compact: bool) {
        let Some(replay) = &self.replay else {
            ui.label("Select a solution and a case.");
            return;
        };
        let (h, last) = (replay.h(), replay.last_frame());
        let playback = &mut self.playback;
        let k = keys::Keys::new(ui.ctx());
        ui.horizontal_wrapped(|ui| {
            use keys::{LEFT, RIGHT, SPACE};

            if ui
                .button(icons::SKIP_BACK)
                .on_hover_ui(|ui| keys::tooltip(ui, "Start", k.home()))
                .clicked()
            {
                playback.seek_start();
            }
            if ui
                .button(icons::CARET_LEFT)
                .on_hover_ui(|ui| {
                    keys::tooltip(ui, "Previous controller sample", &[k.shift(), LEFT]);
                })
                .clicked()
            {
                playback.previous_sample(last);
            }
            let play = if playback.playing {
                icons::PAUSE
            } else {
                icons::PLAY
            };
            if ui
                .button(play)
                .on_hover_ui(|ui| keys::tooltip(ui, "Play / pause", &[SPACE]))
                .clicked()
            {
                playback.toggle(last);
            }
            if ui
                .button(icons::CARET_RIGHT)
                .on_hover_ui(|ui| {
                    keys::tooltip(ui, "Next controller sample", &[k.shift(), RIGHT]);
                })
                .clicked()
            {
                playback.next_sample(last);
            }
            if ui
                .button(icons::SKIP_FORWARD)
                .on_hover_ui(|ui| keys::tooltip(ui, "End", k.end()))
                .clicked()
            {
                playback.seek_end(last);
            }
            ui.separator();
            if compact {
                speed_menu(ui, &mut playback.speed);
                return;
            }
            ui.add(
                egui::Slider::new(&mut playback.speed, 0.1..=50.0)
                    .logarithmic(true)
                    .text("playback speed"),
            );
            ui.separator();
            let mut dt = playback.dt(h);
            let response = ui
                .add(
                    egui::Slider::new(&mut dt, h / f64::from(MAX_SUBDIVISIONS)..=h)
                        .logarithmic(true)
                        .text(
                            egui::RichText::new(format!("dt [s] = h/{}", playback.subdivisions()))
                                .monospace(),
                        ),
                )
                .on_hover_text(
                    "Display sampling time: the playhead moves in steps of dt (Left/Right).\n\
                     dt = h/n, so every controller sample lies on the grid.\n\
                     States between samples are exact, not interpolated.",
                );
            if response.changed() {
                playback.set_subdivisions((h / dt).round() as u32);
            }
            ui.separator();
            ui.monospace(format!(
                "{} on {}: t = {:.3} / {:.1} s",
                replay.solution,
                replay.case.name,
                playback.time(h),
                replay.duration()
            ));
        });
        let t_now = playback.time(h);
        if let Some(t) = timeline_ui(ui, &mut self.timeline, replay.duration(), h, t_now) {
            playback.playing = false;
            playback.seek_time(t, h, last);
        }
    }

    /// Gameplay mode's bar, in place of the playback controls and the timeline: pause, restart
    /// and exit, and the time.
    fn game_bar_ui(&mut self, ui: &mut egui::Ui) {
        let Some(game) = &mut self.game else {
            return;
        };
        let (mut restart, mut exit) = (false, false);
        ui.horizontal(|ui| {
            let pause = if game.paused {
                icons::PLAY
            } else {
                icons::PAUSE
            };
            if ui
                .button(pause)
                .on_hover_ui(|ui| keys::tooltip(ui, "Pause / resume", &[keys::P]))
                .clicked()
            {
                game.paused = !game.paused;
            }
            let size = egui::TextStyle::Button.resolve(ui.style()).size;
            let restart_icon = egui::RichText::new(icons::outline::ARROW_CLOCKWISE)
                .font(icons::outline::font(size));
            restart = ui
                .button(restart_icon)
                .on_hover_ui(|ui| keys::tooltip(ui, "Restart", &[keys::R]))
                .clicked();
            exit = ui
                .button("Exit")
                .on_hover_text("Back to replaying the control policy")
                .clicked();
            ui.separator();
            let (k, _) = game.moment();
            ui.monospace(format!("t = {:.2} s, k = {k}", game.time()));
        });
        if restart {
            self.start_game();
        }
        if exit {
            self.set_flying(false);
        }
    }

    /// The plots of the shown run: the replay, or the flight so far, which grows every frame.
    fn plots_ui(&mut self, ui: &mut egui::Ui) {
        let flight;
        let (replay, plots, t) = if let Some(game) = &self.game {
            flight = Plots::new(&game.replay);
            (&game.replay, &flight, game.time())
        } else {
            let (Some(replay), Some(plots)) = (&self.replay, &self.plots) else {
                return;
            };
            (replay, plots, self.playback.time(replay.h()))
        };
        ui.heading("State plots");
        let seek = egui::ScrollArea::vertical()
            .show(ui, |ui| {
                [
                    time_plot(
                        ui,
                        "Position",
                        "m",
                        &formula::POSITION,
                        t,
                        &[("x", &plots.x, POS_X_COLOR), ("y", &plots.y, POS_Y_COLOR)],
                    ),
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
                    ),
                    time_plot(
                        ui,
                        "Heading angle",
                        "rad",
                        &formula::THETA,
                        t,
                        &[("theta", &plots.theta, THETA_COLOR)],
                    ),
                    time_plot(
                        ui,
                        "Angular velocity",
                        "rad/s",
                        &formula::OMEGA,
                        t,
                        &[("omega", &plots.omega, OMEGA_COLOR)],
                    ),
                    time_plot(
                        ui,
                        "Thrust",
                        "N",
                        &formula::THRUST,
                        t,
                        &[("F", &plots.thrust, THRUST_COLOR)],
                    ),
                    time_plot(
                        ui,
                        "Torque",
                        "N m",
                        &formula::TORQUE,
                        t,
                        &[("tau", &plots.torque, TORQUE_COLOR)],
                    ),
                ]
                .into_iter()
                .flatten()
                .next()
            })
            .inner;
        // A flight cannot be seeked: its time runs freely.
        if let Some(t) = seek
            && self.game.is_none()
        {
            self.playback.playing = false;
            self.playback.seek_time(t, replay.h(), replay.last_frame());
        }
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
        let on_pads = self.game.as_mut().is_some_and(|game| {
            let visible = game.pads.visible(touch_device);
            game.pads.update(ui, rect, visible);
            game.pads.touching()
        });

        // Primary drag orbits the camera or pans, as the pan/rotate button says. Panning is
        // disabled while following the ship, since the center is locked to the ship's
        // position each frame.
        if response.dragged_by(egui::PointerButton::Primary) {
            if self.drag_orbits {
                self.gizmo.orbit(&mut self.camera, response.drag_delta());
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if !self.follow_ship {
                self.camera.pan_by_screen(response.drag_delta());
            }
        }

        // Zoom with scroll/pinch while the cursor is over the canvas. A finger on a pad and
        // another on the canvas are not a pinch.
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

        if self.fit_pending {
            let bounds = match &self.game {
                Some(game) => Some(game.view_bounds()),
                None => self.replay.as_ref().map(Replay::bounds),
            };
            if let Some(bounds) = bounds {
                fit_camera(&mut self.camera, bounds, rect);
            }
            self.fit_pending = false;
        }

        let painter = ui.painter_at(rect);
        self.draw_scene(ui, &painter, rect);
        self.draw_game_overlay(&painter, rect, touch_device);
        if self.show_gizmo {
            self.gizmo.paint(ui, &painter, corner, &self.camera);
        }
        let mut fly = self.game.is_some();
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
                    icons::outline::GAME_CONTROLLER,
                    &mut fly,
                    "Fly it yourself: steer the ship to B with the keyboard or touch",
                )
                .color(palette::GREEN),
                Button::toggle(
                    icons::outline::SEAL_QUESTION,
                    &mut self.show_help,
                    "Keyboard and mouse help",
                )
                .color(palette::YELLOW),
            ],
        );
        self.set_flying(fly);
        if self.show_help {
            self.help_ui(ui, rect, buttons);
        }
    }

    /// The keyboard and mouse help, over the `canvas`, left of its `buttons` and level with
    /// their top. Where the canvas is too small for it, it shrinks, keeping its proportions, so
    /// it always shows whole. Escape closes it, as does its button.
    fn help_ui(&mut self, ui: &egui::Ui, canvas: egui::Rect, buttons: egui::Rect) {
        const GAP: f32 = 8.0;
        let flying = self.game.is_some();
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
                egui::Frame::popup(ui.style()).show(ui, |ui| Self::user_inputs_ui(ui, flying));
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
        let Some((_, _, state)) = self.now() else {
            if self.show_grid {
                draw_grid(painter, &self.camera, rect, &style);
            }
            return;
        };

        // Follow mode: keep the ship centered. Done after pan/zoom so zoom
        // still adjusts scale around the (now ship-centered) view.
        if self.follow_ship {
            self.camera.center = Vec3::plane(world(state.x, state.y));
        }

        if self.show_grid {
            draw_grid(painter, &self.camera, rect, &style);
        }
        let Some((replay, k, offset)) = self.shown() else {
            return;
        };
        if self.show_path {
            self.draw_path(painter, rect, replay, k, &state);
        }
        self.draw_goal(painter, rect, replay, &state);
        match &self.ship3d {
            Some((renderer, plume)) if !debug => {
                let position = world(state.x, state.y);
                if let Some(callback) = ship3d::paint_callback(
                    renderer,
                    &self.camera,
                    rect,
                    position,
                    state.theta as f32,
                    self.circumradius,
                ) {
                    painter.add(callback);
                }
                // After the ship, whose depth they are tested against.
                if self.show_plume {
                    let t = replay.time(k) + offset;
                    let depth =
                        DepthMap::new(renderer, &self.camera, rect, position, self.circumradius);
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
    /// show), and the banner.
    fn draw_game_overlay(&self, painter: &egui::Painter, rect: egui::Rect, touch_device: bool) {
        let Some(game) = &self.game else {
            return;
        };
        let pads = game.pads.visible(touch_device);
        // Above the pads, where they show.
        let marker_area = if pads {
            rect.with_max_y(rect.bottom() - game::PADS_HEIGHT)
        } else {
            rect
        };
        self.draw_goal_marker(painter, rect, marker_area, &game.replay, &game.state());
        if pads {
            game.pads.paint(painter, rect);
        }
        // The policy's run on the same case, for comparison.
        let policy = self
            .replay
            .as_ref()
            .filter(|r| r.case.name == game.replay.case.name)
            .and_then(|r| Some((r.solution.as_str(), r.outcome().steps()?)));
        game_banner(painter, rect, game, policy);
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
            arrow(
                replay.acceleration(k, state),
                ACCELERATION_ARROW_PX_PER_UPS2,
                ACCELERATION_COLOR,
            );
        }
    }
}

impl ViewerApp {
    /// The left panel's content: source, case, current state and display options.
    fn controls_ui(&mut self, ui: &mut egui::Ui) {
        // Scrolls sideways too, so rows are never cut off if the space is narrower than they are.
        egui::ScrollArea::both().show(ui, |ui| {
            self.mode_ui(ui);
            ui.separator();
            self.source_ui(ui);
            ui.separator();
            self.case_ui(ui);
            self.case_info_ui(ui);
            ui.separator();
            self.frame_info_ui(ui);
            ui.separator();
            self.display_ui(ui);
            ui.separator();
            self.sound.ui(ui);
        });
    }

    /// The bar along the bottom: the game's in gameplay mode, else the playback controls and
    /// the timeline.
    fn bottom_bar_ui(&mut self, ui: &mut egui::Ui, compact: bool) {
        if self.game.is_some() {
            self.game_bar_ui(ui);
        } else {
            self.timeline_ui(ui, compact);
        }
    }

    /// Desktop layout: the timeline along the bottom, controls left and plots right above it.
    fn wide_layout(&mut self, ui: &mut egui::Ui) {
        // The timeline first, across the whole window, so neither side panel (nor its
        // collapsed title bar, which slides back in when the pointer comes near) reaches it.
        egui::Panel::bottom("timeline_panel").show(ui, |ui| self.bottom_bar_ui(ui, false));
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

    /// Narrow-screen layout: one full-size view at a time, chosen in the top bar; the
    /// timeline stays available below the canvas and the plots.
    fn compact_layout(&mut self, ui: &mut egui::Ui) {
        if self.compact_view != CompactView::Controls {
            egui::Panel::bottom("timeline_panel").show(ui, |ui| self.bottom_bar_ui(ui, true));
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
        self.poll_runner();
        self.handle_dropped_files(ui);
        self.watch_source(ui);

        // Advance playback first, so every panel shows the same instant.
        self.handle_keys(ui);
        if let Some(game) = &mut self.game {
            let input = Self::keyboard_input(ui).or(game.pads.held());
            game.advance(f64::from(ui.input(|i| i.stable_dt)), input);
            if !game.paused {
                ui.ctx().request_repaint();
            }
        } else if let Some(replay) = &self.replay {
            let (h, duration) = (replay.h(), replay.duration());
            let dt = f64::from(ui.input(|i| i.stable_dt));
            self.playback.advance(dt, h, replay.last_frame());
            if self.playback.playing {
                self.timeline.follow(self.playback.time(h), duration, h);
            }
        }
        if self.playback.playing {
            ui.ctx().request_repaint();
        }

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

        let ctx = ui.ctx().clone();
        self.code_window(&ctx);

        // The thrusters heard at the playhead while the replay plays, or at present while the
        // flight runs.
        let heard = match &self.game {
            Some(game) => (!game.paused).then(|| (&game.replay, game.time())),
            None => self
                .replay
                .as_ref()
                .filter(|_| self.playback.playing)
                .map(|r| (r, self.playback.time(r.h()))),
        };
        let levels = heard.map(|(r, t)| (r.thrust_fraction(t) as f32, r.torque_fraction(t) as f32));
        self.sound.update(&ctx, levels);
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

/// A banner along the top of the canvas while the flight is paused or once it has landed, with
/// the steps the `policy` (its name and step count) takes on the same case, if known.
fn game_banner(
    painter: &egui::Painter,
    canvas: egui::Rect,
    game: &Game,
    policy: Option<(&str, usize)>,
) {
    const TOP: f32 = 16.0;
    let (text, color) = match game.landed() {
        Some(n) => {
            let mut text = format!("Landed at B in {n} steps ({:.2} s)", game.replay.time(n));
            if let Some((name, m)) = policy {
                text += &format!("\n{name} takes {m}");
            }
            (text, palette::GREEN)
        }
        None if game.paused => ("Paused".to_owned(), palette::WHITE),
        None => return,
    };
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(18.0), color);
    let rect = egui::Rect::from_center_size(
        egui::pos2(
            canvas.center().x,
            canvas.top() + TOP + galley.size().y / 2.0 + 6.0,
        ),
        galley.size() + egui::vec2(24.0, 12.0),
    );
    painter.rect_filled(
        rect,
        rect.height() / 2.0,
        egui::Color32::from_rgba_unmultiplied(30, 30, 30, 220),
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
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
        camera.scale = scale;
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

/// Playback speeds offered by the compact layout's speed menu.
const MENU_SPEEDS: [f64; 10] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 4.0, 10.0];

/// A playback speed as the menu shows it: `1x`, `1.5x`, `0.25x`.
fn speed_label(speed: f64) -> String {
    format!("{}x", (speed * 100.0).round() / 100.0)
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

/// The compact layout's playback speed control, as in video players: a button with a gauge
/// and the speed, opening a menu of speeds.
fn speed_menu(ui: &mut egui::Ui, speed: &mut f64) {
    let gauge = IdSalt::new("speed_gauge");
    let size = egui::Vec2::splat(ui.text_style_height(&egui::TextStyle::Button));
    let button =
        egui::Button::new((egui::Atom::custom(gauge, size), speed_label(*speed))).atom_ui(ui);
    if let Some(rect) = button.rect(gauge) {
        let color = ui.style().interact(&button.response).fg_stroke.color;
        paint_gauge(ui.painter(), rect, color);
    }
    let response = button.response.on_hover_text("Playback speed");
    egui::Popup::menu(&response).show(|ui| {
        for option in MENU_SPEEDS {
            let selected = (*speed - option).abs() < 1e-9;
            if ui.selectable_label(selected, speed_label(option)).clicked() {
                *speed = option;
            }
        }
    });
}

/// A speedometer: an arc over the top of `rect` and a needle pointing up and to the right.
fn paint_gauge(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    use std::f32::consts::PI;
    let stroke = egui::Stroke::new(1.5, color);
    let radius = 0.45 * rect.width();
    let center = rect.center() + egui::vec2(0.0, 0.2 * radius);
    let point = |angle: f32, r: f32| center + r * egui::vec2(angle.cos(), -angle.sin());
    // From lower left (210 degrees) over the top to lower right (-30 degrees).
    let arc = (0..=16)
        .map(|i| point(PI * (7.0 / 6.0 - 4.0 / 3.0 * i as f32 / 16.0), radius))
        .collect();
    painter.add(egui::Shape::line(arc, stroke));
    painter.line_segment([center, point(PI / 4.0, 0.75 * radius)], stroke);
    painter.circle_filled(center, 1.5, color);
}

/// Plot `lines` against time with a cursor at `t`, titled "`title` `symbol` [`unit`]". Returns
/// the clicked time, if any.
fn time_plot(
    ui: &mut egui::Ui,
    title: &str,
    unit: &str,
    symbol: &Formula,
    t: f64,
    lines: &[(&str, &Vec<PlotPoint>, egui::Color32)],
) -> Option<f64> {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(format!("{title} "));
        symbol.ui(ui, &egui::TextStyle::Body.resolve(ui.style()));
        ui.label(format!(" [{unit}]"));
    });
    make_plot(title)
        .show(ui, |plot_ui| {
            for &(name, series, color) in lines {
                plot_ui.line(
                    Line::new(name, series.as_slice())
                        .color(color)
                        .allow_band(false),
                );
            }
            plot_ui.vline(VLine::new("", t).color(CURSOR_COLOR));
            let clicked = plot_ui.response().clicked() || plot_ui.response().dragged();
            clicked
                .then(|| plot_ui.pointer_coordinate())
                .flatten()
                .map(|p| p.x)
        })
        .inner
}
