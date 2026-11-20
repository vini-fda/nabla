//! The canvas: the scene, the camera's dragging and zooming, pointing to steer, its buttons,
//! and what is drawn over the flight (the aim, the held heading, the off-screen goal, the pads).

use log::Level;
use nabla_core::State;
use nabla_sim::span;

use super::ViewerApp;
use super::scene::GOAL_COLOR;
use crate::camera::{Camera, Vec3};
use crate::canvas_buttons::{self, Button};
use crate::game::{self, Objective};
use crate::gamepad;
use crate::gizmo::ViewGizmo;
use crate::keys;
use crate::replay::Replay;
use crate::theme::{icons, palette};

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

/// Fraction of the canvas left empty around the ship and the goal by "Fit view".
const FIT_MARGIN: f32 = 0.15;

impl ViewerApp {
    /// The canvas. `inset` is how much of its right edge is taken, at the top, by a collapsed
    /// side panel's floating triangle: the gizmo and the buttons in the top right corner
    /// move left of it.
    pub(super) fn canvas_ui(&mut self, ui: &mut egui::Ui, inset: f32) {
        let _span = span!(Level::Trace, "canvas");
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
                    "Show the path flown in the last minute",
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
        );
        self.reserved = std::iter::once(buttons)
            .chain(ViewGizmo::area(corner).filter(|_| self.show_gizmo))
            .collect();
        if self.show_help {
            self.help_window(ui.ctx(), rect);
        }
    }

    /// Dragging and zooming the canvas, whose `response` this is, in `rect`. `on_pads`: a finger
    /// is on the flight's controls, or points, so another finger is not a pinch.
    pub(super) fn drag_and_zoom(
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
                let at = response
                    .interact_pointer_pos()
                    .unwrap_or_else(|| rect.center());
                self.camera.pan_by_screen(response.drag_delta(), at, rect);
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
            let frame_time = ui.input(|i| i.stable_dt).min(0.1);
            let factor = (zoom * CONTROLLER_ZOOM_RATE * frame_time).exp();
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
    pub(super) fn read_aim(
        &self,
        response: &egui::Response,
        rect: egui::Rect,
    ) -> Option<(egui::Pos2, f64)> {
        if self.assisted() {
            return None;
        }
        let game = &self.game;
        self.aim_from(
            response,
            rect,
            &game.state(),
            game.pads.touched(),
            game.pads.aim_touch(),
        )
    }

    /// [`Self::read_aim`] for the ship in state `s`. `touched`: a touch screen's pads are being
    /// touched, and `touch` is the touch that points, if any.
    pub(super) fn aim_from(
        &self,
        response: &egui::Response,
        rect: egui::Rect,
        s: &State,
        touched: bool,
        touch: Option<egui::Pos2>,
    ) -> Option<(egui::Pos2, f64)> {
        let ship = self.camera.local(s.x, s.y);
        let on_screen = self.camera.world_to_screen(ship, rect)?;
        let pointer = if !self.pointing_now() {
            None
        } else if touched {
            touch
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
        if game.objective() == Objective::ReachTarget {
            self.draw_goal_marker(painter, rect, marker_area, &game.replay, &state);
        }
        if pads {
            game.pads.paint(painter, rect);
        }
        self.notifications.paint(painter, rect);
    }

    /// Where the player points, from the ship in state `s`: a line to the `pointer`, which
    /// points at `heading`, and the edges of the thrust cone around it, as long. Green while
    /// the ship faces the pointer within the cone, and so thrusts.
    pub(super) fn draw_aim(
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
        let center = self.camera.local(s.x, s.y);
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
        let center = self.camera.local(s.x, s.y);
        let tip = center + egui::Vec2::angled(heading as f32) * LENGTH * self.circumradius;
        let ends = [center, tip].map(|p| self.camera.world_to_screen(p, rect));
        if let [Some(from), Some(to)] = ends {
            painter.line_segment([from, to], egui::Stroke::new(1.0, color));
            painter.circle_stroke(to, RING_RADIUS_PX, egui::Stroke::new(1.5, color));
        }
    }

    /// Where the goal is while it is off the canvas: a triangle at the edge of `area` (of the
    /// canvas) pointing at it, with the distance to it from the ship, in state `s`.
    pub(super) fn draw_goal_marker(
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
            .world_to_screen(self.camera.local(task.goal.x, task.goal.y), rect)
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
            format!("Target {distance:.1} m"),
            egui::FontId::monospace(12.0),
            GOAL_COLOR,
        );
    }
}

/// Center the camera on `bounds` (in the plane) and zoom so they fill the canvas, as seen
/// from the camera's side, with a margin.
pub(super) fn fit_camera(camera: &mut Camera, bounds: ([f64; 2], [f64; 2]), rect: egui::Rect) {
    let (lo, hi) = bounds;
    camera.look_at(f64::midpoint(lo[0], hi[0]), f64::midpoint(lo[1], hi[1]));
    // The bounds' extent across and up the screen, in world units.
    let [right, up, _] = camera.orbit.basis();
    let corners = [
        [lo[0], lo[1]],
        [hi[0], lo[1]],
        [lo[0], hi[1]],
        [hi[0], hi[1]],
    ]
    .map(|[x, y]| Vec3::plane(camera.local(x, y)));
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
