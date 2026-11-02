//! The title screen: the game's name and, below its Play, Settings and Help buttons, the
//! starting conditions to fly from (a preset's, the player's own, or random ones), the settings,
//! or the controls.

use std::ops::RangeInclusive;

use nabla_core::suite;

use super::{ViewerApp, input_group, input_row, toggled};
use crate::formula::{self, Formula};
use crate::game::Steering;
use crate::gamepad;
use crate::keys;
use crate::theme::{self, icons, palette};

/// The title's size, in points.
const TITLE_SIZE: f32 = 112.0;
/// Widest the column below the title gets, in points.
const COLUMN_WIDTH: f32 = 540.0;
const BUTTON_HEIGHT: f32 = 40.0;
const BUTTON_TEXT_SIZE: f32 = 16.0;
const PAGE_HEADING_SIZE: f32 = 24.0;
/// Name of the starting conditions once the player changes them, and of random ones.
const CUSTOM: &str = "custom";
const RANDOM: &str = "random";
/// The ranges the starting conditions can be set in, past those of every preset.
const POSITION_RANGE: RangeInclusive<f64> = -100.0..=100.0;
const VELOCITY_RANGE: RangeInclusive<f64> = -20.0..=20.0;
const THETA_RANGE: RangeInclusive<f64> = -std::f64::consts::PI..=std::f64::consts::PI;
const OMEGA_RANGE: RangeInclusive<f64> = -10.0..=10.0;
/// The range the ship's mass, inertia, thrust and torque can be set in: positive, as the model
/// divides by the first two and a ship without the others can't fly.
const SHIP_RANGE: RangeInclusive<f64> = 0.05..=20.0;

/// Which screen shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Screen {
    #[default]
    Title,
    Flight,
}

/// What the title screen shows below its buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum TitlePage {
    #[default]
    Start,
    Settings,
    Help,
}

impl ViewerApp {
    /// The title screen. Enter, or Cross or Options on a controller, plays; Escape goes back from
    /// the settings or the help to the starting conditions.
    pub(super) fn title_ui(&mut self, ui: &mut egui::Ui) {
        let mut play = self.title_keys(ui);
        egui::CentralPanel::default().show(ui, |ui| {
            let top = (ui.available_height() * 0.06).max(12.0);
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    ui.add_space(top);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new("Nabla")
                                .font(theme::title_font(TITLE_SIZE))
                                .color(palette::WHITE),
                        );
                    });
                    ui.add_space(24.0);
                    let width = ui.available_width().min(COLUMN_WIDTH);
                    let margin = (ui.available_width() - width) / 2.0;
                    ui.horizontal(|ui| {
                        ui.add_space(margin);
                        ui.vertical(|ui| {
                            ui.set_width(width);
                            play |= self.title_buttons(ui, width);
                            ui.add_space(12.0);
                            egui::Frame::new()
                                .fill(palette::SURFACE0)
                                .stroke(egui::Stroke::new(1.0, palette::BORDER))
                                .corner_radius(10)
                                .inner_margin(16)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    match self.title_page {
                                        TitlePage::Start => self.setup_ui(ui),
                                        TitlePage::Settings => self.settings_ui(ui),
                                        TitlePage::Help => self.help_page_ui(ui),
                                    }
                                });
                            ui.add_space(24.0);
                        });
                    });
                });
        });
        if play {
            self.score_policy();
            self.start_game();
        }
    }

    /// Whether the keys or the controller play, on the title screen. Escape goes back.
    fn title_keys(&mut self, ui: &egui::Ui) -> bool {
        if ui.ctx().memory(|m| m.focused().is_some()) {
            return false;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.title_page = TitlePage::Start;
        }
        let pad = self.gamepads.state().is_some_and(|pad| {
            pad.just_pressed(gamepad::Button::South) || pad.just_pressed(gamepad::Button::Start)
        });
        pad || ui.input(|i| i.key_pressed(egui::Key::Enter))
    }

    /// Play, Settings and Help, side by side across `width`. Returns whether Play was clicked;
    /// the others show their page, or the starting conditions again.
    fn title_buttons(&mut self, ui: &mut egui::Ui, width: f32) -> bool {
        ui.horizontal(|ui| {
            let spacing = ui.spacing().item_spacing.x;
            let size = egui::vec2((width - 2.0 * spacing) / 3.0, BUTTON_HEIGHT);
            let play_text = icon_text(
                icons::PLAY,
                egui::FontId::proportional(BUTTON_TEXT_SIZE),
                "Play",
                BUTTON_TEXT_SIZE,
                palette::BLACK,
            );
            let play = ui
                .add(
                    egui::Button::new(play_text)
                        .fill(palette::GREEN)
                        .min_size(size),
                )
                .on_hover_ui(|ui| {
                    keys::tooltip(ui, "Fly from the starting conditions", &[keys::ENTER]);
                })
                .clicked();
            for (page, icon, label) in [
                (TitlePage::Settings, icons::outline::GEAR_SIX, "Settings"),
                (TitlePage::Help, icons::outline::QUESTION, "Help"),
            ] {
                let text = icon_text(
                    icon,
                    icons::outline::font(BUTTON_TEXT_SIZE + 2.0),
                    label,
                    BUTTON_TEXT_SIZE,
                    palette::WHITE,
                );
                let open = self.title_page == page;
                if ui
                    .add(egui::Button::new(text).selected(open).min_size(size))
                    .clicked()
                {
                    self.title_page = if open { TitlePage::Start } else { page };
                }
            }
            play
        })
        .inner
    }

    /// The starting conditions: a preset to start from, random ones, the start and goal to
    /// change, and the ship.
    fn setup_ui(&mut self, ui: &mut egui::Ui) {
        self.world_picker_ui(ui);
        ui.separator();
        page_heading(ui, icons::outline::ROCKET, "Starting conditions");
        let mut picked = None;
        ui.horizontal(|ui| {
            ui.label("Preset:");
            egui::ComboBox::from_id_salt("preset")
                .selected_text(egui::RichText::new(&self.setup.name).monospace())
                .height(400.0)
                .show_ui(ui, |ui| {
                    for suite_name in ["standard", "holdout"] {
                        ui.label(egui::RichText::new(suite_name).monospace().strong());
                        for entry in self.cases.iter().filter(|e| e.suite == suite_name) {
                            let name = &entry.case.name;
                            let selected = *name == self.setup.name;
                            if ui
                                .selectable_label(selected, egui::RichText::new(name).monospace())
                                .clicked()
                            {
                                picked = Some(entry.case.clone());
                            }
                        }
                    }
                });
            let random = icon_text(
                icons::outline::DICE_FIVE,
                icons::outline::font(16.0),
                "Random",
                BUTTON_TEXT_SIZE,
                palette::WHITE,
            );
            if ui
                .button(random)
                .on_hover_text("Random starting conditions, with a random ship")
                .clicked()
            {
                let seed = (ui.input(|i| i.time) * 1.0e9) as u64;
                picked = suite::random(RANDOM, seed, 1).pop().map(|mut case| {
                    case.name = RANDOM.to_owned();
                    case
                });
            }
        });
        if let Some(case) = picked {
            self.setup = case;
        }
        ui.add_space(4.0);
        self.conditions_ui(ui);
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(
                "Drag the values, or click one to type it, to start from your own conditions.",
            )
            .small()
            .color(palette::OVERLAY1),
        );
    }

    /// The start, the goal and the ship, one quantity a row, to change, which makes the starting
    /// conditions the player's own.
    fn conditions_ui(&mut self, ui: &mut egui::Ui) {
        let setup = &mut self.setup;
        let (start, task) = (&mut setup.start, &mut setup.task);
        let (goal, p) = (&mut task.goal, &mut task.params);
        let mut changed = false;
        egui::Grid::new("setup")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                let mut row = |ui: &mut egui::Ui,
                               formula: &Formula,
                               name: &str,
                               values: &mut [&mut f64],
                               (range, speed, decimals): (RangeInclusive<f64>, f64, usize),
                               unit: &str| {
                    changed |= edit_cell(ui, formula, name, values, range, speed, decimals, unit);
                    ui.end_row();
                };
                let position = (POSITION_RANGE, 0.1, 2);
                let ship = (SHIP_RANGE, 0.01, 3);
                row(
                    ui,
                    &formula::INITIAL_POSITION,
                    "initial position",
                    &mut [&mut start.x, &mut start.y],
                    position.clone(),
                    "m",
                );
                row(
                    ui,
                    &formula::INITIAL_VELOCITY,
                    "initial velocity",
                    &mut [&mut start.vx, &mut start.vy],
                    (VELOCITY_RANGE, 0.05, 2),
                    "m/s",
                );
                row(
                    ui,
                    &formula::INITIAL_THETA,
                    "initial heading angle",
                    &mut [&mut start.theta],
                    (THETA_RANGE, 0.01, 2),
                    "rad",
                );
                row(
                    ui,
                    &formula::INITIAL_OMEGA,
                    "initial angular velocity",
                    &mut [&mut start.omega],
                    (OMEGA_RANGE, 0.02, 2),
                    "rad/s",
                );
                row(
                    ui,
                    &formula::GOAL,
                    "the goal B",
                    &mut [&mut goal.x, &mut goal.y],
                    position,
                    "m",
                );
                ui.label(egui::RichText::new("Ship").small().color(palette::OVERLAY1));
                ui.end_row();
                row(
                    ui,
                    &formula::MASS,
                    "mass",
                    &mut [&mut p.mass],
                    ship.clone(),
                    "kg",
                );
                row(
                    ui,
                    &formula::INERTIA,
                    "moment of inertia",
                    &mut [&mut p.inertia],
                    ship.clone(),
                    "kg m^2",
                );
                row(
                    ui,
                    &formula::F_MAX,
                    "maximum thrust",
                    &mut [&mut p.f_max],
                    ship.clone(),
                    "N",
                );
                row(
                    ui,
                    &formula::TAU_MAX,
                    "maximum torque",
                    &mut [&mut p.tau_max],
                    ship,
                    "N m",
                );
            });
        if changed {
            setup.name = CUSTOM.to_owned();
        }
    }

    /// The settings kept from flight to flight, the same as in the flight's panel where they are
    /// there too.
    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        page_heading(ui, icons::outline::GEAR_SIX, "Settings");
        self.flight_settings_ui(ui);
        ui.separator();
        self.display_ui(ui, false);
        ui.separator();
        self.sound.ui(ui);
    }

    /// How the flight starts: the stability assist, pointing, and the autopilot.
    fn flight_settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Flight");
        let mut sas = self.steering == Steering::Sas;
        if ui
            .checkbox(&mut sas, "Stability assist (SAS)")
            .on_hover_text("When you stop turning, stop the spin and hold the heading")
            .changed()
        {
            self.steering = toggled(self.steering);
        }
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.pointing, "Point").on_hover_text(
                "Hold the mouse or a finger on the canvas: the ship turns toward it, and thrusts \
                 once it faces it within the cone",
            );
            ui.add_enabled(
                self.pointing,
                egui::DragValue::new(&mut self.aim_cone)
                    .range(super::AIM_CONE_RANGE)
                    .speed(0.5)
                    .max_decimals(0)
                    .prefix("cone ")
                    .suffix(" deg"),
            );
        });
        ui.checkbox(
            &mut self.autopilot_on_start,
            "Autopilot flies from the start",
        )
        .on_hover_text("Switch it off in flight to take over");
        self.policy_ui(ui);
    }

    /// The controls: on the title screen, and in flight on the keyboard and mouse or the touch
    /// screen, and the controller's.
    fn help_page_ui(&self, ui: &mut egui::Ui) {
        use keys::{ENTER, Key, caps, joiner};

        page_heading(ui, icons::outline::QUESTION, "Help");
        let controller = self.gamepads.state().is_some();
        input_group(ui, "Title screen", |ui| {
            input_row(
                ui,
                |ui| {
                    if !keys::Keys::new(ui.ctx()).touch() {
                        caps(ui, &[ENTER]);
                        joiner(ui, "/");
                    }
                    caps(ui, &[Key::Word("Cross")]);
                },
                |ui| ui.label("Play"),
            );
        });
        ui.add_space(4.0);
        Self::user_inputs_ui(ui, controller);
        if !controller {
            ui.add_space(8.0);
            keys::label_with_icons(
                ui,
                &[
                    keys::Run::Icon(icons::outline::GAME_CONTROLLER),
                    keys::Run::Text(" Connect a game controller to see its controls"),
                ],
            );
        }
    }
}

/// A grid cell pair: the `formula` naming the quantity (`name` on hover), and its `values`
/// to drag in `range` at `speed`, shown with `decimals`, then its `unit`. Returns whether any
/// changed.
#[expect(
    clippy::too_many_arguments,
    reason = "the cell's parts, each a plain value"
)]
fn edit_cell(
    ui: &mut egui::Ui,
    formula: &Formula,
    name: &str,
    values: &mut [&mut f64],
    range: RangeInclusive<f64>,
    speed: f64,
    decimals: usize,
    unit: &str,
) -> bool {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    formula.ui(ui, &font).on_hover_text(name);
    ui.horizontal(|ui| {
        let mut changed = false;
        for value in values {
            let drag = egui::DragValue::new(*value)
                .range(range.clone())
                // A loaded flight can be outside the editor's starting ranges.
                .clamp_existing_to_range(false)
                .speed(speed)
                .fixed_decimals(decimals);
            changed |= ui.add(drag).changed();
        }
        ui.label(unit);
        changed
    })
    .inner
}

/// A page's heading on the title screen, after its `icon`.
fn page_heading(ui: &mut egui::Ui, icon: &str, title: &str) {
    ui.label(icon_text(
        icon,
        icons::outline::font(PAGE_HEADING_SIZE),
        title,
        PAGE_HEADING_SIZE,
        palette::WHITE,
    ));
    ui.add_space(4.0);
}

/// The `icon`, in `icon_font`, and then `text`, at `size`, in `color`: a button's text, or a
/// heading's.
fn icon_text(
    icon: &str,
    icon_font: egui::FontId,
    text: &str,
    size: f32,
    color: egui::Color32,
) -> egui::text::LayoutJob {
    let format = |font_id| egui::TextFormat {
        font_id,
        color,
        valign: egui::Align::Center,
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    job.append(icon, 0.0, format(icon_font));
    job.append(text, 8.0, format(egui::FontId::proportional(size)));
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn showing_loaded_conditions_preserves_out_of_editor_range_state() {
        let mut app = ViewerApp::default();
        app.setup.start.x = 500.0;
        app.setup.start.vy = 80.0;
        app.setup.start.theta = 40.0;
        app.setup.start.omega = 30.0;
        let before = app.setup.clone();
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        egui_extras::install_image_loaders(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.conditions_ui(ui));
        output.textures_delta.clear();
        assert_eq!(
            app.setup, before,
            "displaying a save must not clamp its state"
        );
    }
}
