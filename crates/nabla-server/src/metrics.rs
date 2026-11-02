//! What a running server measures, for the log and for the tests that hold it to its budget.

use std::time::Duration;

/// The tick times kept, newest last: a quarter hour at 60 Hz.
const KEPT: usize = 1 << 16;

/// Counters and tick times. A report is a copy.
#[derive(Clone, Debug, Default)]
pub struct Metrics {
    /// Ticks run.
    pub ticks: u64,
    /// Ships flying now.
    pub pilots: usize,
    /// Connections ever accepted, and refused for want of room.
    pub accepted: u64,
    pub refused: u64,
    /// Datagrams in and out, and their payloads in bytes (QUIC's overhead not counted).
    pub datagrams_in: u64,
    pub datagrams_out: u64,
    pub payload_in: u64,
    pub payload_out: u64,
    /// Datagrams dropped on the server's side: a full inbox, or a full outbox, or too large for
    /// the path.
    pub dropped_in: u64,
    pub dropped_out: u64,
    pub oversize: u64,
    /// UDP bytes of connections that have ended, QUIC and IP-less headers included.
    pub udp_in: u64,
    pub udp_out: u64,
    /// The largest snapshot sent, in bytes.
    pub largest_datagram: usize,
    /// How long each recent tick took to compute and hand off, in microseconds.
    work_us: Vec<u32>,
    next: usize,
    /// How late each recent tick started, in microseconds, behind `R(T)`.
    late_us: Vec<u32>,
}

impl Metrics {
    pub(crate) fn tick(&mut self, work: Duration, late: Duration) {
        self.ticks += 1;
        let work = u32::try_from(work.as_micros()).unwrap_or(u32::MAX);
        let late = u32::try_from(late.as_micros()).unwrap_or(u32::MAX);
        if self.work_us.len() < KEPT {
            self.work_us.push(work);
            self.late_us.push(late);
        } else {
            self.work_us[self.next] = work;
            self.late_us[self.next] = late;
            self.next = (self.next + 1) % KEPT;
        }
    }

    /// The `p`th percentile (0 to 1) of the tick times kept, in microseconds.
    pub fn work_percentile(&self, p: f64) -> u32 {
        percentile(&self.work_us, p)
    }

    /// The `p`th percentile of how late ticks started.
    pub fn late_percentile(&self, p: f64) -> u32 {
        percentile(&self.late_us, p)
    }
}

fn percentile(samples: &[u32], p: f64) -> u32 {
    if samples.is_empty() {
        return 0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let index = ((sorted.len() - 1) as f64 * p.clamp(0.0, 1.0)).round() as usize;
    sorted[index]
}

impl std::fmt::Display for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ticks, {} pilots; tick work p50 {} us, p99 {} us, max {} us; start lateness p99 {} us; \
             {} datagrams in ({} B), {} out ({} B, largest {} B); dropped {} in, {} out, {} oversize; \
             {} connections, {} refused",
            self.ticks,
            self.pilots,
            self.work_percentile(0.5),
            self.work_percentile(0.99),
            self.work_percentile(1.0),
            self.late_percentile(0.99),
            self.datagrams_in,
            self.payload_in,
            self.datagrams_out,
            self.payload_out,
            self.largest_datagram,
            self.dropped_in,
            self.dropped_out,
            self.oversize,
            self.accepted,
            self.refused,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_of_what_was_kept() {
        let mut m = Metrics::default();
        for us in 1..=100_u64 {
            m.tick(Duration::from_micros(us), Duration::ZERO);
        }
        assert_eq!(m.work_percentile(0.0), 1);
        assert_eq!(m.work_percentile(1.0), 100);
        assert_eq!(m.work_percentile(0.99), 99);
        assert_eq!(Metrics::default().work_percentile(0.5), 0);
    }

    #[test]
    fn old_samples_are_overwritten() {
        let mut m = Metrics::default();
        for _ in 0..KEPT {
            m.tick(Duration::from_micros(1000), Duration::ZERO);
        }
        for _ in 0..KEPT {
            m.tick(Duration::from_micros(5), Duration::ZERO);
        }
        assert_eq!(m.work_percentile(1.0), 5);
        assert_eq!(m.ticks, 2 * KEPT as u64);
    }
}
