//! A recorded run of the ship on one test case, or the end of one.
//!
//! Vacuum runs follow the exact sampled-data model of `nabla-core`: `s_k` is the state at
//! `t_k = k h`, and `u_k` is the control held during `[t_k, t_{k+1})`. Because the control is constant between samples, the state at any
//! `t = t_k + tau`, `0 <= tau < h`, is exactly one `step` of length `tau` from `s_k` under `u_k`,
//! so the canvas shows the true continuous motion every frame, not only at the samples.
//! Collision worlds instead interpolate the recorded Rapier substeps.
//!
//! A flight has no end, so it keeps only its newest steps, in buffers allocated when it starts: see
//! [`Replay::recording`]. Frames keep counting from the start of the run, whatever was discarded.

use std::collections::VecDeque;

use nabla_core::problem::{Case, Trajectory};
use nabla_core::{Control, State, flow};

/// One run on one case.
pub struct Replay {
    pub case: Case,
    /// `s_k` for the frames from `first` on, oldest first: one more than `controls`.
    states: VecDeque<State>,
    /// `u_k` for the frames from `first` on, oldest first.
    controls: VecDeque<Control>,
    /// Rapier substeps of each of `controls`, for worlds with collisions; empty for exact vacuum
    /// flights.
    collision_steps: VecDeque<Vec<State>>,
    /// The frame of the oldest sample held.
    first: usize,
    /// The most steps held: recording past this discards the oldest.
    max_steps: usize,
}

impl Replay {
    /// A whole run, already flown.
    pub fn new(case: Case, trajectory: Trajectory) -> Self {
        let max_steps = trajectory.controls.len().max(1);
        Self {
            case,
            states: trajectory.states.into(),
            controls: trajectory.controls.into(),
            collision_steps: VecDeque::new(),
            first: 0,
            max_steps,
        }
    }

    /// A run about to be flown from the case's start, which will hold its newest `steps` steps
    /// (at least one), with all the room for them allocated here.
    pub fn recording(case: Case, steps: usize) -> Self {
        let max_steps = steps.max(1);
        let mut states = VecDeque::with_capacity(max_steps + 1);
        states.push_back(case.start);
        Self {
            case,
            states,
            controls: VecDeque::with_capacity(max_steps),
            collision_steps: VecDeque::with_capacity(max_steps),
            first: 0,
            max_steps,
        }
    }

    /// Add a step: the control `u` held over it, the state `next` it ended in, and its Rapier
    /// substeps in a world with collisions. Discards the oldest step once full.
    pub fn record(&mut self, u: Control, next: State, samples: Option<Vec<State>>) {
        if self.controls.len() == self.max_steps {
            self.states.pop_front();
            self.controls.pop_front();
            self.collision_steps.pop_front();
            self.first += 1;
        }
        self.states.push_back(next);
        self.controls.push_back(u);
        if let Some(samples) = samples {
            self.collision_steps.push_back(samples);
        }
    }

    /// Controller sampling period `h`.
    pub fn h(&self) -> f64 {
        self.case.task.params.h
    }

    /// The frame of the oldest sample held.
    pub fn first_frame(&self) -> usize {
        self.first
    }

    /// Index of the last sample (`N`, the number of applied controls).
    pub fn last_frame(&self) -> usize {
        self.first + self.states.len() - 1
    }

    /// The samples held, oldest first: the first is at [`Self::first_frame`].
    pub fn states(&self) -> &VecDeque<State> {
        &self.states
    }

    /// The samples held up to frame `k`, oldest first.
    pub fn states_up_to(&self, k: usize) -> impl Iterator<Item = &State> {
        self.states.range(..=self.index(k))
    }

    /// The controls held, oldest first: the first is at [`Self::first_frame`].
    pub fn controls(&self) -> &VecDeque<Control> {
        &self.controls
    }

    /// The Rapier substeps held, oldest first.
    pub fn collision_steps(&self) -> &VecDeque<Vec<State>> {
        &self.collision_steps
    }

    /// Where in `states` frame `frame` is, or the nearest sample held.
    fn index(&self, frame: usize) -> usize {
        frame.clamp(self.first, self.last_frame()) - self.first
    }

    /// Time `t_k` of sample `k`.
    pub fn time(&self, frame: usize) -> f64 {
        frame as f64 * self.h()
    }

    /// Duration `N h` of the run.
    pub fn duration(&self) -> f64 {
        self.time(self.last_frame())
    }

    /// Sample state `s_k` (clamped to the samples held).
    pub fn state(&self, frame: usize) -> &State {
        &self.states[self.index(frame)]
    }

    /// Exact state at `t_k + offset`, for `0 <= offset < h`: one partial step from `s_k` under
    /// `u_k`. At or after the last sample this is `s_N`, and before the samples held, the oldest.
    pub fn state_at(&self, frame: usize, offset: f64) -> State {
        if frame < self.first {
            return *self.state(frame);
        }
        let held = frame - self.first;
        if let Some(samples) = self.collision_steps.get(held) {
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
        match self.controls.get(held) {
            Some(u) if offset > 0.0 => flow(&self.case.task.params, &self.states[held], u, offset),
            _ => *self.state(frame),
        }
    }

    /// Exact state at time `t`, clamped to the samples held.
    pub fn state_at_time(&self, t: f64) -> State {
        let t = t.clamp(self.time(self.first), self.duration());
        let frame = self.frame_at(t);
        self.state_at(frame, t - self.time(frame))
    }

    /// Thrust at time `t` as a fraction of `F_max`; 0 outside the samples held.
    pub fn thrust_fraction(&self, t: f64) -> f64 {
        if t < 0.0 {
            return 0.0;
        }
        self.control(self.frame_at(t))
            .map_or(0.0, |u| u.thrust / self.case.task.params.f_max)
    }

    /// Torque at time `t` as a fraction of `tau_max` (-1 to 1); 0 outside the samples held.
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

    /// Control `u_k`, held during `[t_k, t_{k+1})`; `None` from the last sample on, and before
    /// the samples held.
    pub fn control(&self, frame: usize) -> Option<&Control> {
        self.controls.get(frame.checked_sub(self.first)?)
    }

    /// Linear acceleration `F/m (cos theta, sin theta)` of `state` under the control held at sample `k`.
    pub fn acceleration(&self, frame: usize, state: &State) -> [f64; 2] {
        let thrust = self.control(frame).map_or(0.0, |u| u.thrust);
        let a = thrust / self.case.task.params.mass;
        [a * state.theta.cos(), a * state.theta.sin()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nabla_core::solutions::SOLUTIONS;
    use nabla_core::suite;

    fn replay() -> Replay {
        scored().0
    }

    /// The first solution's run, and the steps it was scored at.
    fn scored() -> (Replay, Option<usize>) {
        let solution = &SOLUTIONS[0];
        let case = suite::standard()
            .into_iter()
            .find(|c| c.name == "rest_facing_goal")
            .expect("case exists");
        let trajectory = (solution.simulate)(&case);
        let steps = trajectory.outcome.steps();
        (Replay::new(case, trajectory), steps)
    }

    /// A state that says which frame it is, and a control too.
    fn sample(k: usize) -> (State, Control) {
        let k = k as f64;
        (
            State {
                x: k,
                y: 2.0 * k,
                ..State::default()
            },
            Control {
                thrust: 1.0 + k.rem_euclid(2.0),
                torque: 0.0,
            },
        )
    }

    /// A run that holds `max` steps, after `steps` steps of 0.1 s.
    fn recorded(max: usize, steps: usize) -> Replay {
        let mut case = suite::find("agile").expect("case exists");
        case.task.params.h = 0.1;
        case.start = sample(0).0;
        let mut r = Replay::recording(case, max);
        for k in 0..steps {
            r.record(sample(k).1, sample(k + 1).0, None);
        }
        r
    }

    #[test]
    fn a_recording_counts_frames_from_the_start_and_holds_only_its_newest_steps() {
        let r = recorded(10, 4);
        assert_eq!((r.first_frame(), r.last_frame()), (0, 4));

        let r = recorded(10, 25);
        assert_eq!((r.first_frame(), r.last_frame()), (15, 25));
        assert_eq!((r.states().len(), r.controls().len()), (11, 10));
        for k in 15..=25 {
            assert_eq!(*r.state(k), sample(k).0, "frame {k}");
        }
        for k in 15..25 {
            assert_eq!(r.control(k), Some(&sample(k).1), "frame {k}");
        }
        assert!((r.duration() - 2.5).abs() < 1e-9);
        assert_eq!(*r.state(3), sample(15).0, "clamped to the oldest");
        assert_eq!(*r.state(99), sample(25).0, "clamped to the newest");
        assert_eq!(r.control(14), None, "discarded");
        assert_eq!(r.control(25), None, "none yet");
        let up_to: Vec<f64> = r.states_up_to(17).map(|s| s.x).collect();
        assert_eq!(up_to, [15.0, 16.0, 17.0]);
    }

    #[test]
    fn a_recording_never_grows_past_what_it_allocated() {
        let mut r = recorded(10, 0);
        let capacities = (
            r.states().capacity(),
            r.controls().capacity(),
            r.collision_steps().capacity(),
        );
        assert!(capacities.0 >= 11 && capacities.1 >= 10 && capacities.2 >= 10);
        for k in 0..1000 {
            r.record(sample(k).1, sample(k + 1).0, Some(vec![sample(k).0; 3]));
            assert!(r.collision_steps().len() <= 10);
            assert_eq!(
                capacities,
                (
                    r.states().capacity(),
                    r.controls().capacity(),
                    r.collision_steps().capacity(),
                ),
                "at step {k}"
            );
        }
    }

    #[test]
    fn times_before_what_a_recording_holds_read_as_silence_at_its_oldest_state() {
        let r = recorded(10, 25);
        let f_max = r.case.task.params.f_max;
        // Frames 15 to 24 hold controls, from 1.5 s on.
        assert_eq!(r.state_at_time(0.0), sample(15).0);
        assert_eq!(r.state_at(3, 0.05), sample(15).0);
        assert_eq!(r.thrust_fraction(1.0), 0.0);
        let held = r.thrust_fraction(1.55) * f_max;
        assert!((held - sample(15).1.thrust).abs() < 1e-9, "{held}");
        assert_eq!(r.thrust_fraction(2.55), 0.0, "after the last");
    }

    #[test]
    fn substeps_stay_with_their_frames_as_the_oldest_are_discarded() {
        let mut r = recorded(4, 0);
        for k in 0..20 {
            let (from, to) = (sample(k).0, sample(k + 1).0);
            r.record(sample(k).1, to, Some(vec![from, to]));
        }
        assert_eq!(r.collision_steps().len(), 4);
        for k in 16..20 {
            let middle = r.state_at(k, 0.05);
            assert!((middle.x - (k as f64 + 0.5)).abs() < 1e-9, "frame {k}");
        }
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
        let (r, steps) = scored();
        assert_eq!(steps, Some(r.last_frame()), "solved case");
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
            let params = r.case.task.params;
            let u = r.control(k).expect("control");
            let twice = flow(&params, &r.state_at(k, h / 2.0), u, h / 2.0);
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
}
