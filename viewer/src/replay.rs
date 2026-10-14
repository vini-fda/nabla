//! A recorded run of a solution on one test case, and continuous-time playback.
//!
//! The run is computed once, by the exact sampled-data model of `ship2d_optimization`: `s_k` is
//! the state at `t_k = k h`, and `u_k` is the control held during `[t_k, t_{k+1})`. Because the
//! control is constant between samples, the state at any `t = t_k + tau`, `0 <= tau < h`, is exactly
//! one `step` of length `tau` from `s_k` under `u_k`, so playback can show the true continuous
//! motion at any display sampling time `dt`, not only at the controller samples.
//!
//! Playback positions are integer counts of `dt = h / subdivisions`, so the controller samples
//! fall exactly on the display grid.

use ship2d_optimization::problem::{Case, Outcome, Trajectory};
use ship2d_optimization::{Control, Params, State, step};

/// Time series of one plotted quantity: `[t, value]` pairs.
pub type Series = Vec<[f64; 2]>;

/// One solution run on one case.
pub struct Replay {
    /// Name of the solution source.
    pub solution: String,
    pub case: Case,
    pub trajectory: Trajectory,
}

impl Replay {
    pub fn new(solution: &str, case: Case, trajectory: Trajectory) -> Self {
        Self {
            solution: solution.to_owned(),
            case,
            trajectory,
        }
    }

    /// Controller sampling period `h`.
    pub fn h(&self) -> f64 {
        self.case.task.params.h
    }

    /// Index of the last sample (`N`, the number of applied controls).
    pub fn last_frame(&self) -> usize {
        self.trajectory.states.len() - 1
    }

    /// Time `t_k` of sample `k`.
    pub fn time(&self, frame: usize) -> f64 {
        frame as f64 * self.h()
    }

    /// Duration `N h` of the run.
    pub fn duration(&self) -> f64 {
        self.time(self.last_frame())
    }

    /// Sample state `s_k` (clamped to the last sample).
    pub fn state(&self, frame: usize) -> &State {
        &self.trajectory.states[frame.min(self.last_frame())]
    }

    /// Exact state at `t_k + offset`, for `0 <= offset < h`: one partial step from `s_k` under
    /// `u_k`. At or after the last sample this is `s_N`.
    pub fn state_at(&self, frame: usize, offset: f64) -> State {
        match self.trajectory.controls.get(frame) {
            Some(u) if offset > 0.0 => {
                let params = Params {
                    h: offset,
                    ..self.case.task.params
                };
                step(&params, &self.trajectory.states[frame], u)
            }
            _ => *self.state(frame),
        }
    }

    /// Exact state at time `t`, clamped to the run.
    pub fn state_at_time(&self, t: f64) -> State {
        let t = t.clamp(0.0, self.duration());
        let frame = self.frame_at(t);
        self.state_at(frame, t - self.time(frame))
    }

    /// Thrust at time `t` as a fraction of `F_max`; 0 outside the run.
    pub fn thrust_fraction(&self, t: f64) -> f64 {
        if t < 0.0 {
            return 0.0;
        }
        self.control(self.frame_at(t))
            .map_or(0.0, |u| u.thrust / self.case.task.params.f_max)
    }

    /// Torque at time `t` as a fraction of `tau_max` (-1 to 1); 0 outside the run.
    pub fn torque_fraction(&self, t: f64) -> f64 {
        if t < 0.0 {
            return 0.0;
        }
        self.control(self.frame_at(t))
            .map_or(0.0, |u| u.torque / self.case.task.params.tau_max)
    }

    /// The sample `k` whose period `[t_k, t_{k+1})` holds time `t >= 0`.
    fn frame_at(&self, t: f64) -> usize {
        (t / self.h()).floor() as usize
    }

    /// Control `u_k`, held during `[t_k, t_{k+1})`; `None` from the last sample on.
    pub fn control(&self, frame: usize) -> Option<&Control> {
        self.trajectory.controls.get(frame)
    }

    /// Linear acceleration `F/m (cos theta, sin theta)` of `state` under the control held at sample `k`.
    pub fn acceleration(&self, frame: usize, state: &State) -> [f64; 2] {
        let thrust = self.control(frame).map_or(0.0, |u| u.thrust);
        let a = thrust / self.case.task.params.mass;
        [a * state.theta.cos(), a * state.theta.sin()]
    }

    pub fn outcome(&self) -> &Outcome {
        &self.trajectory.outcome
    }

    /// Bounding box `([x_min, y_min], [x_max, y_max])` of the path and the goal.
    pub fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        let goal = self.case.task.goal;
        let init = ([goal.x, goal.y], [goal.x, goal.y]);
        self.trajectory.states.iter().fold(init, |(lo, hi), s| {
            (
                [lo[0].min(s.x), lo[1].min(s.y)],
                [hi[0].max(s.x), hi[1].max(s.y)],
            )
        })
    }

    /// `[t_k, f(s_k)]` for every sample.
    pub fn state_series(&self, f: impl Fn(&State) -> f64) -> Series {
        self.trajectory
            .states
            .iter()
            .enumerate()
            .map(|(k, s)| [self.time(k), f(s)])
            .collect()
    }

    /// Piecewise-constant control signal: `u_k` drawn over `[t_k, t_{k+1}]`.
    pub fn control_series(&self, f: impl Fn(&Control) -> f64) -> Series {
        self.trajectory
            .controls
            .iter()
            .enumerate()
            .flat_map(|(k, u)| [[self.time(k), f(u)], [self.time(k + 1), f(u)]])
            .collect()
    }
}

/// Largest number of display samples per controller sample.
pub const MAX_SUBDIVISIONS: u32 = 1000;

/// Playback position, display sampling time and speed.
///
/// The position is an integer number of ticks of `dt = h / subdivisions`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Playback {
    tick: u64,
    pub playing: bool,
    /// Simulated seconds per real second.
    pub speed: f64,
    /// Display samples per controller sample: `dt = h / subdivisions`.
    subdivisions: u32,
    /// Fraction of a tick accumulated but not yet shown.
    carry: f64,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            tick: 0,
            playing: false,
            speed: 1.0,
            subdivisions: 4,
            carry: 0.0,
        }
    }
}

impl Playback {
    pub fn subdivisions(&self) -> u32 {
        self.subdivisions.clamp(1, MAX_SUBDIVISIONS)
    }

    fn n(&self) -> u64 {
        u64::from(self.subdivisions())
    }

    /// Display sampling time `dt`.
    pub fn dt(&self, h: f64) -> f64 {
        h / f64::from(self.subdivisions())
    }

    /// Change `dt = h / subdivisions`, keeping the playhead at the nearest tick of the new grid.
    pub fn set_subdivisions(&mut self, subdivisions: u32) {
        let old = self.n();
        self.subdivisions = subdivisions.clamp(1, MAX_SUBDIVISIONS);
        let new = self.n();
        self.tick = (self.tick * new + old / 2) / old;
        self.carry = 0.0;
    }

    fn last_tick(&self, last_frame: usize) -> u64 {
        last_frame as u64 * self.n()
    }

    /// The playhead as `(k, tau)` with `t = k h + tau` and `0 <= tau < h`.
    pub fn moment(&self, h: f64) -> (usize, f64) {
        let n = self.n();
        (
            (self.tick / n) as usize,
            (self.tick % n) as f64 * self.dt(h),
        )
    }

    /// Playhead time `t`.
    pub fn time(&self, h: f64) -> f64 {
        let (k, offset) = self.moment(h);
        k as f64 * h + offset
    }

    /// Whether the playhead sits exactly on a controller sample.
    pub fn on_sample(&self) -> bool {
        self.tick.is_multiple_of(self.n())
    }

    fn seek_tick(&mut self, tick: u64, last_frame: usize) {
        self.tick = tick.min(self.last_tick(last_frame));
        self.carry = 0.0;
    }

    /// Jump to the tick nearest to time `t`.
    pub fn seek_time(&mut self, t: f64, h: f64, last_frame: usize) {
        let ticks = (t / self.dt(h)).round().max(0.0);
        let last = self.last_tick(last_frame);
        let tick = if ticks >= last as f64 {
            last
        } else {
            ticks as u64
        };
        self.seek_tick(tick, last_frame);
    }

    pub fn seek_start(&mut self) {
        self.seek_tick(0, 0);
    }

    pub fn seek_end(&mut self, last_frame: usize) {
        self.seek_tick(self.last_tick(last_frame), last_frame);
    }

    /// Move by `delta` ticks of `dt` and pause.
    pub fn step_ticks(&mut self, delta: i64, last_frame: usize) {
        self.playing = false;
        self.seek_tick(self.tick.saturating_add_signed(delta), last_frame);
    }

    /// Move to the previous controller sample (strictly before the playhead) and pause.
    pub fn previous_sample(&mut self, last_frame: usize) {
        self.playing = false;
        let n = self.n();
        let tick = if self.tick.is_multiple_of(n) {
            self.tick.saturating_sub(n)
        } else {
            self.tick - self.tick % n
        };
        self.seek_tick(tick, last_frame);
    }

    /// Move to the next controller sample (strictly after the playhead) and pause.
    pub fn next_sample(&mut self, last_frame: usize) {
        self.playing = false;
        let n = self.n();
        self.seek_tick((self.tick / n + 1) * n, last_frame);
    }

    /// Start playing; from the beginning if already at the end.
    pub fn play(&mut self, last_frame: usize) {
        if self.tick >= self.last_tick(last_frame) {
            self.seek_start();
        }
        self.playing = true;
    }

    pub fn toggle(&mut self, last_frame: usize) {
        if self.playing {
            self.playing = false;
        } else {
            self.play(last_frame);
        }
    }

    /// Advance by `dt` real seconds of playback. Stops at the end.
    pub fn advance(&mut self, dt: f64, h: f64, last_frame: usize) {
        if !self.playing {
            return;
        }
        let ticks = self.carry + dt * self.speed / self.dt(h);
        let whole = ticks.floor();
        self.carry = ticks - whole;
        let remaining = self.last_tick(last_frame).saturating_sub(self.tick);
        if whole >= remaining as f64 {
            self.seek_end(last_frame);
            self.playing = false;
        } else {
            self.tick += whole as u64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ship2d_optimization::registry::valid;
    use ship2d_optimization::suite;

    const H: f64 = 0.1;

    fn replay() -> Replay {
        let (solution, _) = valid().into_iter().next().expect("a stored solution");
        let case = suite::standard()
            .into_iter()
            .find(|c| c.name == "rest_facing_goal")
            .expect("case exists");
        let trajectory = (solution.simulate)(&case);
        Replay::new(solution.name, case, trajectory)
    }

    fn close(a: &State, b: &State, tol: f64) -> bool {
        [
            (a.x, b.x),
            (a.y, b.y),
            (a.vx, b.vx),
            (a.vy, b.vy),
            (a.theta, b.theta),
            (a.omega, b.omega),
        ]
        .iter()
        .all(|(u, v)| (u - v).abs() <= tol)
    }

    #[test]
    fn replay_reaches_the_goal_at_the_last_sample() {
        let r = replay();
        assert_eq!(r.outcome().steps(), Some(r.last_frame()), "solved case");
        assert!(
            r.case.task.reached(r.state(r.last_frame())),
            "ends in target"
        );
        assert!(
            r.control(r.last_frame()).is_none(),
            "no control after the end"
        );
    }

    #[test]
    fn state_between_samples_is_exact_and_continuous() {
        let r = replay();
        let h = r.h();
        for k in [0, 7, r.last_frame() / 2, r.last_frame() - 1] {
            assert_eq!(r.state_at(k, 0.0), *r.state(k), "offset 0 is the sample");
            let almost = r.state_at(k, h * (1.0 - 1e-12));
            assert!(
                close(&almost, r.state(k + 1), 1e-9),
                "continuous at t_(k+1)"
            );
            // Two half steps under the held control reach the next sample.
            let half = Params {
                h: h / 2.0,
                ..r.case.task.params
            };
            let u = r.control(k).expect("control");
            let twice = step(&half, &r.state_at(k, h / 2.0), u);
            assert!(
                close(&twice, r.state(k + 1), 1e-12),
                "consistent with s_(k+1)"
            );
        }
        assert_eq!(
            r.state_at(r.last_frame(), h / 2.0),
            *r.state(r.last_frame()),
            "held at end"
        );
    }

    #[test]
    fn series_cover_every_sample() {
        let r = replay();
        let xs = r.state_series(|s| s.x);
        assert_eq!(xs.len(), r.last_frame() + 1, "one point per state");
        let thrust = r.control_series(|u| u.thrust);
        assert_eq!(thrust.len(), 2 * r.last_frame(), "two points per control");
        let end = thrust.last().expect("non-empty")[0];
        assert!((end - r.duration()).abs() < 1e-9, "ends at t_N");
    }

    #[test]
    fn bounds_contain_path_and_goal() {
        let r = replay();
        let (lo, hi) = r.bounds();
        for s in &r.trajectory.states {
            assert!(
                lo[0] <= s.x && s.x <= hi[0] && lo[1] <= s.y && s.y <= hi[1],
                "path"
            );
        }
        let g = r.case.task.goal;
        assert!(
            lo[0] <= g.x && g.x <= hi[0] && lo[1] <= g.y && g.y <= hi[1],
            "goal"
        );
    }

    #[test]
    fn moment_splits_time_on_the_sample_grid() {
        let mut p = Playback::default();
        p.set_subdivisions(4);
        p.seek_time(0.275, H, 100);
        assert_eq!(p.moment(H).0, 2, "sample index");
        assert!(
            (p.moment(H).1 - 0.075).abs() < 1e-12,
            "offset into the step"
        );
        assert!(!p.on_sample(), "between samples");
        p.seek_time(0.3, H, 100);
        assert_eq!(p.moment(H), (3, 0.0), "exactly on a sample");
        assert!(p.on_sample(), "on a sample");
    }

    #[test]
    fn seek_time_rounds_and_clamps() {
        let mut p = Playback::default();
        p.set_subdivisions(1);
        p.seek_time(-1.0, H, 100);
        assert_eq!(p.moment(H), (0, 0.0), "clamped below");
        p.seek_time(0.26, H, 100);
        assert_eq!(p.moment(H), (3, 0.0), "rounded to the dt grid");
        p.seek_time(1e9, H, 100);
        assert_eq!(p.moment(H), (100, 0.0), "clamped above");
    }

    #[test]
    fn changing_subdivisions_keeps_the_time() {
        let mut p = Playback::default();
        p.set_subdivisions(10);
        p.seek_time(1.23, H, 100);
        p.set_subdivisions(100);
        assert!((p.time(H) - 1.23).abs() < 1e-12, "refined grid keeps t");
        p.set_subdivisions(1);
        assert!(
            (p.time(H) - 1.2).abs() < 1e-12,
            "coarse grid rounds to nearest"
        );
        p.set_subdivisions(0);
        assert_eq!(p.subdivisions(), 1, "at least one tick per sample");
    }

    #[test]
    fn sample_navigation_jumps_between_controller_samples() {
        let mut p = Playback::default();
        p.set_subdivisions(10);
        p.seek_time(0.43, H, 100);
        p.next_sample(100);
        assert_eq!(p.moment(H), (5, 0.0), "next sample");
        p.previous_sample(100);
        assert_eq!(p.moment(H), (4, 0.0), "previous sample from a sample");
        p.step_ticks(3, 100);
        p.previous_sample(100);
        assert_eq!(
            p.moment(H),
            (4, 0.0),
            "previous sample from between samples"
        );
        p.step_ticks(-1, 100);
        assert!((p.time(H) - 0.39).abs() < 1e-12, "one dt back");
    }

    #[test]
    fn playback_advances_at_speed_and_stops_at_end() {
        let mut p = Playback {
            speed: 2.0,
            ..Playback::default()
        };
        p.set_subdivisions(10); // dt = 0.01
        p.play(10);
        p.advance(0.0125, H, 10); // 2.5 ticks
        assert!((p.time(H) - 0.02).abs() < 1e-12, "whole ticks shown");
        p.advance(0.0025, H, 10); // 0.5 + carried 0.5
        assert!((p.time(H) - 0.03).abs() < 1e-12, "fractions carried over");
        p.advance(100.0, H, 10);
        assert_eq!(p.moment(H), (10, 0.0), "clamped at the end");
        assert!(!p.playing, "stops at the end");
        p.play(10);
        assert_eq!(p.moment(H), (0, 0.0), "replays from the start");
    }

    #[test]
    fn paused_playback_does_not_move() {
        let mut p = Playback::default();
        p.seek_time(0.5, H, 100);
        p.advance(10.0, H, 100);
        assert!((p.time(H) - 0.5).abs() < 1e-12, "paused");
    }
}
