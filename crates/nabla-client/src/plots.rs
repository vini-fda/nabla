//! What the state plots show: the last [`WINDOW`] seconds of a flight, in buffers of a fixed size.
//!
//! A flight's [`Replay`] holds the same window, as states and controls. A plot line borrows its
//! points as one slice of `[t, value]` pairs, so the plots have buffers of their own, allocated
//! with the flight, and recording a sample in them never allocates. The step size `h` is fixed
//! for a flight (changing it starts a new one), and so is their size.

use egui_plot::PlotPoint;
use nabla_core::{Control, State};

use crate::replay::Replay;

/// How far back the plots show, in seconds.
pub const WINDOW: f64 = 60.0;

/// The fewest steps of length `h` that cover [`WINDOW`], and at least one.
pub fn window_steps(h: f64) -> usize {
    ((WINDOW / h).ceil() as usize).max(1)
}

/// The newest points of one plotted quantity, oldest first, in a fixed allocation.
///
/// A ring buffer written twice, at `i` and at `i + capacity`: the newest points are then always
/// one contiguous slice, which is what a plot line borrows, and neither a write nor a read has
/// to move any of them.
pub struct Series {
    /// Twice the capacity.
    points: Box<[PlotPoint]>,
    /// Where the next point goes, below the capacity.
    next: usize,
    len: usize,
}

impl Series {
    fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "a series holds at least a point");
        Self {
            points: vec![PlotPoint::new(0.0, 0.0); 2 * capacity].into_boxed_slice(),
            next: 0,
            len: 0,
        }
    }

    fn capacity(&self) -> usize {
        self.points.len() / 2
    }

    /// Add `point`, discarding the oldest if full.
    fn push(&mut self, point: PlotPoint) {
        let capacity = self.capacity();
        self.points[self.next] = point;
        self.points[self.next + capacity] = point;
        self.next = (self.next + 1) % capacity;
        self.len = (self.len + 1).min(capacity);
    }

    fn clear(&mut self) {
        self.next = 0;
        self.len = 0;
    }

    /// The points held, oldest first.
    pub fn points(&self) -> &[PlotPoint] {
        let end = self.next + self.capacity();
        &self.points[end - self.len..end]
    }
}

/// The plotted quantities of a flight, over its last `steps` steps.
pub struct Plots {
    steps: usize,
    pub x: Series,
    pub y: Series,
    pub vx: Series,
    pub vy: Series,
    pub theta: Series,
    pub omega: Series,
    pub thrust: Series,
    pub torque: Series,
}

impl Plots {
    /// Empty buffers for the last `steps` steps (at least one), all allocated here.
    pub fn new(steps: usize) -> Self {
        let steps = steps.max(1);
        // The two ends of `steps` steps.
        let state = || Series::new(steps + 1);
        // A control is held over its step, so it is drawn as a point at each end of it.
        let control = || Series::new(2 * steps);
        Self {
            steps,
            x: state(),
            y: state(),
            vx: state(),
            vy: state(),
            theta: state(),
            omega: state(),
            thrust: control(),
            torque: control(),
        }
    }

    /// How many steps these buffers hold.
    pub fn steps(&self) -> usize {
        self.steps
    }

    /// Record the state `s` at time `t`.
    pub fn record_state(&mut self, t: f64, s: &State) {
        self.x.push(PlotPoint::new(t, s.x));
        self.y.push(PlotPoint::new(t, s.y));
        self.vx.push(PlotPoint::new(t, s.vx));
        self.vy.push(PlotPoint::new(t, s.vy));
        self.theta.push(PlotPoint::new(t, s.theta));
        self.omega.push(PlotPoint::new(t, s.omega));
    }

    /// Record a step: the control `u` held from time `from` to time `to`, and the state `s`
    /// it ended in.
    pub fn record_step(&mut self, from: f64, to: f64, u: &Control, s: &State) {
        self.thrust.push(PlotPoint::new(from, u.thrust));
        self.thrust.push(PlotPoint::new(to, u.thrust));
        self.torque.push(PlotPoint::new(from, u.torque));
        self.torque.push(PlotPoint::new(to, u.torque));
        self.record_state(to, s);
    }

    /// Hold the end of `replay` instead of what these hold, for a run that is not recorded as it
    /// goes, in the same buffers.
    pub fn rerecord(&mut self, replay: &Replay) {
        for series in [
            &mut self.x,
            &mut self.y,
            &mut self.vx,
            &mut self.vy,
            &mut self.theta,
            &mut self.omega,
            &mut self.thrust,
            &mut self.torque,
        ] {
            series.clear();
        }
        let last = replay.last_frame();
        let first = last.saturating_sub(self.steps).max(replay.first_frame());
        self.record_state(replay.time(first), replay.state(first));
        for k in first..last {
            if let Some(u) = replay.control(k) {
                self.record_step(replay.time(k), replay.time(k + 1), u, replay.state(k + 1));
            }
        }
    }

    #[cfg(test)]
    fn all(&self) -> [&Series; 8] {
        [
            &self.x,
            &self.y,
            &self.vx,
            &self.vy,
            &self.theta,
            &self.omega,
            &self.thrust,
            &self.torque,
        ]
    }

    /// Where each buffer is and how long it is: these never change once allocated.
    #[cfg(test)]
    pub fn allocations(&self) -> [(*const PlotPoint, usize); 8] {
        self.all().map(|s| (s.points.as_ptr(), s.points.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nabla_core::problem::{Case, Outcome, Trajectory};
    use nabla_core::suite;

    fn xs(series: &Series) -> Vec<f64> {
        series.points().iter().map(|p| p.x).collect()
    }

    fn filled(capacity: usize, pushes: usize) -> Series {
        let mut series = Series::new(capacity);
        for i in 0..pushes {
            series.push(PlotPoint::new(i as f64, 2.0 * i as f64));
        }
        series
    }

    fn state(t: f64) -> State {
        State {
            x: t,
            y: 2.0 * t,
            vx: 3.0 * t,
            vy: 4.0 * t,
            theta: 5.0 * t,
            omega: 6.0 * t,
        }
    }

    fn control(t: f64) -> Control {
        Control {
            thrust: t,
            torque: -t,
        }
    }

    /// A flight of `steps` steps of `h`, recorded as it goes.
    fn flown(capacity: usize, h: f64, steps: usize) -> Plots {
        let mut plots = Plots::new(capacity);
        plots.record_state(0.0, &state(0.0));
        for k in 0..steps {
            let (from, to) = (k as f64 * h, (k + 1) as f64 * h);
            plots.record_step(from, to, &control(from), &state(to));
        }
        plots
    }

    #[test]
    fn a_series_keeps_its_points_in_order_until_full() {
        assert!(filled(4, 0).points().is_empty());
        assert_eq!(xs(&filled(4, 3)), [0.0, 1.0, 2.0]);
        assert_eq!(xs(&filled(4, 4)), [0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn a_full_series_discards_its_oldest_point_at_each_new_one() {
        // Past the end of the ring, around it, and to a whole number of turns.
        assert_eq!(xs(&filled(4, 5)), [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(xs(&filled(4, 7)), [3.0, 4.0, 5.0, 6.0]);
        assert_eq!(xs(&filled(4, 8)), [4.0, 5.0, 6.0, 7.0]);
        assert_eq!(xs(&filled(4, 4003)), [3999.0, 4000.0, 4001.0, 4002.0]);
        assert_eq!(xs(&filled(1, 3)), [2.0]);
        for p in filled(4, 7).points() {
            assert!((p.y - 2.0 * p.x).abs() < 1e-12, "each value with its time");
        }
    }

    #[test]
    fn a_cleared_series_starts_over() {
        let mut series = filled(4, 6);
        series.clear();
        assert!(series.points().is_empty());
        series.push(PlotPoint::new(9.0, 0.0));
        assert_eq!(xs(&series), [9.0]);
    }

    #[test]
    fn the_window_is_covered_by_at_most_a_step_more_than_it() {
        for h in [0.01, 0.02, 1.0 / 60.0, 0.03, 0.05, 0.07, 0.1] {
            let span = window_steps(h) as f64 * h;
            assert!(
                (WINDOW - 1e-9..WINDOW + h + 1e-9).contains(&span),
                "h = {h} gives {span} s"
            );
        }
        assert_eq!(window_steps(0.02), 3000);
        assert_eq!(window_steps(100.0), 1, "a step longer than the window");
    }

    #[test]
    fn every_plot_spans_the_same_steps_before_and_after_it_fills() {
        let plots = flown(10, 0.5, 4);
        assert_eq!(plots.x.points().len(), 5);
        assert_eq!(plots.thrust.points().len(), 8);

        let plots = flown(10, 0.5, 25);
        for series in plots.all() {
            let points = series.points();
            let (first, last) = (points[0].x, points[points.len() - 1].x);
            assert!((first - 7.5).abs() < 1e-12, "starts at {first}");
            assert!((last - 12.5).abs() < 1e-12, "ends at {last}");
        }
        assert_eq!(plots.omega.points().len(), 11);
        assert_eq!(plots.torque.points().len(), 20);
    }

    #[test]
    fn a_control_stays_a_whole_step_as_the_oldest_are_discarded() {
        let plots = flown(3, 1.0, 10);
        let thrust: Vec<[f64; 2]> = plots.thrust.points().iter().map(|p| [p.x, p.y]).collect();
        assert_eq!(
            thrust,
            [
                [7.0, 7.0],
                [8.0, 7.0],
                [8.0, 8.0],
                [9.0, 8.0],
                [9.0, 9.0],
                [10.0, 9.0],
            ]
        );
    }

    #[test]
    fn recording_never_grows_or_moves_the_buffers() {
        let mut plots = flown(100, 0.1, 1);
        let allocations = plots.allocations();
        for k in 1..1000 {
            let (from, to) = (f64::from(k) * 0.1, f64::from(k + 1) * 0.1);
            plots.record_step(from, to, &control(from), &state(to));
            assert_eq!(plots.allocations(), allocations);
            assert!(plots.x.points().len() <= 101);
            assert!(plots.thrust.points().len() <= 200);
        }
        // Twice the points held at most: 101 for a state and 200 for a control.
        let sizes = plots.allocations().map(|(_, len)| len);
        assert_eq!(sizes, [202, 202, 202, 202, 202, 202, 400, 400]);
    }

    /// A run of `steps` steps of `h` as a replay.
    fn replay(h: f64, steps: usize) -> Replay {
        let mut case: Case = suite::find("agile").expect("case exists");
        case.task.params.h = h;
        let time = |k: usize| k as f64 * h;
        Replay::new(
            case,
            Trajectory {
                states: (0..=steps).map(|k| state(time(k))).collect(),
                controls: (0..steps).map(|k| control(time(k))).collect(),
                outcome: Outcome::Timeout,
            },
        )
    }

    #[test]
    fn rerecording_a_run_matches_recording_it_as_it_goes_in_the_same_buffers() {
        let mut plots = Plots::new(10);
        let allocations = plots.allocations();
        for steps in [0, 4, 10, 25, 3] {
            plots.rerecord(&replay(0.5, steps));
            let expected = flown(10, 0.5, steps);
            for (got, want) in plots.all().into_iter().zip(expected.all()) {
                assert_eq!(got.points(), want.points(), "after {steps} steps");
            }
            assert_eq!(plots.allocations(), allocations);
        }
    }
}
