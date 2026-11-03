//! The margin controller: how a client keeps its commands arriving just before they are due.
//!
//! A command stamped `k` is on time if it reaches the server before tick `k` is due, and its
//! *margin* `mu` is by how much. Too small, and commands miss their tick. Too large, and the
//! pilot's input waits for nothing. The client has no clock of the server's and needs none: the
//! server reports the margin it saw in each snapshot, and the client steers it by stretching its
//! own tick a little (`docs/multiplayer.typ`, "Keeping the margin").
//!
//! With `e_n = mu_n - mu*` and a client tick that lasts `h + gamma e_(n - n_f)`, where `n_f` is
//! the feedback delay in ticks, `e_(n+1) = e_n - gamma e_(n - n_f) + w_n` for a disturbance `w`.
//! The gain `gamma = 1 / (3 (n_f + 1))` is stable, does not oscillate, and stays stable if the
//! delay is under-estimated by a good deal.

/// The most a tick is stretched or shrunk, as a fraction of `h`: too little to see.
pub const MAX_STRETCH: f64 = 0.05;

/// When the margin is this many ticks short of its target, stretching the tick is too slow a
/// remedy, and the client jumps ahead instead.
pub const JUMP_TICKS: f64 = 6.0;

/// How many of the newest reports the estimate of the feedback delay is the least of: about a
/// quarter of a second, as long as a path's change takes to be followed.
///
/// A report that was queued while the client did not run (a hidden tab, a window held by the
/// mouse) arrives long after it was sent, and looks like a path with a delay of seconds. A
/// minimum ignores such reports at once, and follows a path that really gets longer within the
/// window.
const DELAY_WINDOW: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarginController {
    /// `h`, in seconds.
    h: f64,
    /// `mu*`, in seconds.
    target: f64,
    /// `e`: the margin of the newest report, less the target, in seconds.
    error: f64,
    /// `n_f`: the delay of the feedback in ticks, about `RTT / h`: the least of the last
    /// [`DELAY_WINDOW`] measurements.
    delay_ticks: f64,
    recent: [f64; DELAY_WINDOW],
    filled: usize,
    next: usize,
}

impl MarginController {
    /// A controller for ticks of `h` seconds that steers for margin `target` seconds, given a
    /// first estimate of the round trip in ticks.
    pub fn new(h: f64, target: f64, round_trip_ticks: f64) -> Self {
        Self {
            h,
            target,
            error: 0.0,
            delay_ticks: round_trip_ticks.max(1.0),
            recent: [0.0; DELAY_WINDOW],
            filled: 0,
            next: 0,
        }
    }

    /// `gamma = 1 / (3 (n_f + 1))`.
    pub fn gain(&self) -> f64 {
        1.0 / (3.0 * (self.delay_ticks + 1.0))
    }

    /// `e`, in seconds: positive when commands arrive earlier than they need to.
    pub fn error(&self) -> f64 {
        self.error
    }

    /// The estimate of `n_f`.
    pub fn delay_ticks(&self) -> f64 {
        self.delay_ticks
    }

    /// A report of the newest command's margin alone, from a snapshot that says nothing of the
    /// path.
    pub fn report(&mut self, margin: f64) {
        self.error = margin - self.target;
    }

    /// A report: the server saw the newest command's margin as `margin` seconds, in a snapshot
    /// that arrived when the client was `ticks_ahead` ticks past the one the snapshot is the
    /// world after.
    ///
    /// By the lemma on lead, `ticks_ahead * h = RTT + mu`, so the round trip in ticks follows.
    pub fn observe(&mut self, margin: f64, ticks_ahead: f64) {
        self.report(margin);
        let round_trip = (ticks_ahead - margin / self.h).max(1.0);
        self.recent[self.next] = round_trip;
        self.next = (self.next + 1) % DELAY_WINDOW;
        self.filled = (self.filled + 1).min(DELAY_WINDOW);
        self.delay_ticks = self.recent[..self.filled]
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min);
    }

    /// How many ticks the client should be ahead of the newest snapshot's last tick: the lead
    /// `L + 1 = (RTT + mu*) / h` of the lemma.
    pub fn lead_ticks(&self) -> u32 {
        (self.delay_ticks + self.target / self.h).round().max(1.0) as u32
    }

    /// Whether the newest report found commands so late that the client should jump to the lead
    /// it should have, rather than wait for the stretch to get it there.
    pub fn far_behind(&self) -> bool {
        self.error < -JUMP_TICKS * self.h
    }

    /// How long the client's next tick lasts, on its own clock: `h + gamma e`, within 5% of `h`.
    pub fn tick_length(&self) -> f64 {
        let stretch = self.gain() * self.error;
        self.h + stretch.clamp(-MAX_STRETCH * self.h, MAX_STRETCH * self.h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: f64 = 1.0 / 60.0;

    #[test]
    fn the_gain_is_the_papers_for_the_delay() {
        let c = MarginController::new(H, 0.017, 6.0);
        assert!((c.gain() - 1.0 / 21.0).abs() < 1e-12);
    }

    #[test]
    fn a_margin_that_is_too_large_lengthens_the_tick_and_too_small_shortens_it() {
        let mut c = MarginController::new(H, 0.017, 6.0);
        assert!(
            (c.tick_length() - H).abs() < 1e-15,
            "no report, no correction"
        );
        c.observe(0.017 + 0.01, 7.0);
        assert!(c.tick_length() > H);
        c.observe(0.017 - 0.01, 7.0);
        assert!(c.tick_length() < H);
    }

    #[test]
    fn the_stretch_is_clamped_to_five_percent() {
        let mut c = MarginController::new(H, 0.017, 6.0);
        c.observe(1.0, 7.0);
        assert!((c.tick_length() - 1.05 * H).abs() < 1e-15);
        c.observe(-1.0, 7.0);
        assert!((c.tick_length() - 0.95 * H).abs() < 1e-15);
    }

    #[test]
    fn the_round_trip_is_read_off_the_lead() {
        // 6 ticks ahead with one tick of margin: the round trip is 5 ticks, whatever the
        // estimate was before.
        let mut c = MarginController::new(H, 0.017, 20.0);
        for _ in 0..200 {
            c.observe(H, 6.0);
        }
        assert!((c.delay_ticks() - 5.0).abs() < 1e-6, "{}", c.delay_ticks());
    }

    #[test]
    fn reports_that_were_queued_are_ignored_and_a_longer_path_is_followed() {
        let mut c = MarginController::new(H, 0.017, 6.0);
        for _ in 0..100 {
            c.observe(H, 6.0);
        }
        // A stall's backlog arrives late: it looks like a delay of 200 ticks. While fresh reports
        // are among the newest, the estimate is theirs.
        for _ in 0..DELAY_WINDOW - 1 {
            c.observe(H, 200.0);
        }
        assert!((c.delay_ticks() - 5.0).abs() < 1e-9, "{}", c.delay_ticks());
        // A path that really is longer takes over once the window has passed.
        for _ in 0..DELAY_WINDOW {
            c.observe(H, 16.0);
        }
        assert!((c.delay_ticks() - 15.0).abs() < 1e-9, "{}", c.delay_ticks());
    }
}
