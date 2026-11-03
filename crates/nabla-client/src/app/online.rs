//! The online screen: connecting to a server, the lobby where the pilot picks a ship, and the
//! flight among other pilots.
//!
//! The flight itself, with its prediction, smoothing and interpolation, is in
//! [`crate::online`]. This is what draws it, and what the pilot presses.

use std::ops::RangeInclusive;

use nabla_net::client::Stage;
use nabla_net::link::Address;

use super::{
    Aim, GridStyle, Input, ViewerApp, canvas_buttons, draw_grid, fit_camera, icons, keys, palette,
    screen_path, world,
};
use crate::camera::Vec3;
use crate::canvas_buttons::Button;
use crate::gamepad;
use crate::net;
use crate::online::{Hands, Online, View};
use crate::ship3d::{self, DepthMap};

/// The server the address field starts with.
pub(super) const DEFAULT_SERVER: &str = "https://localhost:4433";
/// A server named in the page's address, as in `?server=https%3A%2F%2Fhost%3A4433%23sha256%3D...`:
/// the address field starts with it. `connect` in the query connects at once.
#[cfg(target_arch = "wasm32")]
pub(super) fn server_from_page() -> Option<(String, bool)> {
    let query = web_sys::window()?.location().search().ok()?;
    let mut server = None;
    let mut connect = false;
    for pair in query.trim_start_matches('?').split('&') {
        if let Some(value) = pair.strip_prefix("server=") {
            server = js_sys::decode_uri_component(value)
                .ok()
                .and_then(|text| text.as_string());
        } else if pair == "connect" || pair == "connect=1" {
            connect = true;
        }
    }
    server.map(|server| (server, connect))
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn server_from_page() -> Option<(String, bool)> {
    None
}

/// How much of the canvas the first view of the flight frames around the ship, in world units.
const FIRST_VIEW_HALF_SIZE: f64 = 25.0;
/// The arena's edge.
const ARENA_COLOR: egui::Color32 = palette::RED;

impl ViewerApp {
    /// Connect to the server in the address field, and show the online screen.
    pub(super) fn connect_online(&mut self, ctx: &egui::Context) {
        self.online_error = None;
        let address = match Address::parse(self.server_address.trim()) {
            Ok(address) => address,
            Err(why) => {
                self.online_error = Some(why);
                return;
            }
        };
        match net::connect(&address) {
            Ok(transport) => {
                log::info!("Connecting to {address}");
                self.online = Some(Online::new(transport, self.join_request));
                self.screen = super::Screen::Online;
                self.online_fit = true;
                self.follow_ship = true;
                ctx.request_repaint();
            }
            Err(why) => self.online_error = Some(why),
        }
    }

    /// Back to the title screen, and the connection closed.
    fn disconnect_online(&mut self) {
        self.online = None;
        self.online_levels = None;
        self.screen = super::Screen::Title;
        self.notifications.clear();
    }

    /// The controller's buttons, as in a single-player flight, but for pause, which a server
    /// does not have: North toggles the stability assist, West the autopilot, and Select (Create)
    /// shows or hides the controller's guide.
    fn online_controller_buttons(&mut self) {
        let Some(pad) = self.gamepads.state().cloned() else {
            return;
        };
        if pad.just_pressed(gamepad::Button::North) {
            self.toggle_steering();
        }
        let orbit_handled = self.orbit_buttons(&pad);
        if pad.just_pressed(gamepad::Button::West) && !orbit_handled {
            self.engage_objective();
        }
        if pad.just_pressed(gamepad::Button::Select) {
            self.toggle_controller_guide();
        }
    }

    /// What the pilot holds now, on the keyboard, the controller, and the pointer.
    fn online_hands(&mut self, ui: &egui::Ui) -> Hands {
        let controller = self
            .gamepads
            .state()
            .map_or_else(Input::default, gamepad::State::input);
        let mut input = Self::keyboard_input(ui).or(controller);
        input.aim = self.aiming.map(|(_, heading)| Aim {
            heading,
            cone: self.aim_cone.to_radians(),
        });
        self.input_thrust = input.thrust;
        Hands {
            input,
            steering: self.steering,
        }
    }

    pub(super) fn online_ui(&mut self, ui: &mut egui::Ui) {
        if ui.ctx().memory(|m| m.focused().is_none()) {
            let plain = |key| ui.input(|i| i.key_pressed(key) && i.modifiers.is_none());
            if plain(egui::Key::F) {
                self.follow_ship = !self.follow_ship;
            }
            if plain(egui::Key::T) {
                self.toggle_steering();
            }
            if plain(egui::Key::A) {
                self.toggle_autopilot();
            }
        }
        self.online_controller_buttons();
        let hands = self.online_hands(ui);
        // Flying, and with a predicted state to take the orbit from.
        let playing = self
            .online
            .as_ref()
            .is_some_and(|o| o.stage() == Stage::Playing && o.next_state().is_some());
        if playing && !self.online_ready {
            // The ship is new: its nearest body is the orbit to offer, and B is the objective.
            self.reset_orbit_controls();
            self.choose_objective(super::Objective::ReachB);
        }
        self.online_ready = playing;
        let Some(online) = &mut self.online else {
            self.screen = super::Screen::Title;
            return;
        };
        online.frame(hands);
        if let Some(fault) = online.take_fault() {
            self.notifications
                .post(format!("Flight assist switched off: {fault}"), palette::RED);
        }
        if online.take_landed() {
            self.notifications
                .post("The autopilot landed at B", palette::GREEN);
        }
        let stage = online.stage();
        let closed = online.closed().map(str::to_owned);
        // Polled for what the network brought, a frame at a time.
        if stage == Stage::Playing {
            ui.ctx().request_repaint();
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        if stage != Stage::Playing {
            self.online_levels = None;
        }
        if stage == Stage::Playing {
            self.refresh_orbit_prediction();
            self.refresh_prediction();
        }
        match (stage, closed) {
            (Stage::Playing, None) => self.online_flight_ui(ui),
            (_, closed) => {
                egui::CentralPanel::default().show(ui, |ui| {
                    self.lobby_ui(ui, stage, closed.as_deref());
                });
            }
        }
        self.notifications
            .paint(ui.painter(), ui.ctx().content_rect());
        if self.notifications.any() {
            ui.ctx().request_repaint();
        }
    }

    /// Connecting, the lobby, and the reasons a client is turned away.
    fn lobby_ui(&mut self, ui: &mut egui::Ui, stage: Stage, closed: Option<&str>) {
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.heading("Nabla online");
            ui.label(
                egui::RichText::new(self.server_address.split('#').next().unwrap_or_default())
                    .monospace()
                    .color(palette::OVERLAY1),
            );
            ui.add_space(12.0);
            let width = ui.available_width().min(460.0);
            ui.allocate_ui(egui::vec2(width, 0.0), |ui| {
                ui.set_width(width);
                if let Some(why) = closed {
                    ui.colored_label(palette::RED, format!("The connection ended: {why}"));
                } else {
                    match stage {
                        Stage::Connecting => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Connecting");
                            });
                        }
                        Stage::Joining => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Taking a ship");
                            });
                        }
                        Stage::Full => {
                            ui.colored_label(palette::YELLOW, "The server is full.");
                            ui.label("Every place is taken. Try again in a moment.");
                        }
                        Stage::Rejected => {
                            ui.colored_label(palette::RED, "The server turned this client away.");
                        }
                        Stage::Lobby | Stage::Playing => self.ship_picker_ui(ui),
                    }
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if matches!(stage, Stage::Full) && ui.button("Ask again").clicked() {
                        self.rejoin_online();
                    }
                    if ui.button("Back to the title screen").clicked() {
                        self.disconnect_online();
                    }
                });
            });
        });
    }

    /// The ship the pilot asks for, within what the world allows, and the button to take it.
    fn ship_picker_ui(&mut self, ui: &mut egui::Ui) {
        let Some(online) = &self.online else { return };
        let Some(world) = online.world() else { return };
        let ships = world.ships.unwrap_or_default();
        ui.label(format!("World: {}", world.name));
        if let Some(lobby) = online.lobby() {
            ui.label(format!(
                "{} of {} places taken",
                lobby.pilots, lobby.max_pilots
            ));
        }
        ui.add_space(8.0);
        ui.strong("Your ship");
        let request = &mut self.join_request;
        egui::Grid::new("ship_request")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                let row = |ui: &mut egui::Ui, name: &str, value: &mut f64, range: [f64; 2]| {
                    ui.label(name);
                    ui.add(
                        egui::DragValue::new(value)
                            .range(RangeInclusive::new(range[0], range[1]))
                            .speed(0.01 * (range[1] - range[0]))
                            .max_decimals(2),
                    );
                    ui.end_row();
                };
                row(ui, "mass", &mut request.mass, ships.mass);
                row(ui, "moment of inertia", &mut request.inertia, ships.inertia);
                row(ui, "maximum thrust", &mut request.f_max, ships.f_max);
                row(ui, "maximum torque", &mut request.tau_max, ships.tau_max);
            });
        let granted = world.grant(request);
        let (a, beta) = (
            granted.f_max / granted.mass,
            granted.tau_max / granted.inertia,
        );
        ui.label(
            egui::RichText::new(format!(
                "Acceleration {a:.1}, angular acceleration {beta:.1}. The server's caps are {} and {}, \
                 and it grants the nearest ship it allows.",
                ships.acceleration_cap, ships.angular_cap
            ))
            .small()
            .color(palette::OVERLAY1),
        );
        ui.add_space(8.0);
        if ui
            .add(egui::Button::new("Take this ship").fill(palette::GREEN))
            .clicked()
        {
            self.rejoin_online();
        }
    }

    fn rejoin_online(&mut self) {
        let request = self.join_request;
        if let Some(online) = &mut self.online {
            online.join(request);
        }
        self.online_fit = true;
    }

    /// The flight: a bar along the bottom, the settings panel on the left, the plots on the
    /// right, and the canvas.
    fn online_flight_ui(&mut self, ui: &mut egui::Ui) {
        let Some(online) = &mut self.online else {
            return;
        };
        let local = online.local_time();
        let views = online.views(local);
        egui::Panel::bottom("online_bar").show(ui, |ui| self.online_bar_ui(ui));
        let mut show_controls = self.show_controls;
        super::CONTROLS_PANEL.show(ui, &mut show_controls, &mut 0, |ui, _| {
            self.online_controls_ui(ui);
        });
        self.show_controls = show_controls;
        let mut show_plots = self.show_plots;
        let mut view = self.right_view as usize;
        let reveal = super::RIGHT_PANEL.show(ui, &mut show_plots, &mut view, |ui, view| {
            match super::RightView::ALL[view] {
                super::RightView::Plots => {
                    if let Some(own) = views.first().filter(|view| view.own) {
                        Self::plots_of(ui, &own.replay, own.t);
                    }
                }
                super::RightView::Objective => self.objective_ui(ui),
            }
        });
        self.right_view = super::RightView::ALL[view];
        self.show_plots = show_plots;
        egui::CentralPanel::default().show(ui, |ui| {
            self.online_canvas_ui(ui, super::canvas_inset(reveal), &views);
        });
    }

    /// The settings panel of a flight on a server: who is flying, and how the canvas looks.
    fn online_controls_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::both().show(ui, |ui| {
            ui.heading("Online");
            if let Some(online) = &self.online {
                if let Some(world) = online.world() {
                    ui.label(format!("World: {}", world.name));
                }
                let info = online.info();
                ui.label(format!(
                    "{} {} in the world",
                    info.ships,
                    if info.ships == 1 { "ship" } else { "ships" }
                ));
                if let Some(tick) = info.tick {
                    ui.monospace(format!("tick {tick}"));
                }
            }
            ui.separator();
            ui.heading("Display options");
            ui.add_enabled_ui(self.ship3d.is_some(), |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.view, super::CanvasView::Ship3d, "3D");
                    ui.selectable_value(&mut self.view, super::CanvasView::Debug, "Debug")
                        .on_hover_text("Flat ships");
                });
            });
            ui.checkbox(&mut self.show_grid, "Grid");
            ui.add_enabled_ui(!self.debug_view(), |ui| {
                ui.checkbox(&mut self.show_plume, "Thruster plumes");
            });
            ui.horizontal(|ui| {
                if ui.button("Fit view").clicked() {
                    self.online_fit = true;
                }
                if ui.button("Reset view").clicked() {
                    self.camera = crate::camera::Camera::default();
                    self.gizmo.stop();
                    self.online_fit = true;
                }
            });
            ui.separator();
            self.sound.ui(ui);
        });
    }

    fn online_bar_ui(&mut self, ui: &mut egui::Ui) {
        let (mut home, mut leave, mut sas, mut autopilot) = (false, false, false, false);
        ui.horizontal(|ui| {
            home = super::outline_button(ui, icons::outline::HOUSE)
                .on_hover_text("Disconnect, and back to the title screen")
                .clicked();
            leave = ui
                .button("Leave")
                .on_hover_text("Give the ship up, and go back to the lobby")
                .clicked();
            ui.separator();
            let mut on = self.steering == super::Steering::Sas;
            sas = ui
                .toggle_value(&mut on, "SAS")
                .on_hover_ui(|ui| {
                    keys::tooltip(
                        ui,
                        "Stability assist: when you stop turning, stop the spin and hold the heading",
                        &[keys::T],
                    );
                })
                .changed();
            let assisted = self.online.as_ref().is_some_and(Online::assisted);
            let mut engaged = assisted;
            autopilot = ui
                .toggle_value(&mut engaged, "Autopilot")
                .on_hover_ui(|ui| {
                    let action = format!(
                        "Autopilot: {}. Switch it off to take over",
                        self.flight_objective().label()
                    );
                    keys::tooltip(ui, &action, &[keys::A]);
                })
                .changed();
            ui.add_enabled_ui(self.follow_ship && !assisted, |ui| {
                ui.toggle_value(&mut self.pointing, "Point")
                    .on_hover_text(
                        "Hold the mouse or a finger on the canvas: the ship turns toward it, \
                         and thrusts once it faces it within the cone",
                    );
            });
            ui.separator();
            if let Some(online) = &self.online {
                let info = online.info();
                let rtt = info
                    .rtt
                    .map_or_else(|| "-".to_owned(), |rtt| format!("{:.0} ms", rtt * 1e3));
                let margin = info
                    .margin_error
                    .map_or_else(|| "-".to_owned(), |e| format!("{:+.1} ms", e * 1e3));
                let c = info.corrections;
                ui.monospace(format!(
                    "ping {rtt}  margin {margin}  ships {}  correction {:.0e} (max)  jumps {}",
                    info.ships, c.max_position, c.jumps
                ));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                super::controller_status_ui(ui, &self.gamepads);
            });
        });
        if home {
            self.disconnect_online();
        }
        if leave {
            if let Some(online) = &mut self.online {
                online.leave();
            }
            self.online_levels = None;
        }
        if sas {
            self.toggle_steering();
        }
        if autopilot {
            self.toggle_autopilot();
        }
    }

    fn online_canvas_ui(&mut self, ui: &mut egui::Ui, inset: f32, views: &[View]) {
        let (rect, response) =
            ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        // Where the gizmo and the buttons go, in its top right corner.
        let corner = rect.with_max_x(rect.right() - inset);
        let Some(own) = views.first().filter(|view| view.own) else {
            ui.painter_at(rect)
                .rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
            return;
        };
        self.online_levels = Some((
            own.replay.thrust_fraction(own.t) as f32,
            own.replay.torque_fraction(own.t) as f32,
        ));
        self.gizmo
            .input(ui, corner, &mut self.camera, self.show_gizmo);

        self.drag_and_zoom(ui, &response, rect, false);
        if self.online_fit {
            let (x, y) = (own.state.x, own.state.y);
            let half = FIRST_VIEW_HALF_SIZE;
            fit_camera(
                &mut self.camera,
                ([x - half, y - half], [x + half, y + half]),
                rect,
            );
            self.online_fit = false;
        }
        let painter = ui.painter_at(rect);
        self.draw_online_scene(ui, &painter, rect, views);
        self.draw_orbits(&painter, rect);
        self.draw_prediction(&painter, rect);
        if self.show_path {
            let k = (own.t / own.replay.h()).floor() as usize;
            self.draw_path(
                &painter,
                rect,
                &own.replay,
                k.min(own.replay.last_frame()),
                &own.state,
            );
        }
        self.aiming = self.online_aim(&response, rect, &own.state);
        if let Some((pointer, heading)) = self.aiming {
            self.draw_aim(&painter, rect, &own.state, pointer, heading);
        }
        if let Some(online) = &self.online
            && online.assisted()
        {
            self.draw_goal(&painter, rect, &own.replay, &own.state);
            self.draw_goal_marker(&painter, rect, rect, &own.replay, &own.state);
        }
        if self.show_gizmo {
            self.gizmo.paint(ui, &painter, corner, &self.camera);
        }
        let buttons = self.online_canvas_buttons(ui, &painter, corner);
        self.reserved = std::iter::once(buttons)
            .chain(crate::gizmo::ViewGizmo::area(corner).filter(|_| self.show_gizmo))
            .collect();
        if self.show_help {
            self.help_window(ui.ctx(), rect);
        }
    }

    /// The canvas's buttons, in its top right `corner`: where they are drawn.
    fn online_canvas_buttons(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        corner: egui::Rect,
    ) -> egui::Rect {
        canvas_buttons::show(
            ui,
            painter,
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
                    "Show the path flown in the last half minute",
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
                    "User inputs: how to fly. The controller's Create button shows it too",
                )
                .color(palette::YELLOW),
            ],
        )
    }

    /// Where the player points, as [`ViewerApp::read_aim`] has it, for the ship in `state`.
    fn online_aim(
        &self,
        response: &egui::Response,
        rect: egui::Rect,
        state: &nabla_core::State,
    ) -> Option<(egui::Pos2, f64)> {
        if self.online.as_ref().is_some_and(Online::assisted) {
            return None;
        }
        self.aim_from(response, rect, state, false, None)
    }

    /// The arena, the world's bodies, and the ships with their plumes.
    fn draw_online_scene(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        rect: egui::Rect,
        views: &[View],
    ) {
        let Some(online) = &self.online else { return };
        let Some(world_data) = online.world() else {
            return;
        };
        let bodies = world_data.bodies.clone();
        let half_side = world_data.arena.map_or(0.0, |arena| arena.half_side);
        let hull = world_data.hull_radius();
        let debug = self.debug_view();
        let style = if debug {
            GridStyle::from_visuals(ui.visuals())
        } else {
            GridStyle::faint(ui.visuals())
        };
        let own = views[0].state;
        if self.follow_ship {
            self.camera.center = Vec3::plane(world(own.x, own.y));
        }
        if self.show_grid {
            draw_grid(painter, &self.camera, rect, &style);
        }
        // The arena's wall.
        let corner = |x: f64, y: f64| Vec3::plane(world(x, y));
        let wall = [
            (-half_side, -half_side),
            (half_side, -half_side),
            (half_side, half_side),
            (-half_side, half_side),
            (-half_side, -half_side),
        ];
        for run in screen_path(&self.camera, rect, wall.iter().map(|&(x, y)| corner(x, y))) {
            painter.add(egui::Shape::line(
                run,
                egui::Stroke::new(2.0_f32, ARENA_COLOR),
            ));
        }
        if debug {
            crate::planets::draw_debug(painter, &self.camera, rect, &bodies);
            for view in views {
                self.draw_ship_sized(painter, rect, &view.state, hull);
            }
            return;
        }
        let Some((renderer, plume)) = &self.ship3d else {
            return;
        };
        let ships: Vec<_> = views
            .iter()
            .map(|view| (world(view.state.x, view.state.y), view.state.theta as f32))
            .collect();
        let positions: Vec<_> = ships.iter().map(|&(position, _)| position).collect();
        let depth = DepthMap::for_world(renderer, &self.camera, rect, &positions, hull, &bodies);
        painter.add(crate::planets::paint(
            &self.camera,
            rect,
            &depth,
            &bodies,
            renderer.gamma_output,
        ));
        if let Some(callback) =
            ship3d::paint_callback(renderer, &self.camera, rect, &ships, hull, &depth)
        {
            painter.add(callback);
        }
        // After the ships, whose depth they are tested against.
        if self.show_plume
            && let Some(callback) = plume.paint(
                &self.camera,
                rect,
                views.iter().map(|view| (&view.replay, view.t)),
                hull,
                &depth,
            )
        {
            painter.add(callback);
        }
    }
}
