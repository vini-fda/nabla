//! Trajectory viewer: open a solution's Rust source and pick a test case. The source is
//! compiled and run, and the run is replayed in continuous time on a pannable, zoomable canvas,
//! next to plots of the state and control signals.

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui_plot::{Legend, Line, Plot, PlotPoint, VLine};
use ship2d_optimization::State;
use ship2d_optimization::db::Entry;
use ship2d_optimization::problem::{Case, Outcome, Trajectory};
use ship2d_optimization::registry::StoredSolution;
use ship2d_optimization::suite;

use crate::camera::Camera;
use crate::grid::{GridStyle, draw_grid};
use crate::replay::{MAX_SUBDIVISIONS, Playback, Replay, Series};
use crate::source::{self, Runner, SolutionSource, Status};
use crate::theme::{self, palette};
use crate::timeline::{TimelineView, timeline_ui};

const TRIANGLE_STROKE_WIDTH: f32 = 2.0;

const PLOT_HEIGHT: f32 = 110.0;
const PLOT_PANEL_DEFAULT_WIDTH: f32 = 300.0;
const PLOT_PANEL_MIN_WIDTH: f32 = 220.0;

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

/// Case selected when nothing was persisted.
const DEFAULT_CASE: &str = "rest_facing_away";

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
    standard.chain(holdout).collect()
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

/// We derive Deserialize/Serialize so we can persist app state on shutdown.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)] // if we add new fields, give them default values when deserializing old state
pub struct ViewerApp {
    circumradius: f32,
    alpha: f32,
    camera: Camera,
    show_velocity: bool,
    show_acceleration: bool,
    show_path: bool,
    follow_ship: bool,
    playback: Playback,
    /// The solution source file.
    source_path: Option<PathBuf>,
    /// Name of the selected test case.
    case: Option<String>,
    /// Show the source code window.
    show_code: bool,
    /// Recompile when the source file changes on disk.
    watch_file: bool,

    #[serde(skip)]
    source: Option<SolutionSource>,
    #[serde(skip)]
    runner: Runner,
    #[serde(skip)]
    database: Vec<(Entry, &'static StoredSolution, PathBuf)>,
    #[serde(skip)]
    cases: Vec<CaseEntry>,
    #[serde(skip)]
    replay: Option<Replay>,
    #[serde(skip)]
    plots: Option<Plots>,
    #[serde(skip)]
    fit_pending: bool,
    #[serde(skip)]
    timeline: TimelineView,
    /// Rewind and refit the view when the pending trajectory arrives.
    #[serde(skip)]
    rewind_pending: bool,
    /// `egui` time of the last check for changes on disk.
    #[serde(skip)]
    last_watch_check: f64,
    #[serde(skip)]
    load_error: Option<String>,
    #[serde(skip)]
    compact_view: CompactView,
}

impl Default for ViewerApp {
    fn default() -> Self {
        Self {
            circumradius: DEFAULT_CIRCUMRADIUS,
            alpha: DEFAULT_ALPHA,
            camera: Camera::default(),
            show_velocity: true,
            show_acceleration: true,
            show_path: true,
            follow_ship: false,
            playback: Playback::default(),
            source_path: None,
            case: None,
            show_code: false,
            watch_file: true,
            source: None,
            runner: Runner::default(),
            database: Vec::new(),
            cases: Vec::new(),
            replay: None,
            plots: None,
            fit_pending: false,
            timeline: TimelineView::default(),
            rewind_pending: false,
            last_watch_check: 0.0,
            load_error: None,
            compact_view: CompactView::default(),
        }
    }
}

impl ViewerApp {
    /// Called once before the first frame. Opens `source` if given, else the last source.
    pub fn new(cc: &eframe::CreationContext<'_>, source: Option<PathBuf>) -> Self {
        theme::apply(&cc.egui_ctx);

        // Load previous app state (if any).
        // Note that you must enable the `persistence` feature for this to work.
        let mut app: Self = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, eframe::APP_KEY))
            .unwrap_or_default();
        app.runner = Runner::new(cc.egui_ctx.clone());
        app.database = source::database();
        app.cases = all_cases();

        // Fall back to defaults when the persisted choices no longer exist.
        let case_ok = app
            .case
            .as_deref()
            .is_some_and(|name| app.cases.iter().any(|e| e.case.name == name));
        if !case_ok {
            app.case = Some(DEFAULT_CASE.to_owned());
        }
        match source
            .or_else(|| app.source_path.clone())
            .filter(|p| p.is_file())
        {
            Some(path) => app.open_file(&path),
            None => {
                if let Some((_, solution, path)) = app.database.first().cloned() {
                    app.open_stored(solution, &path);
                }
            }
        }
        app
    }

    fn selected_case(&self) -> Option<&Case> {
        let name = self.case.as_deref()?;
        self.cases.iter().map(|e| &e.case).find(|c| c.name == name)
    }

    fn open_file(&mut self, path: &Path) {
        match SolutionSource::from_file(path) {
            Ok(source) => {
                self.source_path = Some(path.to_owned());
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
        let (Some(source), Some(case)) = (&self.source, self.selected_case().cloned()) else {
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
            if let Some(case) = self.selected_case().filter(|c| c.name == name).cloned() {
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
    fn handle_dropped_files(&mut self, ui: &egui::Ui) {
        let dropped = ui.input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            if let Some(path) = &file.path {
                self.open_file(path);
            } else if let Some(bytes) = &file.bytes {
                match String::from_utf8(bytes.to_vec()) {
                    Ok(text) => self.set_source(SolutionSource::from_text(&file.name, text), true),
                    Err(_) => self.load_error = Some(format!("{} is not UTF-8 text", file.name)),
                }
            }
        }
    }

    fn last_frame(&self) -> usize {
        self.replay.as_ref().map_or(0, Replay::last_frame)
    }

    /// Sample index `k`, offset `tau` into the step, and the exact state at the playhead.
    fn now(&self) -> Option<(usize, f64, State)> {
        let replay = self.replay.as_ref()?;
        let (k, offset) = self.playback.moment(replay.h());
        Some((k, offset, replay.state_at(k, offset)))
    }

    /// Space: play/pause. Left/Right: one dt. Shift+Left/Right: previous/next controller
    /// sample. Home/End: start/end.
    fn handle_keys(&mut self, ui: &egui::Ui) {
        if ui.ctx().memory(|m| m.focused().is_some()) {
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

    fn source_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Solution source");
        ui.horizontal(|ui| {
            #[cfg(not(target_arch = "wasm32"))]
            if ui.button("Open .rs...").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("Rust source", &["rs"])
                    .set_directory(ship2d_optimization::db::solutions_dir())
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
            .selected_text("Open a stored solution...")
            .show_ui(ui, |ui| {
                for (entry, solution, path) in &self.database {
                    let label = format!("{}.rs  (score {})", solution.name, entry.score);
                    if ui.selectable_label(false, label).clicked() {
                        stored = Some((*solution, path.clone()));
                    }
                }
            });
        if let Some((solution, path)) = stored {
            self.open_stored(solution, &path);
        }

        if let Some(source) = &self.source {
            let label = ui.label(egui::RichText::new(format!("{}.rs", source.name)).strong());
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
        ui.heading("Case");
        let old_case = self.case.clone();
        egui::ComboBox::from_id_salt("case")
            .selected_text(self.case.as_deref().unwrap_or("-"))
            .height(400.0)
            .show_ui(ui, |ui| {
                for suite_name in ["standard", "holdout"] {
                    ui.label(egui::RichText::new(suite_name).strong());
                    for entry in self.cases.iter().filter(|e| e.suite == suite_name) {
                        let name = &entry.case.name;
                        ui.selectable_value(&mut self.case, Some(name.clone()), name);
                    }
                }
            });
        if self.case != old_case {
            self.run(true);
        }
    }

    /// Optional window showing the source with syntax highlighting and line numbers.
    fn code_window(&mut self, ctx: &egui::Context) {
        let Some(source) = &self.source else {
            return;
        };
        let mut open = self.show_code;
        egui::Window::new(format!("{}.rs", source.name))
            .id(egui::Id::new("source_code_window"))
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
                let theme =
                    egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());
                egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        let lines = source.text.lines().count().max(1);
                        let numbers: Vec<String> = (1..=lines).map(|n| n.to_string()).collect();
                        let numbers = egui::RichText::new(numbers.join("\n"))
                            .monospace()
                            .color(ui.visuals().weak_text_color());
                        ui.add(egui::Label::new(numbers).selectable(false));
                        egui_extras::syntax_highlighting::code_view_ui(
                            ui,
                            &theme,
                            &source.text,
                            "rs",
                        );
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
            let mut row = |name: &str, value: String| {
                ui.label(name);
                ui.monospace(value);
                ui.end_row();
            };
            row("m, I", format!("{:.3}, {:.3}", p.mass, p.inertia));
            row(
                "F_max, tau_max",
                format!("{:.3}, {:.3}", p.f_max, p.tau_max),
            );
            row("h", format!("{} s", p.h));
            row(
                "goal B",
                format!("({:.2}, {:.2})", task.goal.x, task.goal.y),
            );
            row("start p", format!("({:.2}, {:.2})", s.x, s.y));
            row("start v", format!("({:.2}, {:.2})", s.vx, s.vy));
            row(
                "start theta, omega",
                format!("{:.2}, {:.2}", s.theta, s.omega),
            );
        });
        let Some(replay) = &self.replay else {
            return;
        };
        let outcome = match replay.outcome() {
            Outcome::Reached(n) => format!("Reached in {n} steps ({:.1} s)", replay.time(*n)),
            Outcome::Timeout => "Failed: step budget exhausted".to_owned(),
            Outcome::Inadmissible { step, .. } => {
                format!("Failed: inadmissible control at step {step}")
            }
            Outcome::Panic { step } => format!("Failed: panic at step {step}"),
        };
        ui.label(egui::RichText::new(outcome).strong());
    }

    fn frame_info_ui(&self, ui: &mut egui::Ui) {
        let (Some(replay), Some((k, offset, s))) = (&self.replay, self.now()) else {
            return;
        };
        let goal = replay.case.task.goal;
        ui.heading(format!("t = {:.4} s", replay.time(k) + offset));
        ui.label(if self.playback.on_sample() {
            format!("at controller sample k = {k}")
        } else {
            format!("sample k = {k} + {offset:.4} s")
        });
        egui::Grid::new("frame_info").num_columns(2).show(ui, |ui| {
            let mut row = |name: &str, value: String| {
                ui.label(name);
                ui.monospace(value);
                ui.end_row();
            };
            row("p", format!("({:.3}, {:.3})", s.x, s.y));
            row("v", format!("({:.3}, {:.3})", s.vx, s.vy));
            row("theta, omega", format!("{:.3}, {:.3}", s.theta, s.omega));
            row(
                "dist. to B",
                format!("{:.4}", (s.x - goal.x).hypot(s.y - goal.y)),
            );
            row("speed", format!("{:.4}", s.vx.hypot(s.vy)));
            let control = replay.control(k).map_or_else(
                || "- (final state)".to_owned(),
                |u| format!("{:.3}, {:.3}", u.thrust, u.torque),
            );
            row("u_k = (F, tau)", control);
        });
    }

    fn display_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Display");
        ui.add(
            egui::Slider::new(&mut self.circumradius, 0.05..=10.0)
                .logarithmic(true)
                .text("ship size (world units)"),
        );
        ui.add(
            egui::Slider::new(&mut self.alpha, 0.0..=std::f32::consts::PI).text("ship shape alpha"),
        );
        ui.checkbox(&mut self.show_path, "Path (grey: full, blue: so far)");
        ui.checkbox(&mut self.show_velocity, "Velocity (green)");
        ui.checkbox(&mut self.show_acceleration, "Acceleration (red)");
        ui.checkbox(&mut self.follow_ship, "Follow ship");
        ui.horizontal(|ui| {
            if ui.button("Fit view").clicked() {
                self.fit_pending = true;
            }
            if ui.button("Reset view").clicked() {
                self.camera = Camera::default();
            }
        });
        ui.label("Drag: pan   Scroll: zoom");
        ui.label("Space: play/pause   Left / Right: +/-dt");
        ui.label("Shift+Left / Right: +/-sample   Home / End");
    }

    fn timeline_ui(&mut self, ui: &mut egui::Ui) {
        let Some(replay) = &self.replay else {
            ui.label("Select a solution and a case.");
            return;
        };
        let (h, last) = (replay.h(), replay.last_frame());
        let playback = &mut self.playback;
        ui.horizontal_wrapped(|ui| {
            if ui.button("|<<").on_hover_text("Start (Home)").clicked() {
                playback.seek_start();
            }
            if ui
                .button("|<")
                .on_hover_text("Previous controller sample (Shift+Left)")
                .clicked()
            {
                playback.previous_sample(last);
            }
            let play = if playback.playing { "Pause" } else { "Play" };
            if ui
                .button(play)
                .on_hover_text("Play / pause (Space)")
                .clicked()
            {
                playback.toggle(last);
            }
            if ui
                .button(">|")
                .on_hover_text("Next controller sample (Shift+Right)")
                .clicked()
            {
                playback.next_sample(last);
            }
            if ui.button(">>|").on_hover_text("End (End)").clicked() {
                playback.seek_end(last);
            }
            ui.separator();
            ui.add(
                egui::Slider::new(&mut playback.speed, 0.1..=50.0)
                    .logarithmic(true)
                    .text("x real time"),
            );
            ui.separator();
            let mut dt = playback.dt(h);
            let response = ui
                .add(
                    egui::Slider::new(&mut dt, h / f64::from(MAX_SUBDIVISIONS)..=h)
                        .logarithmic(true)
                        .text(format!("dt [s] = h/{}", playback.subdivisions())),
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
            ui.label(format!(
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

    fn plots_ui(&mut self, ui: &mut egui::Ui) {
        let (Some(replay), Some(plots)) = (&self.replay, &self.plots) else {
            return;
        };
        let t = self.playback.time(replay.h());
        ui.heading("State");
        ui.label("Click a plot to jump to that time.");
        let seek = egui::ScrollArea::vertical()
            .show(ui, |ui| {
                [
                    time_plot(
                        ui,
                        "Position",
                        t,
                        &[("x", &plots.x, POS_X_COLOR), ("y", &plots.y, POS_Y_COLOR)],
                    ),
                    time_plot(
                        ui,
                        "Velocity",
                        t,
                        &[
                            ("vx", &plots.vx, VEL_X_COLOR),
                            ("vy", &plots.vy, VEL_Y_COLOR),
                        ],
                    ),
                    time_plot(ui, "theta", t, &[("theta", &plots.theta, THETA_COLOR)]),
                    time_plot(ui, "omega", t, &[("omega", &plots.omega, OMEGA_COLOR)]),
                    time_plot(ui, "Thrust F", t, &[("F", &plots.thrust, THRUST_COLOR)]),
                    time_plot(ui, "Torque tau", t, &[("tau", &plots.torque, TORQUE_COLOR)]),
                ]
                .into_iter()
                .flatten()
                .next()
            })
            .inner;
        if let Some(t) = seek {
            self.playback.playing = false;
            self.playback.seek_time(t, replay.h(), replay.last_frame());
        }
    }

    fn canvas_ui(&mut self, ui: &mut egui::Ui) {
        let (rect, response) =
            ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());

        // Pan with primary drag (disabled while following the ship, since
        // the center is locked to the ship's position each frame).
        if !self.follow_ship && response.dragged_by(egui::PointerButton::Primary) {
            self.camera.pan_by_screen(response.drag_delta());
        }

        // Zoom with scroll/pinch while the cursor is over the canvas.
        if let Some(hover_pos) = response.hover_pos() {
            let (scroll_y, zoom_delta) = ui
                .ctx()
                .input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = zoom_delta * (scroll_y / 200.0).exp();
            if factor.is_finite() && factor > 0.0 && (factor - 1.0).abs() > f32::EPSILON {
                self.camera.zoom_at(factor, hover_pos, rect);
            }
        }

        if self.fit_pending {
            if let Some(replay) = &self.replay {
                fit_camera(&mut self.camera, replay.bounds(), rect);
            }
            self.fit_pending = false;
        }

        let painter = ui.painter_at(rect);
        let style = GridStyle::from_visuals(ui.visuals());
        let (Some(replay), Some((k, _, state))) = (&self.replay, self.now()) else {
            draw_grid(&painter, &self.camera, rect, &style);
            return;
        };

        // Follow mode: keep the ship centered. Done after pan/zoom so zoom
        // still adjusts scale around the (now ship-centered) view.
        if self.follow_ship {
            self.camera.center = world(state.x, state.y);
        }

        draw_grid(&painter, &self.camera, rect, &style);
        if self.show_path {
            self.draw_path(&painter, rect, replay, k, &state);
        }
        self.draw_goal(&painter, rect, replay, &state);
        self.draw_ship(&painter, rect, &state);
        self.draw_arrows(&painter, rect, replay, k, &state);
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
        let full = screen_path(&self.camera, rect, states);
        let mut traveled = screen_path(&self.camera, rect, &states[..=k]);
        traveled.push(self.camera.world_to_screen(world(now.x, now.y), rect));
        painter.add(egui::Shape::line(
            full,
            egui::Stroke::new(1.0_f32, PATH_COLOR),
        ));
        painter.add(egui::Shape::line(
            traveled,
            egui::Stroke::new(2.0_f32, TRAVELED_COLOR),
        ));

        let start = self
            .camera
            .world_to_screen(world(states[0].x, states[0].y), rect);
        painter.circle_stroke(
            start,
            START_RADIUS_PX,
            egui::Stroke::new(1.5_f32, TRAVELED_COLOR),
        );
    }

    fn draw_goal(&self, painter: &egui::Painter, rect: egui::Rect, replay: &Replay, now: &State) {
        let task = &replay.case.task;
        let goal = self
            .camera
            .world_to_screen(world(task.goal.x, task.goal.y), rect);
        let reached = task.reached(now);
        let color = GOAL_COLOR.gamma_multiply(if reached { 0.35 } else { 1.0 });
        painter.circle_stroke(goal, GOAL_RADIUS_PX, egui::Stroke::new(2.0_f32, color));
        painter.circle_filled(goal, 2.0, color);

        // Position tolerance eps_p, drawn to scale once it is visible.
        let tolerance_px = task.tolerance.position as f32 * self.camera.scale;
        if tolerance_px > 1.5 {
            painter.circle_stroke(goal, tolerance_px, egui::Stroke::new(1.0_f32, color));
        }
        painter.text(
            goal + egui::vec2(GOAL_LABEL_OFFSET_PX, -GOAL_LABEL_OFFSET_PX),
            egui::Align2::LEFT_BOTTOM,
            if reached { "B (reached)" } else { "B" },
            egui::FontId::monospace(12.0),
            color,
        );
    }

    fn draw_ship(&self, painter: &egui::Painter, rect: egui::Rect, state: &State) {
        let center = self.camera.world_to_screen(world(state.x, state.y), rect);
        let (sin_theta, cos_theta) = (state.theta as f32).sin_cos();
        let (sin_2alpha, cos_2alpha) = (2.0 * self.alpha).sin_cos();

        // Columns of P: three unit-circle vertices of the base isosceles triangle.
        // alpha = angle between the midline and the main side.
        let base_vertices = [
            (1.0_f32, 0.0_f32),
            (-cos_2alpha, sin_2alpha),
            (-cos_2alpha, -sin_2alpha),
        ];

        let pixel_radius = self.circumradius * self.camera.scale;
        let vertices: Vec<egui::Pos2> = base_vertices
            .iter()
            .map(|&(px, py)| {
                let x = cos_theta * px - sin_theta * py;
                let y = sin_theta * px + cos_theta * py;
                egui::pos2(center.x + pixel_radius * x, center.y - pixel_radius * y)
            })
            .collect();

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
        let center = self.camera.world_to_screen(world(state.x, state.y), rect);

        // World vectors -> screen vectors: flip y. Use fixed pixel scales
        // (decoupled from camera zoom) so arrows behave like HUD indicators.
        let arrow = |v: [f64; 2], px_per_unit: f32, color: egui::Color32| {
            let screen = egui::vec2(v[0] as f32, -(v[1] as f32)) * px_per_unit;
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
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.source_ui(ui);
            ui.separator();
            self.case_ui(ui);
            self.case_info_ui(ui);
            ui.separator();
            self.frame_info_ui(ui);
            ui.separator();
            self.display_ui(ui);
        });
    }

    /// Desktop layout: controls left, plots right, timeline below the canvas.
    fn wide_layout(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("config_panel").show_inside(ui, |ui| self.controls_ui(ui));
        egui::Panel::bottom("timeline_panel").show_inside(ui, |ui| self.timeline_ui(ui));
        egui::Panel::right("plots_panel")
            .resizable(true)
            .default_size(PLOT_PANEL_DEFAULT_WIDTH)
            .min_size(PLOT_PANEL_MIN_WIDTH)
            .show_inside(ui, |ui| self.plots_ui(ui));
        egui::CentralPanel::default().show_inside(ui, |ui| self.canvas_ui(ui));
    }

    /// Narrow-screen layout: one full-size view at a time, chosen in the top bar; the
    /// timeline stays available below the canvas and the plots.
    fn compact_layout(&mut self, ui: &mut egui::Ui) {
        if self.compact_view != CompactView::Controls {
            egui::Panel::bottom("timeline_panel").show_inside(ui, |ui| self.timeline_ui(ui));
        }
        egui::CentralPanel::default().show_inside(ui, |ui| match self.compact_view {
            CompactView::Canvas => self.canvas_ui(ui),
            CompactView::Controls => self.controls_ui(ui),
            CompactView::Plots => self.plots_ui(ui),
        });
    }
}

impl eframe::App for ViewerApp {
    /// Called by the framework to save state before shutdown.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    /// Called each time the UI needs repainting, which may be many times per second.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_runner();
        self.handle_dropped_files(ui);
        self.watch_source(ui);

        // Advance playback first, so every panel shows the same instant.
        self.handle_keys(ui);
        if let Some(replay) = &self.replay {
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
        egui::Panel::top("top_panel").show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                if compact {
                    for (view, label) in [
                        (CompactView::Canvas, "View"),
                        (CompactView::Controls, "Controls"),
                        (CompactView::Plots, "Plots"),
                    ] {
                        ui.selectable_value(&mut self.compact_view, view, label);
                    }
                    ui.separator();
                }
            });
        });

        if compact {
            self.compact_layout(ui);
        } else {
            self.wide_layout(ui);
        }

        let ctx = ui.ctx().clone();
        self.code_window(&ctx);
    }
}

fn world(x: f64, y: f64) -> egui::Vec2 {
    egui::vec2(x as f32, y as f32)
}

/// Screen polyline through `states`, skipping points closer than `PATH_MIN_SEGMENT_PX`.
fn screen_path(camera: &Camera, rect: egui::Rect, states: &[State]) -> Vec<egui::Pos2> {
    let mut path: Vec<egui::Pos2> = Vec::with_capacity(states.len());
    for s in states {
        let p = camera.world_to_screen(world(s.x, s.y), rect);
        if path
            .last()
            .is_none_or(|q| q.distance(p) >= PATH_MIN_SEGMENT_PX)
        {
            path.push(p);
        }
    }
    if let (Some(s), Some(&q)) = (states.last(), path.last()) {
        let p = camera.world_to_screen(world(s.x, s.y), rect);
        if p != q {
            path.push(p);
        }
    }
    path
}

/// Center the camera on `bounds` and zoom so they fill the canvas, with a margin.
fn fit_camera(camera: &mut Camera, bounds: ([f64; 2], [f64; 2]), rect: egui::Rect) {
    let (lo, hi) = bounds;
    camera.center = world(f64::midpoint(lo[0], hi[0]), f64::midpoint(lo[1], hi[1]));
    let size = world(hi[0] - lo[0], hi[1] - lo[1]).max(egui::vec2(1.0, 1.0));
    let usable = rect.size() * (1.0 - 2.0 * FIT_MARGIN);
    let scale = (usable.x / size.x).min(usable.y / size.y);
    if scale.is_finite() && scale > 0.0 {
        camera.scale = scale;
    }
}

fn make_plot(id: &str) -> Plot<'static> {
    Plot::new(id.to_owned())
        .height(PLOT_HEIGHT)
        .legend(Legend::default())
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
}

/// Plot `lines` against time with a cursor at `t`. Returns the clicked time, if any.
fn time_plot(
    ui: &mut egui::Ui,
    title: &str,
    t: f64,
    lines: &[(&str, &Vec<PlotPoint>, egui::Color32)],
) -> Option<f64> {
    ui.label(title);
    make_plot(title)
        .show(ui, |plot_ui| {
            for &(name, series, color) in lines {
                plot_ui.line(Line::new(name, series.as_slice()).color(color));
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
