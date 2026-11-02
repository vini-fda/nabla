//! A recorded run of the ship on one test case.
//!
//! Vacuum runs follow the exact sampled-data model of `nabla-core`: `s_k` is the state at
//! `t_k = k h`, and `u_k` is the control held during `[t_k, t_{k+1})`. Because the control is constant between samples, the state at any
//! `t = t_k + tau`, `0 <= tau < h`, is exactly one `step` of length `tau` from `s_k` under `u_k`,
//! so the canvas shows the true continuous motion every frame, not only at the samples.
//! Collision worlds instead interpolate the recorded Rapier substeps.

use nabla_core::problem::{Case, Trajectory};
use nabla_core::{Control, Params, State, step};

/// Time series of one plotted quantity: `[t, value]` pairs.
pub type Series = Vec<[f64; 2]>;

/// One run on one case.
pub struct Replay {
    pub case: Case,
    pub trajectory: Trajectory,
    /// Rapier substeps for worlds with collisions; empty for exact vacuum flights.
    pub collision_steps: Vec<Vec<State>>,
}

impl Replay {
    pub fn new(case: Case, trajectory: Trajectory) -> Self {
        Self {
            case,
            trajectory,
            collision_steps: Vec::new(),
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
        if let Some(samples) = self.collision_steps.get(frame) {
            let t = (offset / self.h()).clamp(0.0, 1.0) * (samples.len() - 1) as f64;
            let i = (t.floor() as usize).min(samples.len() - 2);
            let (a, b) = (samples[i], samples[i + 1]);
            let mix = |a: f64, b: f64| a + (b - a) * (t - i as f64);
            return State {
                x: mix(a.x, b.x),
                y: mix(a.y, b.y),
                vx: mix(a.vx, b.vx),
                vy: mix(a.vy, b.vy),
                theta: mix(a.theta, b.theta),
                omega: mix(a.omega, b.omega),
            };
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use nabla_core::solutions::SOLUTIONS;
    use nabla_core::suite;

    fn replay() -> Replay {
        let solution = &SOLUTIONS[0];
        let case = suite::standard()
            .into_iter()
            .find(|c| c.name == "rest_facing_goal")
            .expect("case exists");
        let trajectory = (solution.simulate)(&case);
        Replay::new(case, trajectory)
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
        assert_eq!(
            r.trajectory.outcome.steps(),
            Some(r.last_frame()),
            "solved case"
        );
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
}
