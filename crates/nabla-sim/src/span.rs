//! Timing of the costly operations, for the log: their start, and their end with how long they
//! took. In `RUST_LOG` (natively) or `?log=` (on the web), as everywhere in the app:
//!
//! * `info`: what happens in a session: a flight starts, the autopilot engages, a world loads.
//! * `debug`: the operations done once in a while, such as building the physics or a new
//!   controller, and a control sample over its sampling period.
//! * `trace`: the operations of every frame or sample.

use log::Level;
use web_time::Instant;

/// A costly operation under way: logs its end when dropped. See [`span!`](crate::span!).
#[must_use = "the span ends when dropped"]
pub struct Span {
    level: Level,
    target: &'static str,
    label: String,
    started: Instant,
}

impl Span {
    /// Logs the start of the operation called `label`. Use [`span!`](crate::span!) instead.
    pub fn start(level: Level, target: &'static str, label: String) -> Self {
        log::log!(target: target, level, "{label}: start");
        Self {
            level,
            target,
            label,
            started: Instant::now(),
        }
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let took = self.started.elapsed().as_secs_f64() * 1e3;
        log::log!(target: self.target, self.level, "{}: end, {took:.3} ms", self.label);
    }
}

/// Logs the start and the end of an operation, with how long it took.
///
/// The start is logged at `level`, in the caller's module, and the end when the guard
/// this returns is dropped, with how long it took. Described as with `format!`. Costs nothing
/// when `level` is off, not even the description:
///
/// ```ignore
/// let _span = span!(Level::Debug, "orbit capture check, body {body}");
/// ```
#[macro_export]
macro_rules! span {
    ($level:expr, $($description:tt)+) => {
        log::log_enabled!($level).then(|| {
            $crate::span::Span::start($level, module_path!(), format!($($description)+))
        })
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The messages of this module, whatever else the other tests log meanwhile.
    static LOGGED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    struct Capture;

    fn undescribable() -> &'static str {
        panic!("a span that's off isn't even described");
    }

    impl log::Log for Capture {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= Level::Debug
        }

        fn log(&self, record: &log::Record<'_>) {
            if record.target() == module_path!() {
                LOGGED
                    .lock()
                    .expect("not poisoned")
                    .push(format!("{}", record.args()));
            }
        }

        fn flush(&self) {}
    }

    #[test]
    fn a_span_logs_its_start_and_its_end_only_when_its_level_is_on() {
        log::set_logger(&Capture).expect("the only logger of the tests");
        log::set_max_level(log::LevelFilter::Debug);

        {
            let span = span!(Level::Debug, "work {}", 7);
            assert!(span.is_some());
            let logged = LOGGED.lock().expect("not poisoned").clone();
            assert_eq!(logged, ["work 7: start"]);
        }
        let logged = LOGGED.lock().expect("not poisoned").clone();
        assert_eq!(logged.len(), 2);
        assert!(
            logged[1].starts_with("work 7: end, ") && logged[1].ends_with(" ms"),
            "{logged:?}"
        );

        let off = span!(Level::Trace, "{}", undescribable());
        assert!(off.is_none());
    }
}
