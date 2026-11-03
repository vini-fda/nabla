//! The log format of `cargo nabla`, for development.
//!
//! It adds a timestamp, the thread when it is not the main one, and the source line to what the
//! default format says. `NABLA_LOG_STYLE=dev` in the environment chooses it, and `cargo xtask
//! run` sets that.

use std::io::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

/// What `RUST_LOG` is when it is not set, in the dev format: everything of ours at `debug`, the
/// rest at `info` but for the graphics stack, which says a lot at `info` and little of use.
pub const DEFAULT_FILTER: &str = "info,naga=warn,wgpu_core=warn,wgpu_hal=warn,\
     nabla_client=debug,nabla_net=debug,nabla_sim=debug,nabla_core=debug";

/// `ms` milliseconds after the Unix epoch, as `2026-11-02T20:56:46.472Z` (UTC).
pub fn iso_utc(ms: u64) -> String {
    let (days, in_day) = (ms / 86_400_000, ms % 86_400_000);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm), in unsigned
    // arithmetic from 0000-03-01, as 1970 is after it.
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    let (hours, minutes) = (in_day / 3_600_000, in_day / 60_000 % 60);
    let (seconds, millis) = (in_day / 1000 % 60, in_day % 1000);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z")
}

/// Make `builder` write the dev format.
pub fn format(builder: &mut env_logger::Builder) -> &mut env_logger::Builder {
    builder.format(|buf, record| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(0));
        let level = buf.default_level_style(record.level());
        let thread = std::thread::current();
        let thread = match thread.name() {
            Some("main") | None => String::new(),
            Some(name) => format!(" [{name}]"),
        };
        let place = match (record.file(), record.line()) {
            // The spans of `span!` all log from the macro, which says nothing of where they are.
            (Some(file), Some(line)) if !file.ends_with("span.rs") => format!(" {file}:{line}"),
            _ => String::new(),
        };
        writeln!(
            buf,
            "{} {level}{:<5}{level:#} {}{thread}{place}: {}",
            iso_utc(now),
            record.level(),
            record.target(),
            record.args()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_come_out_right() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            iso_utc(951_782_400_000 + 86_399_999),
            "2000-02-29T23:59:59.999Z"
        );
        assert_eq!(iso_utc(1_793_652_000_123), "2026-11-02T20:40:00.123Z");
        assert_eq!(iso_utc(4_107_542_400_000), "2100-03-01T00:00:00.000Z");
    }
}
