//! The scene on the canvas: the gravity field, the grid, the bodies, the path flown, the goal
//! and the ship, flat or in 3D, and the collision meshes.

use nabla_core::State;

use super::{CanvasView, ViewerApp};
use crate::camera::{Camera, Vec3};
use crate::game::Objective;
use crate::grid::{GridStyle, draw_grid};
use crate::replay::Replay;
use crate::ship3d::{self, DepthMap};
use crate::theme::palette;

const TRIANGLE_STROKE_WIDTH: f32 = 2.0;

/// The velocity and acceleration arrows are drawn in the world, from the ship: the velocity
/// arrow's length is the velocity times this time `dt`, and the acceleration arrow's is the
/// acceleration times `dt^2`, both in world units.
const ARROW_SECONDS: f64 = 1.0;
const ARROW_STROKE_WIDTH: f32 = 2.0;
/// The length of a solid arrowhead along the arrow, and its width across it, in pixels; at
/// most half the arrow's length, for a short arrow.
const ARROWHEAD_LENGTH: f32 = 12.0;
const ARROWHEAD_WIDTH: f32 = 9.0;
const ARROW_MIN_PIXELS: f32 = 4.0;

const VELOCITY_COLOR: egui::Color32 = palette::GREEN;
const ACCELERATION_COLOR: egui::Color32 = palette::RED;

const PATH_COLOR: egui::Color32 = palette::OVERLAY0;
const TRAVELED_COLOR: egui::Color32 = palette::BLUE;
const PATH_MIN_SEGMENT_PX: f32 = 0.5;

pub(super) const GOAL_COLOR: egui::Color32 = palette::YELLOW;

const GOAL_RADIUS_PX: f32 = 10.0;
const GOAL_LABEL_OFFSET_PX: f32 = 12.0;
const START_RADIUS_PX: f32 = 5.0;

impl ViewerApp {
    /// Everything on the canvas but the gizmo.
    pub(super) fn draw_scene(&mut self, ui: &egui::Ui, painter: &egui::Painter, rect: egui::Rect) {
        let debug = self.debug_view();
        let style = if debug {
            GridStyle::from_visuals(ui.visuals())
        } else {
            GridStyle::faint(ui.visuals())
        };
        let (_, _, state) = self.now();

        // Follow mode: keep the ship centered. Done after pan/zoom so zoom
        // still adjusts scale around the (now ship-centered) view.
        let bodies = self.place_camera(&state, &self.game.world.bodies.clone());

        if self.show_field {
            self.draw_field(ui, painter, rect, debug);
        }
        // Warm the field's contour cache while the update can still touch it: the 3D view
        // draws the lines after the planets and ships, which test their depth, reading the
        // cache then.
        if self.show_field && !debug && self.game.world.gravity_constant > 0.0 {
            crate::field::contours(
                &self.camera,
                rect,
                &bodies,
                self.game.world.gravity_constant,
                &mut self.field_contours,
            );
        }
        // The field's lines likewise: traced here, when the world has changed, and drawn at
        // once in the flat view, under the grid, where nothing hides them.
        if self.show_streamlines && self.game.world.gravity_constant > 0.0 {
            crate::streamlines::streamlines(
                &self.game.world.bodies,
                self.game.world.gravity_constant,
                &mut self.streamlines,
            );
            if debug {
                self.draw_streamlines(ui, painter, rect, None);
            }
        }
        if self.show_grid {
            draw_grid(painter, &self.camera, rect, &style);
        }
        let (replay, k, offset) = self.shown();
        if debug {
            crate::planets::draw_debug(painter, &self.camera, rect, &bodies);
        }
        match &self.ship3d {
            Some((renderer, plume)) if !debug => {
                let position = self.camera.local(state.x, state.y);
                let depth = DepthMap::for_world(
                    renderer,
                    &self.camera,
                    rect,
                    &[position],
                    self.circumradius,
                    &bodies,
                );
                painter.add(crate::planets::paint(
                    &self.camera,
                    rect,
                    &depth,
                    &bodies,
                    renderer.gamma_output,
                ));
                if let Some(callback) = ship3d::paint_callback(
                    renderer,
                    &self.camera,
                    rect,
                    &[(position, state.theta as f32)],
                    self.circumradius,
                    &depth,
                ) {
                    painter.add(callback);
                }
                if self.show_streamlines {
                    self.draw_streamlines(ui, painter, rect, Some(&depth));
                }
                // After the ship, whose depth they are tested against.
                let t = replay.time(k) + offset;
                if let Some(callback) =
                    plume.paint(&self.camera, rect, [(replay, t)], self.circumradius, &depth)
                {
                    painter.add(callback);
                }
                // The marks on the plane, over everything whose depth they are tested
                // against, as their flat draws have them.
                self.draw_marks_3d(painter, rect, &depth, ui.ctx().pixels_per_point());
            }
            _ => self.draw_scene_flat(painter, rect, replay, k, &state),
        }
        if self.show_collision_meshes {
            self.draw_collision_meshes(
                painter,
                rect,
                std::slice::from_ref(&state),
                self.game.world.players[0].radius,
                &bodies,
            );
        }
    }

    /// Settle the camera for the frame: on the ship in `state` while following it, and with
    /// its floating origin near the view. `bodies` as the drawing takes them then, at their
    /// places from that origin.
    pub(super) fn place_camera(
        &mut self,
        state: &State,
        bodies: &[crate::world::CelestialBody],
    ) -> Vec<crate::world::CelestialBody> {
        if self.follow_ship {
            self.camera.look_at(state.x, state.y);
        }
        // Panning may have taken the view far from the floating origin.
        self.camera.rebase();
        let local = self.camera.local_bodies(bodies);
        self.camera.frame_bodies(&local);
        local
    }

    /// The flat view's draws, with no 3D renderer in front: the path flown, the goal, the
    /// ship with its velocity and acceleration arrows, the orbits and the prediction.
    fn draw_scene_flat(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        replay: &Replay,
        k: usize,
        state: &State,
    ) {
        if self.show_path {
            self.draw_path(painter, rect, replay, k, state);
        }
        if self.game.objective() == Objective::ReachTarget {
            self.draw_goal(painter, rect, replay, state);
        }
        self.draw_ship(painter, rect, state);
        self.draw_arrows(painter, rect, replay, k, state);
        self.draw_orbits(painter, rect);
        self.draw_prediction(painter, rect);
    }

    /// The colliders' own shapes, at the displayed (between-sample) position, drawn last so
    /// meshes remain visible through the 3D models: the bodies as the balls their colliders
    /// are, each ship as its hull, at the displayed heading.
    pub(super) fn draw_collision_meshes(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        states: &[State],
        hull: f32,
        bodies: &[crate::world::CelestialBody],
    ) {
        let stroke = egui::Stroke::new(2.0, palette::PEACH);
        crate::planets::draw_circles(
            painter,
            &self.camera,
            rect,
            bodies
                .iter()
                .map(|body| (egui::vec2(body.position[0], body.position[1]), body.radius)),
            stroke,
        );
        for state in states {
            draw_collision_hull(
                painter,
                &self.camera,
                rect,
                self.camera.local(state.x, state.y),
                state.theta as f32,
                hull,
                stroke,
            );
        }
    }

    /// The gravity field's lines, traced by `draw_scene` beforehand: glowing streamlines with
    /// arrowheads, and pulses falling along them. In the 3D view they are drawn after the
    /// planets and ships, and tested against their depth through the scene's own `depth`
    /// map; the flat view has none. The pulses move by the UI's clock, whether or not the
    /// flight does, so the canvas keeps repainting while they show.
    pub(super) fn draw_streamlines(
        &self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        rect: egui::Rect,
        depth: Option<&DepthMap>,
    ) {
        let Some(streamlines) = &self.streamlines else {
            return;
        };
        if self.flight_world().gravity_constant == 0.0 {
            return;
        }
        if let Some(callback) = crate::streamlines::paint(
            &self.camera,
            rect,
            streamlines,
            depth,
            ui.ctx().pixels_per_point(),
            ui.input(|i| i.time),
        ) {
            painter.add(callback);
            ui.ctx().request_repaint();
        }
    }

    /// Whether the canvas shows the debug view: when chosen, or when there is no 3D view.
    pub(super) fn debug_view(&self) -> bool {
        self.view == CanvasView::Debug || self.ship3d.is_none()
    }

    /// The bodies' gravity field on the canvas, under everything else: soft potential bands
    /// from the wgpu pass, then crisp equipotential contour lines, which the 3D view draws
    /// itself, depth-tested, after the planets and ships (see `draw_scene`). Only in a world
    /// with gravity.
    pub(super) fn draw_field(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        rect: egui::Rect,
        debug: bool,
    ) {
        let world = self.flight_world();
        let gravity = world.gravity_constant;
        if gravity == 0.0 {
            return;
        }
        let bodies = self.camera.local_bodies(&world.bodies);
        if let Some(callback) = crate::field::paint(&self.camera, rect, &bodies, gravity) {
            painter.add(callback);
        }
        if debug {
            // Cloned so the contour cache can be borrowed mutably alongside, with the bodies
            // at their places from the floating origin.
            let world = crate::world::World {
                bodies,
                ..self.flight_world().clone()
            };
            crate::field::draw_contours(
                painter,
                &self.camera,
                rect,
                &world,
                self.field_line_width,
                &mut self.field_contours,
                ui.ctx().pixels_per_point(),
            );
        }
    }

    /// The path held (a flight's last minute), that up to the ship's current (possibly
    /// between-samples) position, and where the run started, even once that path has moved on.
    pub(super) fn draw_path(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        replay: &Replay,
        k: usize,
        now: &State,
    ) {
        let point = |s: &State| Vec3::plane(self.camera.local(s.x, s.y));
        let full = screen_path(&self.camera, rect, replay.states().iter().map(point));
        let traveled = screen_path(
            &self.camera,
            rect,
            replay.states_up_to(k).chain([now]).map(point),
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

        let start = replay.case.start;
        if let Some(start) = self
            .camera
            .world_to_screen(self.camera.local(start.x, start.y), rect)
        {
            painter.circle_stroke(
                start,
                START_RADIUS_PX,
                egui::Stroke::new(1.5_f32, TRAVELED_COLOR),
            );
        }
    }

    pub(super) fn draw_goal(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        replay: &Replay,
        now: &State,
    ) {
        let task = &replay.case.task;
        let reached = task.reached(now);
        let color = GOAL_COLOR.gamma_multiply(if reached { 0.35 } else { 1.0 });

        // Position tolerance eps_p, drawn to scale once it is visible: a circle in the plane,
        // an ellipse once the view is tilted.
        let tolerance = task.tolerance.position as f32;
        if tolerance * self.camera.scale > 1.5 {
            let center = self.camera.local(task.goal.x, task.goal.y);
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
            .world_to_screen(self.camera.local(task.goal.x, task.goal.y), rect)
        else {
            return;
        };
        painter.circle_stroke(goal, GOAL_RADIUS_PX, egui::Stroke::new(2.0_f32, color));
        painter.circle_filled(goal, 2.0, color);
        painter.text(
            goal + egui::vec2(GOAL_LABEL_OFFSET_PX, -GOAL_LABEL_OFFSET_PX),
            egui::Align2::LEFT_BOTTOM,
            if reached {
                "Target (reached)"
            } else {
                "Target"
            },
            egui::FontId::monospace(12.0),
            color,
        );
    }

    /// The goal in the 3D view, as depth-tested marks of the marks' pass: the same shapes
    /// `draw_goal` gives egui in the flat view. The position tolerance as a circle
    /// in the plane, an ellipse once the view is tilted, drawn to scale once it is visible;
    /// the target itself as a billboard ring and dot at the goal, lying at the goal's own
    /// depth, so what stands in front of it hides it as one. Both `GOAL_COLOR`, faded once
    /// reached.
    pub(super) fn goal_strokes(
        &self,
        overlays: &mut crate::overlays::Overlays<'_>,
        replay: &Replay,
        now: &State,
    ) {
        let task = &replay.case.task;
        let reached = task.reached(now);
        let color = GOAL_COLOR.gamma_multiply(if reached { 0.35 } else { 1.0 });
        let tolerance = task.tolerance.position as f32;
        if tolerance * self.camera.scale > 1.5 {
            let center = self.camera.local(task.goal.x, task.goal.y);
            let circle = (0..64).map(|i| {
                let angle = i as f32 / 64.0 * std::f32::consts::TAU;
                Vec3::plane(center + egui::Vec2::angled(angle) * tolerance)
            });
            overlays.stroke(circle, true, 1.0, color);
        }
        let goal = self.camera.local(task.goal.x, task.goal.y);
        overlays.ring(goal, GOAL_RADIUS_PX, 2.0, color);
        overlays.disc(goal, 2.0, color);
    }

    /// The marks on the plane in the 3D view, over the planets, ships and plumes whose depth
    /// they are tested against: the field's contour lines, the path flown, the goal, the
    /// orbits and the prediction, as one pass of depth-tested ribbons (`crate::overlays`) --
    /// the same widths, colors and dash patterns their flat draws (`draw_contours`,
    /// `draw_path`, `draw_goal`, `draw_orbits`, `draw_prediction`) give egui where nothing
    /// hides them.
    fn draw_marks_3d(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        depth: &DepthMap,
        pixels_per_point: f32,
    ) {
        let mut overlays =
            crate::overlays::Overlays::new(&self.camera, rect, depth, pixels_per_point);
        if self.show_field {
            crate::field::contour_strokes(
                &mut overlays,
                &self.camera,
                &self.field_contours,
                self.field_line_width,
            );
        }
        let (replay, k, offset) = self.shown();
        let state = replay.state_at(k, offset);
        if self.show_path {
            path_strokes(&mut overlays, replay, k, &state);
        }
        if self.game.objective() == Objective::ReachTarget {
            self.goal_strokes(&mut overlays, replay, &state);
        }
        self.orbit_strokes(&mut overlays, rect);
        self.prediction_strokes(&mut overlays);
        if let Some(callback) = overlays.finish() {
            painter.add(callback);
        }
        if self.game.objective() == Objective::ReachTarget {
            self.draw_goal_label(painter, rect, replay, &state);
        }
    }

    /// The goal's label beside its mark: the one part of the goal that is not a mark on the
    /// plane but a text over the canvas, so it stays in front of everything drawn.
    pub(super) fn draw_goal_label(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        replay: &Replay,
        now: &State,
    ) {
        let task = &replay.case.task;
        let reached = task.reached(now);
        let color = GOAL_COLOR.gamma_multiply(if reached { 0.35 } else { 1.0 });
        let Some(goal) = self
            .camera
            .world_to_screen(self.camera.local(task.goal.x, task.goal.y), rect)
        else {
            return;
        };
        painter.text(
            goal + egui::vec2(GOAL_LABEL_OFFSET_PX, -GOAL_LABEL_OFFSET_PX),
            egui::Align2::LEFT_BOTTOM,
            if reached {
                "Target (reached)"
            } else {
                "Target"
            },
            egui::FontId::monospace(12.0),
            color,
        );
    }

    fn draw_ship(&self, painter: &egui::Painter, rect: egui::Rect, state: &State) {
        self.draw_ship_sized(painter, rect, state, self.circumradius);
    }

    pub(super) fn draw_ship_sized(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        state: &State,
        circumradius: f32,
    ) {
        let center = self.camera.local(state.x, state.y);
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
                let vertex = center + egui::vec2(x, y) * circumradius;
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

    pub(super) fn draw_arrows(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        replay: &Replay,
        k: usize,
        state: &State,
    ) {
        let Some(center) = self
            .camera
            .project(Vec3::plane(self.camera.local(state.x, state.y)), rect)
        else {
            return;
        };

        // A world vector drawn as the world segment from the ship to the ship plus `d`, so it
        // zooms with the world like any other length in it.
        let arrow = |d: [f64; 2], color: egui::Color32| {
            let tip = Vec3::plane(self.camera.local(state.x + d[0], state.y + d[1]));
            let Some(tip) = self.camera.project(tip, rect) else {
                return;
            };
            let screen = tip - center;
            if screen.length() > ARROW_MIN_PIXELS {
                solid_arrow(painter, center, screen, color);
            }
        };
        let dt = ARROW_SECONDS;
        if self.show_velocity {
            arrow([state.vx * dt, state.vy * dt], VELOCITY_COLOR);
        }
        if self.show_acceleration {
            let thrust = replay.acceleration(k, state);
            let world = self.flight_world();
            let gravity = crate::physics::gravity_acceleration(
                &world.bodies,
                world.gravity_constant,
                [state.x, state.y],
            );
            let a = [thrust[0] + gravity[0], thrust[1] + gravity[1]];
            arrow([a[0] * dt * dt, a[1] * dt * dt], ACCELERATION_COLOR);
        }
    }
}

/// An arrow from `origin` along `vector`, on screen, with a filled triangular head of a fixed
/// size in pixels.
fn solid_arrow(
    painter: &egui::Painter,
    origin: egui::Pos2,
    vector: egui::Vec2,
    color: egui::Color32,
) {
    let length = vector.length();
    let along = vector / length;
    let across = along.rot90();
    let head = ARROWHEAD_LENGTH.min(0.5 * length);
    let width = ARROWHEAD_WIDTH * head / ARROWHEAD_LENGTH;
    let tip = origin + vector;
    let base = tip - along * head;
    // The shaft stops inside the head, so its square end does not poke past the tip's sides.
    painter.line_segment(
        [origin, base + along * (0.5 * head)],
        egui::Stroke::new(ARROW_STROKE_WIDTH, color),
    );
    painter.add(egui::Shape::convex_polygon(
        vec![
            tip,
            base + across * (0.5 * width),
            base - across * (0.5 * width),
        ],
        color,
        egui::Stroke::NONE,
    ));
}

/// The ship's own collision mesh, over the canvas among the display options' collision
/// meshes: the collision hull shared with the physics, turned to the displayed heading
/// `theta` and scaled to the world's hull radius `rho`.
fn draw_collision_hull(
    painter: &egui::Painter,
    camera: &Camera,
    rect: egui::Rect,
    position: egui::Vec2,
    theta: f32,
    rho: f32,
    stroke: egui::Stroke,
) {
    let (sin, cos) = theta.sin_cos();
    let count = nabla_sim::hull::HULL.len();
    let points = (0..=count).map(|i| {
        let [x, y] = nabla_sim::hull::HULL[i % count];
        Vec3::plane(position + egui::vec2(cos * x - sin * y, sin * x + cos * y) * rho)
    });
    for run in camera.project_polyline(points, rect) {
        painter.add(egui::Shape::line(run, stroke));
    }
}

/// The polyline through world points `points` on screen, as the runs of it in front of the
/// eye, each skipping points closer than `PATH_MIN_SEGMENT_PX` but its last.
pub(super) fn screen_path(
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

/// The path flown in the 3D view, as depth-tested strokes of the marks' pass
/// (`crate::overlays`): the same shapes `draw_path` gives egui in the flat view - the
/// flight so far in `PATH_COLOR`, what has been flown of it up to `now` in `TRAVELED_COLOR`
/// over it, and where the run started, even once that path has moved on.
pub(super) fn path_strokes(
    overlays: &mut crate::overlays::Overlays<'_>,
    replay: &Replay,
    k: usize,
    now: &State,
) {
    let camera = *overlays.camera();
    let point = |s: &State| Vec3::plane(camera.local(s.x, s.y));
    overlays.stroke(replay.states().iter().map(point), false, 1.0, PATH_COLOR);
    overlays.stroke(
        replay.states_up_to(k).chain([now]).map(point),
        false,
        2.0,
        TRAVELED_COLOR,
    );
    let start = replay.case.start;
    overlays.ring(
        camera.local(start.x, start.y),
        START_RADIUS_PX,
        1.5,
        TRAVELED_COLOR,
    );
}
