//! Navigation gizmo, as in Blender's viewport: the world axes as the camera sees them, in
//! the canvas's top right corner. Drag it to orbit the camera; click an axis's ball to look
//! back along that axis from its side (click it again to look from the opposite side). The
//! **Top-down view** button under it returns to the default view. Turns to a clicked view
//! are animated.

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};

use crate::camera::{Camera, Orbit, Vec3};
use crate::theme::palette;

/// Distance from the gizmo's center to an axis's ball, and the ball's radius, in pixels.
const ARM: f32 = 36.0;
const BALL: f32 = 10.0;
/// Space between the gizmo and the canvas's top and right edges. On the right, it clears
/// a collapsed side panel's title bar.
const MARGIN_TOP: f32 = 12.0;
const MARGIN_RIGHT: f32 = 42.0;
/// Radians the camera turns per pixel dragged.
const ORBIT_PER_PIXEL: f32 = 0.01;
/// How fast a clicked view is reached: the remaining turn shrinks by `e` this many times a
/// second.
const TURN_RATE: f32 = 14.0;

/// The axes: direction, label, color.
const AXES: [(Vec3, &str, Color32); 3] = [
    (Vec3::X, "X", palette::RED),
    (Vec3::Y, "Y", palette::GREEN),
    (Vec3::Z, "Z", palette::BLUE),
];

/// One of the six balls: axis `axis` of [`AXES`], its positive end or its negative one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Ball {
    axis: usize,
    positive: bool,
}

impl Ball {
    fn all() -> impl Iterator<Item = Self> {
        (0..3).flat_map(|axis| [true, false].map(|positive| Self { axis, positive }))
    }

    fn direction(self) -> Vec3 {
        let (direction, _, _) = AXES[self.axis];
        if self.positive { direction } else { -direction }
    }

    /// Where the ball is drawn, around the gizmo's `center`, and how near the viewer it is
    /// (-1 to 1).
    fn placement(self, center: Pos2, orbit: Orbit) -> (Pos2, f32) {
        let [right, up, toward_viewer] = orbit.basis();
        let d = self.direction();
        (
            center + Vec2::new(d.dot(right), -d.dot(up)) * ARM,
            d.dot(toward_viewer),
        )
    }
}

#[derive(Default)]
pub struct ViewGizmo {
    /// The view being turned to, after a click.
    target: Option<Orbit>,
    /// The ball under the pointer, and whether the pointer is over the gizmo or dragging it.
    hovered: Option<Ball>,
    active: bool,
}

impl ViewGizmo {
    /// Where the gizmo's center is in `canvas`, if the canvas has room for it.
    fn center(canvas: Rect) -> Option<Pos2> {
        let reach = ARM + BALL;
        let center = Pos2::new(
            canvas.right() - MARGIN_RIGHT - reach,
            canvas.top() + MARGIN_TOP + reach,
        );
        let room = canvas.width() > 2.0 * (MARGIN_RIGHT + 2.0 * reach)
            && canvas.height() > 2.0 * (MARGIN_TOP + 2.0 * reach);
        room.then_some(center)
    }

    /// Handles the gizmo's input and turns `camera`. Call it before anything is drawn with
    /// the camera, so the frame shows the new view, and [`Self::paint`] after, so the
    /// gizmo is drawn over the scene.
    pub fn input(&mut self, ui: &egui::Ui, canvas: Rect, camera: &mut Camera) {
        self.hovered = None;
        self.active = false;
        if let Some(center) = Self::center(canvas) {
            let rect = Rect::from_center_size(center, Vec2::splat(2.0 * (ARM + BALL)));
            let response = ui.interact(
                rect,
                ui.scope_id().with("view_gizmo"),
                Sense::click_and_drag(),
            );
            self.hovered = response
                .hover_pos()
                .and_then(|pointer| ball_at(center, camera.orbit, pointer));
            self.active = response.hovered() || response.dragged();

            if response.dragged() {
                self.target = None;
                let delta = response.drag_delta() * ORBIT_PER_PIXEL;
                camera.orbit = Orbit {
                    azimuth: camera.orbit.azimuth - delta.x,
                    elevation: camera.orbit.elevation + delta.y,
                }
                .normalized();
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if self.hovered.is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            } else if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            if response.clicked()
                && let Some(ball) = self.hovered
            {
                self.target = Some(view_for_click(camera.orbit, ball.direction()));
            }
        }
        self.animate(ui.ctx(), camera);
    }

    /// Turns `camera` toward the target view, if any.
    fn animate(&mut self, ctx: &egui::Context, camera: &mut Camera) {
        let Some(target) = self.target else {
            return;
        };
        let dt = ctx.input(|i| i.stable_dt).min(0.1);
        camera.orbit = camera.orbit.toward(target, 1.0 - (-TURN_RATE * dt).exp());
        if camera.orbit.is_near(target, 1e-3) {
            camera.orbit = target;
            self.target = None;
        } else {
            ctx.request_repaint();
        }
    }

    /// Stops any turn under way, for when the camera is set from elsewhere.
    pub fn stop(&mut self) {
        self.target = None;
    }

    /// Paints the gizmo for `camera` over the canvas, with the button back to the top-down
    /// view.
    pub fn paint(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        canvas: Rect,
        camera: &Camera,
    ) {
        let Some(center) = Self::center(canvas) else {
            return;
        };
        let orbit = camera.orbit;
        if self.active {
            painter.circle_filled(
                center,
                ARM + BALL + 4.0,
                ui.visuals().text_color().gamma_multiply(0.08),
            );
        }

        // Back to front.
        let mut balls: Vec<(Ball, Pos2, f32)> = Ball::all()
            .map(|ball| {
                let (position, depth) = ball.placement(center, orbit);
                (ball, position, depth)
            })
            .collect();
        balls.sort_by(|a, b| a.2.total_cmp(&b.2));
        for (ball, position, _) in balls {
            let (_, label, color) = AXES[ball.axis];
            let hovered = self.hovered == Some(ball);
            if ball.positive {
                painter.line_segment([center, position], Stroke::new(2.0, color));
                painter.circle_filled(position, BALL, color);
                painter.text(
                    position,
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::FontId::proportional(12.0),
                    palette::SURFACE0,
                );
            } else {
                // Opaque, so it hides the arms behind it: a dim tint of the axis's color
                // over the canvas's.
                let fill = ui.visuals().panel_fill.lerp_to_gamma(color, 0.25);
                painter.circle(position, BALL - 0.75, fill, Stroke::new(1.5, color));
                if hovered {
                    painter.text(
                        position,
                        egui::Align2::CENTER_CENTER,
                        format!("-{label}"),
                        egui::FontId::proportional(10.0),
                        palette::WHITE,
                    );
                }
            }
            if hovered {
                painter.circle_stroke(position, BALL + 1.5, Stroke::new(1.5, palette::WHITE));
            }
        }

        let button = Rect::from_center_size(
            center + Vec2::new(0.0, ARM + BALL + 18.0),
            Vec2::new(100.0, 20.0),
        );
        let top_down = orbit.is_near(Orbit::TOP, 1e-4) && self.target.is_none();
        let response = ui
            .put(
                button,
                egui::Button::new("Top-down view")
                    .small()
                    .wrap_mode(egui::TextWrapMode::Extend)
                    .selected(top_down)
                    .min_size(button.size()),
            )
            .on_hover_text("Back to the default view, looking straight down at the plane");
        if response.clicked() {
            self.target = Some(Orbit::TOP);
            ui.ctx().request_repaint();
        }
    }
}

/// The ball under `pointer` in the gizmo centered on `center`, the nearest the viewer if
/// several overlap.
fn ball_at(center: Pos2, orbit: Orbit, pointer: Pos2) -> Option<Ball> {
    Ball::all()
        .map(|ball| (ball, ball.placement(center, orbit)))
        .filter(|(_, (position, _))| position.distance(pointer) <= BALL)
        .max_by(|(_, (_, a)), (_, (_, b))| a.total_cmp(b))
        .map(|(ball, _)| ball)
}

/// The view a click on the ball of `direction` turns to: from that side, or from the
/// opposite side when already looking from that one.
fn view_for_click(current: Orbit, direction: Vec3) -> Orbit {
    if current.toward_viewer().dot(direction) > 0.9999 {
        Orbit::facing(-direction)
    } else {
        Orbit::facing(direction)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_view_shows_x_right_and_y_up_with_z_in_front() {
        let center = Pos2::new(100.0, 100.0);
        let x = Ball {
            axis: 0,
            positive: true,
        };
        let y = Ball {
            axis: 1,
            positive: true,
        };
        let z = Ball {
            axis: 2,
            positive: true,
        };
        let (px, _) = x.placement(center, Orbit::TOP);
        let (py, _) = y.placement(center, Orbit::TOP);
        let (pz, depth) = z.placement(center, Orbit::TOP);
        assert!(px.x > center.x + ARM - 1e-3);
        assert!(py.y < center.y - ARM + 1e-3);
        assert!((pz - center).length() < 1e-3 && depth > 0.999);
        // +Z covers -Z, right behind it.
        assert_eq!(ball_at(center, Orbit::TOP, center), Some(z));
    }

    #[test]
    fn clicking_an_axis_looks_from_its_side_then_the_other() {
        let from_x = view_for_click(Orbit::TOP, Vec3::X);
        assert!(from_x.is_near(Orbit::facing(Vec3::X), 1e-6));
        let back = view_for_click(from_x, Vec3::X);
        assert!(back.is_near(Orbit::facing(-Vec3::X), 1e-6));
        // From above, clicking Z looks from below.
        assert!(view_for_click(Orbit::TOP, Vec3::Z).is_near(Orbit::facing(-Vec3::Z), 1e-6));
    }
}
