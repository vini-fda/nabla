//! Game controllers (a PS5 or an Xbox controller, ...), which egui doesn't read: read with
//! gilrs, natively from the system's HID devices and in the browser through its Gamepad API.
//!
//! This reads the first connected controller every frame: its sticks and the buttons the flight
//! uses, as [`State`], which [`State::input`] turns into the flight's controls. The left stick
//! and the bottom face button also show over the canvas, in [`readout_ui`]. Buttons are named
//! as on a Sony controller; on an Xbox one, Cross is A, Triangle Y, L1 LB and R2 RT. A browser
//! only lists a controller once one of its buttons has been pressed with the page open.

use std::time::Duration;

use egui::{Color32, Pos2, Stroke, Vec2};
pub use gilrs::Button;
use gilrs::{Axis, Event, EventType, Gilrs};

use crate::game::Input;
use crate::theme::palette;

/// How often to look for a controller while none is connected. While one is, every frame: its
/// events don't wake egui up the way the mouse's and the keyboard's do.
const SEARCH_INTERVAL: Duration = Duration::from_millis(250);

/// Radius of the stick's well in the readout, and of the dot showing where the stick is.
const STICK_RADIUS: f32 = 28.0;
const STICK_DOT_RADIUS: f32 = 5.0;
/// Radius of the button's light in the readout.
const BUTTON_RADIUS: f32 = 9.0;
const STICK_COLOR: Color32 = palette::SAPPHIRE;
const PRESSED_COLOR: Color32 = palette::GREEN;

/// A controller's state, as read this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// The name the system or the browser gives the controller.
    pub name: String,
    /// The left stick, each axis from -1 to 1, with x to the right and y up.
    pub stick: [f32; 2],
    /// The right stick, the same way.
    pub right_stick: [f32; 2],
    /// The bottom face button: Cross on a Sony controller, A on an Xbox one.
    pub south: bool,
    /// The right trigger, pressed past its threshold.
    pub r2: bool,
    /// The left shoulder button.
    pub l1: bool,
    /// The buttons pressed since the last frame, for the ones that switch something on a press
    /// rather than while held.
    pub pressed: Vec<Button>,
}

impl State {
    /// The flight's controls held on the controller: the left stick turns, as hard as it is
    /// pushed sideways (left, counterclockwise), Cross or R2 thrusts, and L1 is fine control.
    pub fn input(&self) -> Input {
        Input {
            thrust: self.south || self.r2,
            turn: f64::from(-self.stick[0]),
            fine: self.l1,
            aim: None,
        }
    }

    /// Whether `button` was pressed since the last frame.
    pub fn just_pressed(&self, button: Button) -> bool {
        self.pressed.contains(&button)
    }
}

/// The controllers, and the first connected one's state at the last [`Self::poll`].
pub struct Gamepads {
    /// `None` where gilrs could not start (it logged why).
    gilrs: Option<Gilrs>,
    state: Option<State>,
}

impl Default for Gamepads {
    fn default() -> Self {
        let gilrs = Gilrs::new()
            .inspect_err(|e| log::warn!("No game controllers: {e}"))
            .ok();
        Self { gilrs, state: None }
    }
}

impl Gamepads {
    /// Takes the controllers' events since the last call and reads the first connected one.
    /// Call it once a frame: it also asks for the next frame, soon enough to follow the
    /// controller.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(gilrs) = &mut self.gilrs else {
            return;
        };
        // The events update gilrs's own copy of each controller's state; the presses among them
        // are kept, with the controller they came from.
        let mut pressed = Vec::new();
        while let Some(Event { id, event, .. }) = gilrs.next_event() {
            if let EventType::ButtonPressed(button, _) = event {
                pressed.push((id, button));
            }
        }
        self.state = gilrs
            .gamepads()
            .find(|(_, pad)| pad.is_connected())
            .map(|(id, pad)| State {
                name: pad.name().to_owned(),
                stick: [pad.value(Axis::LeftStickX), pad.value(Axis::LeftStickY)],
                right_stick: [pad.value(Axis::RightStickX), pad.value(Axis::RightStickY)],
                south: pad.is_pressed(Button::South),
                r2: pad.is_pressed(Button::RightTrigger2),
                l1: pad.is_pressed(Button::LeftTrigger),
                pressed: pressed
                    .iter()
                    .filter(|&&(from, _)| from == id)
                    .map(|&(_, button)| button)
                    .collect(),
            });
        if self.state.is_some() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(SEARCH_INTERVAL);
        }
    }

    /// The first connected controller's state at the last [`Self::poll`].
    pub fn state(&self) -> Option<&State> {
        self.state.as_ref()
    }
}

/// Shows `state` in a small panel at `pos`: the controller's name, its left stick as a dot in a
/// well, with the axes' values, and whether its Cross button is pressed.
pub fn readout_ui(ctx: &egui::Context, pos: Pos2, state: &State) {
    egui::Area::new(egui::Id::unique("gamepad_readout"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(
                    egui::RichText::new(&state.name)
                        .small()
                        .color(palette::OVERLAY1),
                );
                ui.horizontal(|ui| {
                    stick_ui(ui, state.stick);
                    ui.vertical(|ui| {
                        let [x, y] = state.stick;
                        ui.monospace(format!("x = {x:+.3}"));
                        ui.monospace(format!("y = {y:+.3}"));
                        ui.horizontal(|ui| {
                            button_ui(ui, state.south);
                            ui.label(if state.south {
                                "Cross: pressed"
                            } else {
                                "Cross: released"
                            });
                        });
                    });
                });
            });
        });
}

/// The stick at `[x, y]` as a dot in a round well, with crosshairs at its center.
fn stick_ui(ui: &mut egui::Ui, [x, y]: [f32; 2]) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(2.0 * STICK_RADIUS), egui::Sense::hover());
    let painter = ui.painter();
    let center = rect.center();
    let faint = Stroke::new(1.0, palette::OVERLAY0);
    painter.circle(center, STICK_RADIUS, palette::SURFACE1, faint);
    painter.hline(rect.x_range(), center.y, faint);
    painter.vline(center.x, rect.y_range(), faint);
    // Screen y points down; the stick's y up.
    let offset = Vec2::new(x, -y) * (STICK_RADIUS - STICK_DOT_RADIUS);
    painter.circle_filled(center + offset, STICK_DOT_RADIUS, STICK_COLOR);
}

/// A light, filled while the button is `pressed`.
fn button_ui(ui: &mut egui::Ui, pressed: bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(2.0 * BUTTON_RADIUS), egui::Sense::hover());
    let painter = ui.painter();
    let (fill, stroke) = if pressed {
        (PRESSED_COLOR, PRESSED_COLOR)
    } else {
        (Color32::TRANSPARENT, palette::OVERLAY1)
    };
    painter.circle(
        rect.center(),
        BUTTON_RADIUS - 1.0,
        fill,
        Stroke::new(1.5, stroke),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_controller_flies_as_the_keyboard_and_the_joystick_do() {
        let idle = State::default().input();
        assert_eq!(idle, Input::default(), "nothing held");
        let state = State {
            stick: [-0.5, 0.9],
            r2: true,
            l1: true,
            ..State::default()
        };
        let input = state.input();
        assert!(input.thrust && input.fine, "R2 thrusts, L1 is fine control");
        assert_eq!(
            input.turn, 0.5,
            "left turns left, as hard as pushed; up does not count"
        );
        let cross = State {
            south: true,
            stick: [1.0, 0.0],
            ..State::default()
        };
        assert!(cross.input().thrust, "Cross thrusts");
        assert_eq!(cross.input().turn, -1.0, "full right");
    }
}
