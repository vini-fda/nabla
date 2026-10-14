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
const CONTROLS_PANEL: SidePanel = SidePanel {
    dock: Dock::Left,
    id: "config_panel",
    title: "Control policy",
    // Narrowest the contents can be dragged to; below this their rows would be cut off.
    min_width: 320.0,
    default_width: 320.0,
};
const PLOTS_PANEL: SidePanel = SidePanel {
    dock: Dock::Right,
    id: "plots_panel",
    title: "State",
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

/// Smallest sampling period offered by the slider.
const H_MIN: f64 = 0.01;

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
    show_velocity: bool,
    show_acceleration: bool,
    show_path: bool,
    follow_ship: bool,
    playback: Playback,
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
            show_velocity: true,
            show_acceleration: true,
            show_path: true,
            follow_ship: false,
            playback: Playback::default(),
            case: Some(DEFAULT_CASE.to_owned()),
            h: suite::H,
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
            show_controls: true,
            show_plots: true,
            compact_view: CompactView::default(),
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

        let mut app = Self {
            runner: Runner::new(cc.egui_ctx.clone()),
            database: source::database(),
            cases: all_cases(),
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
        ui.heading("Case");
        let old_case = self.case.clone();
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
        if self.case != old_case {
            self.run(true);
        }
        let h = ui.add(
            egui::Slider::new(&mut self.h, H_MIN..=suite::H)
                .text("h (sampling period)")
                .suffix(" s"),
        );
        if h.double_clicked() {
            self.h = suite::H;
        }
        h.on_hover_text(format!(
            "Seconds between controller samples. The suites use {} s, so step counts at other \
             values are not scores. Double-click to reset.",
            suite::H
        ));
        if self.replay.as_ref().is_some_and(|r| r.h() != self.h) && !self.runner.busy() {
            // Keep the playhead time.
            self.run(false);
        }
    }

    /// Optional window showing the source with syntax highlighting and line numbers.
    fn code_window(&mut self, ctx: &egui::Context) {
        let Some(source) = &self.source else {
            return;
        };
        let mut open = self.show_code;
        egui::Window::new(egui::RichText::new(format!("{}.rs", source.name)).monospace())
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
                ui.monospace(name);
                ui.monospace(value);
                ui.end_row();
            };
            row("m, I", format!("{:.3}, {:.3}", p.mass, p.inertia));
            row(
                "F_max, tau_max",
                format!("{:.3}, {:.3}", p.f_max, p.tau_max),
            );
            row(
                "goal B",
                format!("({:.2}, {:.2})", task.goal.x, task.goal.y),
            );
            row("start position", format!("({:.2}, {:.2})", s.x, s.y));
            row("start velocity", format!("({:.2}, {:.2})", s.vx, s.vy));
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
        ui.heading("Current state");
        ui.label(format!("t = {:.4} s", replay.time(k) + offset));
        ui.label(format!("sample k = {k} + {offset:.4} s"));
        egui::Grid::new("frame_info").num_columns(2).show(ui, |ui| {
            let mut row = |name: &str, value: String| {
                ui.monospace(name);
                ui.monospace(value);
                ui.end_row();
            };
            row("position", format!("({:.3}, {:.3})", s.x, s.y));
            row("velocity", format!("({:.3}, {:.3})", s.vx, s.vy));
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
            if ui
                .button("\u{23EE}")
                .on_hover_text("Start (Home)")
                .clicked()
            {
                playback.seek_start();
            }
            if ui
                .button("|\u{25C0}")
                .on_hover_text("Previous controller sample (Shift+Left)")
                .clicked()
            {
                playback.previous_sample(last);
            }
            let play = if playback.playing {
                "\u{23F8}"
            } else {
                "\u{25B6}"
            };
            if ui
                .button(play)
                .on_hover_text("Play / pause (Space)")
                .clicked()
            {
                playback.toggle(last);
            }
            if ui
                .button("\u{25B6}|")
                .on_hover_text("Next controller sample (Shift+Right)")
                .clicked()
            {
                playback.next_sample(last);
            }
            if ui.button("\u{23ED}").on_hover_text("End (End)").clicked() {
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

    fn plots_ui(&mut self, ui: &mut egui::Ui) {
        let (Some(replay), Some(plots)) = (&self.replay, &self.plots) else {
            return;
        };
        let t = self.playback.time(replay.h());
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
        // Scrolls sideways too, so rows are never cut off if the space is narrower than they are.
        egui::ScrollArea::both().show(ui, |ui| {
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
        let mut show_controls = self.show_controls;
        CONTROLS_PANEL.show(ui, &mut show_controls, |ui| self.controls_ui(ui));
        self.show_controls = show_controls;
        egui::Panel::bottom("timeline_panel").show(ui, |ui| self.timeline_ui(ui));
        let mut show_plots = self.show_plots;
        PLOTS_PANEL.show(ui, &mut show_plots, |ui| self.plots_ui(ui));
        self.show_plots = show_plots;
        egui::CentralPanel::default().show(ui, |ui| self.canvas_ui(ui));
    }

    /// Narrow-screen layout: one full-size view at a time, chosen in the top bar; the
    /// timeline stays available below the canvas and the plots.
    fn compact_layout(&mut self, ui: &mut egui::Ui) {
        if self.compact_view != CompactView::Controls {
            egui::Panel::bottom("timeline_panel").show(ui, |ui| self.timeline_ui(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| match self.compact_view {
            CompactView::Canvas => self.canvas_ui(ui),
            CompactView::Controls => {
                ui.heading(CONTROLS_PANEL.title);
                self.controls_ui(ui);
            }
            CompactView::Plots => {
                ui.heading(PLOTS_PANEL.title);
                self.plots_ui(ui);
            }
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
        // The tab bar only exists in the compact layout; the wide layout shows everything.
        if compact {
            egui::Panel::top("top_panel").show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (view, label) in [
                        (CompactView::Canvas, "View"),
                        (CompactView::Controls, "Controls"),
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
        let id = egui::Id::new(id);
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
        .interact(rect, ui.id().with("title_bar"), egui::Sense::click())
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
