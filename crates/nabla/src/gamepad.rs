//! Game controllers (a PS5 or an Xbox controller, ...), which egui doesn't read: read with
//! gilrs, natively from the system's HID devices and in the browser through its Gamepad API.
//!
//! This reads the first connected controller every frame, as [`State`]: its sticks, its
//! triggers' depth and the buttons held, which [`State::input`] turns into the flight's
//! controls; `controller_view` draws them. Buttons are named as on a Sony controller; on an
//! Xbox one, Cross is A, Circle B, Square X, Triangle Y, L1 and R1 LB and RB, L2 and R2 LT and
//! RT, Create View and Options Menu. A browser only lists a controller once one of its buttons
//! has been pressed with the page open, and it counts that press as the controller's first.

use std::time::Duration;

pub use gilrs::Button;
use gilrs::{Axis, Event, EventType, GamepadId, Gilrs};

use crate::game::Input;

/// How often to look for a controller while none is connected. While one is, every frame: its
/// events don't wake egui up the way the mouse's and the keyboard's do.
const SEARCH_INTERVAL: Duration = Duration::from_millis(250);

/// The touchpad's click on macOS, which gilrs maps to no [`Button`], by its raw code: the 14th
/// HID button (the button page's usage 14, which SDL's mappings call `b13`).
#[cfg(not(target_arch = "wasm32"))]
const MACOS_TOUCHPAD: u32 = (0x09 << 16) | 14;
/// The touchpad's click in the browser's standard mapping: the button after the 17 standard
/// ones, which gilrs doesn't read.
#[cfg(target_arch = "wasm32")]
const WEB_TOUCHPAD: u32 = 17;

/// The buttons in the order of the browser's standard mapping.
#[cfg(target_arch = "wasm32")]
const WEB_BUTTONS: [Button; 17] = [
    Button::South,
    Button::East,
    Button::West,
    Button::North,
    Button::LeftTrigger,
    Button::RightTrigger,
    Button::LeftTrigger2,
    Button::RightTrigger2,
    Button::Select,
    Button::Start,
    Button::LeftThumb,
    Button::RightThumb,
    Button::DPadUp,
    Button::DPadDown,
    Button::DPadLeft,
    Button::DPadRight,
    Button::Mode,
];

/// The buttons read as held: every one the controller view shows.
const BUTTONS: [Button; 17] = [
    Button::South,
    Button::East,
    Button::West,
    Button::North,
    Button::LeftTrigger,
    Button::RightTrigger,
    Button::LeftTrigger2,
    Button::RightTrigger2,
    Button::Select,
    Button::Start,
    Button::Mode,
    Button::LeftThumb,
    Button::RightThumb,
    Button::DPadUp,
    Button::DPadDown,
    Button::DPadLeft,
    Button::DPadRight,
];

/// A controller's state, as read this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// The name the system or the browser gives the controller.
    pub name: String,
    /// The left stick, each axis from -1 to 1, with x to the right and y up.
    pub stick: [f32; 2],
    /// The right stick, the same way.
    pub right_stick: [f32; 2],
    /// How far L2 and R2 are pulled, from 0 to 1.
    pub triggers: [f32; 2],
    /// The buttons held, the triggers once pulled past their threshold.
    pub held: Vec<Button>,
    /// The touchpad is pressed down (clicked, not just touched).
    pub touchpad: bool,
    /// The buttons pressed since the last frame, for the ones that switch something on a press
    /// rather than while held.
    pub pressed: Vec<Button>,
}

impl State {
    /// The flight's controls held on the controller: the left stick turns, as hard as it is
    /// pushed sideways (left, counterclockwise), Cross or R2 thrusts, and L2 is fine control.
    pub fn input(&self) -> Input {
        Input {
            thrust: self.is_held(Button::South) || self.is_held(Button::RightTrigger2),
            turn: f64::from(-self.stick[0]),
            fine: self.is_held(Button::LeftTrigger2),
            aim: None,
        }
    }

    /// Which way the shoulder buttons zoom while held: 1 in (R1), -1 out (L1), 0 for neither
    /// or both.
    pub fn zoom(&self) -> f32 {
        let held = |button| f32::from(u8::from(self.is_held(button)));
        held(Button::RightTrigger) - held(Button::LeftTrigger)
    }

    /// Whether `button` is held.
    pub fn is_held(&self, button: Button) -> bool {
        self.held.contains(&button)
    }

    /// Whether `button` was pressed since the last frame.
    pub fn just_pressed(&self, button: Button) -> bool {
        self.pressed.contains(&button)
    }
}

/// The controller being read changed, with its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Connection {
    Connected(String),
    Disconnected(String),
}

/// What changed from the controller named `before` to the one named `after`. Another controller
/// taking over counts as it connecting.
fn connection_change(before: Option<&str>, after: Option<&str>) -> Option<Connection> {
    match (before, after) {
        (before, Some(after)) if before != Some(after) => {
            Some(Connection::Connected(after.to_owned()))
        }
        (Some(before), None) => Some(Connection::Disconnected(before.to_owned())),
        _ => None,
    }
}

/// A controller's name without what the browser appends to it, such as Chrome's
/// " (STANDARD GAMEPAD Vendor: 054c Product: 0ce6)".
fn tidy_name(name: &str) -> String {
    name.split(" (STANDARD GAMEPAD")
        .next()
        .unwrap_or(name)
        .trim()
        .to_owned()
}

/// Whether the touchpad of `pad` is pressed down (clicked, not just touched): on macOS, by its
/// raw code, the touchpad being a button gilrs reads but doesn't name.
#[cfg(not(target_arch = "wasm32"))]
fn touchpad(pad: &gilrs::Gamepad<'_>) -> bool {
    pad.state()
        .buttons()
        .any(|(code, data)| data.is_pressed() && code.into_u32() == MACOS_TOUCHPAD)
}

/// The first connected controller as the browser's Gamepad API has it, if in its standard
/// mapping. This is the one gilrs reads, the first connected.
#[cfg(target_arch = "wasm32")]
fn first_web_pad() -> Option<web_sys::Gamepad> {
    use wasm_bindgen::JsCast;
    web_sys::window()?
        .navigator()
        .get_gamepads()
        .ok()?
        .iter()
        .filter(|p| !p.is_null())
        .map(JsCast::unchecked_into::<web_sys::Gamepad>)
        .find(web_sys::Gamepad::connected)
        .filter(|p| p.mapping() == web_sys::GamepadMappingType::Standard)
}

/// Whether the touchpad of the first connected controller is pressed down, read straight from
/// the browser's Gamepad API: gilrs only reads the standard mapping's first 17 buttons.
#[cfg(target_arch = "wasm32")]
fn touchpad(_pad: &gilrs::Gamepad<'_>) -> bool {
    use wasm_bindgen::JsCast as _;
    first_web_pad().is_some_and(|p| {
        let button = p.buttons().get(WEB_TOUCHPAD);
        !button.is_undefined() && button.unchecked_into::<web_sys::GamepadButton>().pressed()
    })
}

/// Queues the press of the buttons held on `id`, which just connected. A browser only lists a
/// controller once a button is pressed, and gilrs takes the state it first sees as the starting
/// point, so without this that press would go unread until it was let go and pressed again.
#[cfg(target_arch = "wasm32")]
fn queue_held_buttons(gilrs: &mut Gilrs, id: GamepadId, gone: &[GamepadId]) {
    use wasm_bindgen::JsCast as _;
    let first = gilrs
        .gamepads()
        .map(|(id, _)| id)
        .find(|id| !gone.contains(id));
    let Some(web_pad) = first_web_pad().filter(|_| first == Some(id)) else {
        return;
    };
    let buttons = web_pad.buttons();
    let events: Vec<_> = WEB_BUTTONS
        .into_iter()
        .zip(0..)
        .filter_map(|(button, index)| {
            let data = buttons.get(index);
            if data.is_undefined() {
                return None;
            }
            let data = data.unchecked_into::<web_sys::GamepadButton>();
            let code = gilrs.gamepad(id).button_code(button)?;
            let value = data.value() as f32;
            data.pressed().then_some([
                Event::new(id, EventType::ButtonPressed(button, code)),
                Event::new(id, EventType::ButtonChanged(button, value, code)),
            ])
        })
        .flatten()
        .collect();
    for event in events {
        gilrs.insert_event(event);
    }
}

/// The controllers, and the first connected one's state at the last [`Self::poll`].
pub struct Gamepads {
    /// `None` where gilrs could not start (it logged why).
    gilrs: Option<Gilrs>,
    /// The controllers disconnected since they last connected. gilrs can't tell in the browser:
    /// it asks the last object the browser gave it for the controller, not the browser.
    gone: Vec<GamepadId>,
    state: Option<State>,
}

impl Default for Gamepads {
    fn default() -> Self {
        let gilrs = Gilrs::new()
            .inspect_err(|e| log::warn!("No game controllers: {e}"))
            .ok();
        Self {
            gilrs,
            gone: Vec::new(),
            state: None,
        }
    }
}

impl Gamepads {
    /// Takes the controllers' events since the last call and reads the first connected one.
    /// Call it once a frame: it also asks for the next frame, soon enough to follow the
    /// controller. Returns whether the controller being read changed.
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<Connection> {
        let gilrs = self.gilrs.as_mut()?;
        // The events update gilrs's own copy of each controller's state; the presses among them
        // are kept, with the controller they came from.
        let mut pressed = Vec::new();
        while let Some(Event { id, event, .. }) = gilrs.next_event() {
            match event {
                EventType::ButtonPressed(button, code) => {
                    pressed.push((id, button));
                    if button == Button::Unknown {
                        log::debug!("Unmapped controller button, code {}", code.into_u32());
                    }
                }
                EventType::Connected => {
                    self.gone.retain(|&gone| gone != id);
                    #[cfg(target_arch = "wasm32")]
                    queue_held_buttons(gilrs, id, &self.gone);
                }
                EventType::Disconnected => self.gone.push(id),
                _ => {}
            }
        }
        let state = gilrs
            .gamepads()
            .find(|(id, _)| !self.gone.contains(id))
            .map(|(id, pad)| {
                let depth = |button| pad.button_data(button).map_or(0.0, |d| d.value());
                State {
                    name: tidy_name(pad.name()),
                    stick: [pad.value(Axis::LeftStickX), pad.value(Axis::LeftStickY)],
                    right_stick: [pad.value(Axis::RightStickX), pad.value(Axis::RightStickY)],
                    triggers: [depth(Button::LeftTrigger2), depth(Button::RightTrigger2)],
                    held: BUTTONS
                        .into_iter()
                        .filter(|&button| pad.is_pressed(button))
                        .collect(),
                    touchpad: touchpad(&pad),
                    pressed: pressed
                        .iter()
                        .filter(|&&(from, _)| from == id)
                        .map(|&(_, button)| button)
                        .collect(),
                }
            });
        if state.is_some() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(SEARCH_INTERVAL);
        }
        let name = |state: &Option<State>| state.as_ref().map(|pad| pad.name.clone());
        let change = connection_change(name(&self.state).as_deref(), name(&state).as_deref());
        self.state = state;
        change
    }

    /// The first connected controller's state at the last [`Self::poll`].
    pub fn state(&self) -> Option<&State> {
        self.state.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A controller with `held` held and the left stick at `stick`.
    fn holding(held: &[Button], stick: [f32; 2]) -> State {
        State {
            stick,
            held: held.to_vec(),
            ..State::default()
        }
    }

    #[test]
    fn the_controller_flies_as_the_keyboard_and_the_joystick_do() {
        assert_eq!(State::default().input(), Input::default(), "nothing held");
        let input = holding(&[Button::RightTrigger2, Button::LeftTrigger2], [-0.5, 0.9]).input();
        assert!(input.thrust && input.fine, "R2 thrusts, L2 is fine control");
        assert_eq!(
            input.turn, 0.5,
            "left turns left, as hard as pushed; up does not count"
        );
        let cross = holding(&[Button::South], [1.0, 0.0]).input();
        assert!(cross.thrust, "Cross thrusts");
        assert_eq!(cross.turn, -1.0, "full right");
    }

    #[test]
    fn a_change_of_controller_is_told_once() {
        let change = connection_change;
        assert_eq!(
            change(None, Some("DualSense")),
            Some(Connection::Connected("DualSense".to_owned()))
        );
        assert_eq!(
            change(Some("DualSense"), None),
            Some(Connection::Disconnected("DualSense".to_owned()))
        );
        assert_eq!(
            change(Some("DualSense"), Some("Xbox")),
            Some(Connection::Connected("Xbox".to_owned())),
            "another took over"
        );
        assert_eq!(change(Some("DualSense"), Some("DualSense")), None);
        assert_eq!(change(None, None), None);
    }

    #[test]
    fn names_lose_what_the_browser_appends() {
        let chrome = "DualSense Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 0ce6)";
        assert_eq!(tidy_name(chrome), "DualSense Wireless Controller");
        assert_eq!(
            tidy_name("PS5 Controller"),
            "PS5 Controller",
            "native: as is"
        );
        assert_eq!(
            tidy_name("Pad (wired)"),
            "Pad (wired)",
            "other parentheses stay"
        );
    }

    #[test]
    fn the_shoulder_buttons_zoom() {
        let (l1, r1) = (Button::LeftTrigger, Button::RightTrigger);
        assert_eq!(holding(&[r1], [0.0; 2]).zoom(), 1.0, "R1 in");
        assert_eq!(holding(&[l1], [0.0; 2]).zoom(), -1.0, "L1 out");
        assert_eq!(holding(&[l1, r1], [0.0; 2]).zoom(), 0.0, "both: neither");
        assert!(
            !holding(&[l1], [0.0; 2]).input().fine,
            "L1 is not fine control"
        );
    }
}
