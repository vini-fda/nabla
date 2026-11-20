//! The User inputs help: the keyboard and mouse, the touch gestures, or the controller, in a
//! window over the canvas or on the title screen.

use super::ViewerApp;
use crate::controller_guide;
use crate::game;
use crate::keys;
use crate::theme::{icons, palette};

/// What the User inputs panel shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HelpTab {
    /// The keyboard and mouse, or the touch screen.
    Device,
    Controller,
}

impl ViewerApp {
    /// Shows or hides the User inputs window, opening on the controller's tab.
    pub(super) fn toggle_controller_guide(&mut self) {
        self.show_help = !self.show_help;
        if self.show_help {
            self.help_tab = Some(HelpTab::Controller);
        }
    }

    /// The user inputs help: touch gestures on a phone or tablet, keyboard and mouse elsewhere.
    /// With a `controller` connected, its controls too.
    pub(super) fn user_inputs_ui(&mut self, ui: &mut egui::Ui, width: f32) {
        let k = keys::Keys::new(ui.ctx());
        let shown = self.help_tab();
        let mut tab = shown;
        ui.horizontal(|ui| {
            let device = if k.touch() {
                "Touch"
            } else {
                "Keyboard and mouse"
            };
            ui.selectable_value(&mut tab, HelpTab::Device, device);
            ui.selectable_value(&mut tab, HelpTab::Controller, "Controller");
        });
        if tab != shown {
            self.help_tab = Some(tab);
        }
        match tab {
            HelpTab::Device if k.touch() => Self::touch_inputs_ui(ui),
            HelpTab::Device => Self::keyboard_mouse_inputs_ui(ui, k),
            HelpTab::Controller => {
                controller_guide::ui(ui, width, self.gamepads.state(), &mut self.show_guides);
            }
        }
    }

    /// The tab of the User inputs panel showing: the one last picked, or the controller's while
    /// one is connected.
    pub(super) fn help_tab(&self) -> HelpTab {
        self.help_tab.unwrap_or_else(|| {
            if self.gamepads.state().is_some() {
                HelpTab::Controller
            } else {
                HelpTab::Device
            }
        })
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
                |ui| ui.label("Turn, -100% to +100%"),
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

    /// The user inputs help, in a window that starts in the bottom right corner of the `canvas`,
    /// no larger than it. Escape closes it, as do its close button and the canvas's button.
    pub(super) fn help_window(&mut self, ctx: &egui::Context, canvas: egui::Rect) {
        const MARGIN: f32 = 8.0;
        /// With the title bar and the frame's margins: the controller's map at its full size.
        const DEFAULT_SIZE: egui::Vec2 = egui::vec2(controller_guide::WIDTH + 24.0, 470.0);
        let room = canvas.shrink(MARGIN);
        let size = DEFAULT_SIZE.min(room.size());
        let mut open = self.show_help;
        egui::Window::new("User inputs")
            .id(egui::Id::unique("user_inputs_window"))
            .open(&mut open)
            .collapsible(false)
            .vscroll(true)
            .default_rect(egui::Rect::from_min_max(
                room.right_bottom() - size,
                room.right_bottom(),
            ))
            .show(ctx, |ui| {
                let width = ui.available_width();
                self.user_inputs_ui(ui, width);
            });
        self.show_help = open && !ctx.input(|i| i.key_pressed(egui::Key::Escape));
    }
}

/// A group of the User inputs panel: its name, then its rows, the inputs in a column wide
/// enough that every group's actions line up.
pub(super) fn input_group(ui: &mut egui::Ui, name: &str, rows: impl FnOnce(&mut egui::Ui)) {
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
pub(super) fn input_row<A, B>(
    ui: &mut egui::Ui,
    inputs: impl FnOnce(&mut egui::Ui) -> A,
    action: impl FnOnce(&mut egui::Ui) -> B,
) {
    ui.horizontal(inputs);
    action(ui);
    ui.end_row();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_shows_the_controller_guide_whatever_tab_was_picked() {
        let mut app = ViewerApp {
            help_tab: Some(HelpTab::Device),
            ..ViewerApp::default()
        };
        assert!(app.show_guides, "the guides are on by default");
        app.toggle_controller_guide();
        assert!(app.show_help);
        assert_eq!(app.help_tab(), HelpTab::Controller);
        app.toggle_controller_guide();
        assert!(!app.show_help);
    }
}
