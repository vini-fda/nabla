//! Evaluate a controller on both suites, and the text format used to report results from a
//! compiled candidate back to the harness.

use crate::model::{Control, State};
use crate::problem::{Case, Controller, Outcome, Trajectory, run, simulate};
use crate::suite;
use std::fmt::Write;
use std::thread;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuiteKind {
    Standard,
    Holdout,
}

impl SuiteKind {
    fn name(self) -> &'static str {
        match self {
            SuiteKind::Standard => "standard",
            SuiteKind::Holdout => "holdout",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "standard" => Some(SuiteKind::Standard),
            "holdout" => Some(SuiteKind::Holdout),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CaseResult {
    pub suite: SuiteKind,
    pub name: String,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub results: Vec<CaseResult>,
}

/// Prefix of report lines, so that anything else a solution prints is ignored.
const MARK: &str = "@ship2d";

impl Report {
    /// Every case of both suites was solved.
    pub fn passed(&self) -> bool {
        self.results.iter().all(|r| r.outcome.steps().is_some())
    }

    /// The score: total steps over the standard suite, if every case of both suites is solved.
    pub fn score(&self) -> Option<usize> {
        self.passed().then(|| self.standard_steps().sum())
    }

    /// Largest step count over the standard suite, if every case of both suites is solved.
    pub fn max_steps(&self) -> Option<usize> {
        self.passed()
            .then(|| self.standard_steps().max().unwrap_or(0))
    }

    fn standard_steps(&self) -> impl Iterator<Item = usize> + '_ {
        self.results
            .iter()
            .filter(|r| r.suite == SuiteKind::Standard)
            .filter_map(|r| r.outcome.steps())
    }

    pub fn failures(&self) -> impl Iterator<Item = &CaseResult> {
        self.results.iter().filter(|r| r.outcome.steps().is_none())
    }

    /// Serialize as one marked, tab-separated line per case, plus header and footer lines.
    pub fn to_text(&self) -> String {
        let mut out = format!("{MARK}\tbegin\t{}\n", suite::SUITE_VERSION);
        for r in &self.results {
            let detail = outcome_fields(&r.outcome);
            writeln!(
                out,
                "{MARK}\tcase\t{}\t{}\t{detail}",
                r.suite.name(),
                r.name
            )
            .unwrap();
        }
        writeln!(out, "{MARK}\tend").unwrap();
        out
    }

    /// Parse the output of [`Report::to_text`], ignoring unmarked lines.
    pub fn parse(text: &str) -> Result<Report, String> {
        let mut lines = text
            .lines()
            .filter_map(|l| l.strip_prefix(MARK)?.strip_prefix('\t'));
        let version = format!("begin\t{}", suite::SUITE_VERSION);
        if lines.next() != Some(version.as_str()) {
            return Err(format!(
                "missing or mismatched header (expected suite v{})",
                suite::SUITE_VERSION
            ));
        }
        let mut results = Vec::new();
        for line in lines {
            if line == "end" {
                return Ok(Report { results });
            }
            let f: Vec<&str> = line.split('\t').collect();
            let bad = || format!("malformed report line: {line:?}");
            if f.len() < 4 || f[0] != "case" {
                return Err(bad());
            }
            let outcome = parse_outcome(&f[3..]).ok_or_else(bad)?;
            let suite = SuiteKind::parse(f[1]).ok_or_else(bad)?;
            results.push(CaseResult {
                suite,
                name: f[2].to_string(),
                outcome,
            });
        }
        Err("report truncated (no end line)".to_string())
    }
}

/// Tab-separated fields describing an outcome.
fn outcome_fields(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Reached(n) => format!("reached\t{n}"),
        Outcome::Timeout => "timeout".to_string(),
        Outcome::Inadmissible { step, control } => {
            format!(
                "inadmissible\t{step}\t{}\t{}",
                control.thrust, control.torque
            )
        }
        Outcome::Panic { step } => format!("panic\t{step}"),
    }
}

/// Inverse of [`outcome_fields`], from already split fields.
fn parse_outcome(f: &[&str]) -> Option<Outcome> {
    let num = |i: usize| f.get(i)?.parse::<usize>().ok();
    let real = |i: usize| f.get(i)?.parse::<f64>().ok();
    Some(match *f.first()? {
        "reached" => Outcome::Reached(num(1)?),
        "timeout" => Outcome::Timeout,
        "inadmissible" => Outcome::Inadmissible {
            step: num(1)?,
            control: Control {
                thrust: real(2)?,
                torque: real(3)?,
            },
        },
        "panic" => Outcome::Panic { step: num(1)? },
        _ => return None,
    })
}

/// Serialize a trajectory as marked, tab-separated lines: one `s` line per state, one `u` line
/// per control, then the outcome. `f64` values round-trip exactly.
pub fn trajectory_to_text(t: &Trajectory) -> String {
    let mut out = format!("{MARK}\ttrace\t{}\n", suite::SUITE_VERSION);
    for s in &t.states {
        writeln!(
            out,
            "{MARK}\ts\t{}\t{}\t{}\t{}\t{}\t{}",
            s.x, s.y, s.vx, s.vy, s.theta, s.omega
        )
        .unwrap();
    }
    for u in &t.controls {
        writeln!(out, "{MARK}\tu\t{}\t{}", u.thrust, u.torque).unwrap();
    }
    writeln!(out, "{MARK}\toutcome\t{}", outcome_fields(&t.outcome)).unwrap();
    writeln!(out, "{MARK}\tend").unwrap();
    out
}

/// Parse the output of [`trajectory_to_text`], ignoring unmarked lines.
pub fn parse_trajectory(text: &str) -> Result<Trajectory, String> {
    let mut lines = text
        .lines()
        .filter_map(|l| l.strip_prefix(MARK)?.strip_prefix('\t'));
    let header = format!("trace\t{}", suite::SUITE_VERSION);
    if lines.next() != Some(header.as_str()) {
        return Err(format!(
            "missing or mismatched trace header (expected suite v{})",
            suite::SUITE_VERSION
        ));
    }
    let (mut states, mut controls, mut outcome) = (Vec::new(), Vec::new(), None);
    for line in lines {
        let f: Vec<&str> = line.split('\t').collect();
        let bad = || format!("malformed trace line: {line:?}");
        let reals = || -> Result<Vec<f64>, String> {
            f[1..]
                .iter()
                .map(|v| v.parse::<f64>().map_err(|_| bad()))
                .collect()
        };
        match f[0] {
            "s" => match reals()?.as_slice() {
                &[x, y, vx, vy, theta, omega] => states.push(State {
                    x,
                    y,
                    vx,
                    vy,
                    theta,
                    omega,
                }),
                _ => return Err(bad()),
            },
            "u" => match reals()?.as_slice() {
                &[thrust, torque] => controls.push(Control { thrust, torque }),
                _ => return Err(bad()),
            },
            "outcome" => outcome = Some(parse_outcome(&f[1..]).ok_or_else(bad)?),
            "end" => {
                let outcome = outcome.ok_or("trace has no outcome")?;
                if states.len() != controls.len() + 1 {
                    return Err("trace must have one more state than controls".to_string());
                }
                return Ok(Trajectory {
                    states,
                    controls,
                    outcome,
                });
            }
            _ => return Err(bad()),
        }
    }
    Err("trace truncated (no end line)".to_string())
}

/// Run `C` on every case, in parallel. Each case gets its own controller instance.
fn run_all<C: Controller>(cases: &[Case]) -> Vec<Outcome> {
    let workers = thread::available_parallelism().map_or(1, |n| n.get());
    let chunk = cases.len().div_ceil(workers).max(1);
    thread::scope(|scope| {
        let handles: Vec<_> = cases
            .chunks(chunk)
            .map(|chunk| scope.spawn(move || chunk.iter().map(run::<C>).collect::<Vec<_>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("evaluation thread panicked"))
            .collect()
    })
}

/// Evaluate `C` on the standard and holdout suites.
pub fn evaluate<C: Controller>() -> Report {
    let mut results = Vec::new();
    for (kind, cases) in [
        (SuiteKind::Standard, suite::standard()),
        (SuiteKind::Holdout, suite::holdout()),
    ] {
        let outcomes = run_all::<C>(&cases);
        results.extend(
            cases
                .into_iter()
                .zip(outcomes)
                .map(|(case, outcome)| CaseResult {
                    suite: kind,
                    name: case.name,
                    outcome,
                }),
        );
    }
    Report { results }
}

/// Entry point of a compiled candidate.
///
/// * no arguments: evaluate both suites and print the report;
/// * `trace <case>`: simulate one case of either suite and print its trajectory.
pub fn candidate_main<C: Controller>() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, name] if command == "trace" => match suite::find(name) {
            Some(case) => print!("{}", trajectory_to_text(&simulate::<C>(&case))),
            None => {
                eprintln!("unknown case {name:?}");
                std::process::exit(2);
            }
        },
        _ => print!("{}", evaluate::<C>().to_text()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_round_trips() {
        let report = Report {
            results: vec![
                CaseResult {
                    suite: SuiteKind::Standard,
                    name: "a".into(),
                    outcome: Outcome::Reached(12),
                },
                CaseResult {
                    suite: SuiteKind::Standard,
                    name: "b".into(),
                    outcome: Outcome::Timeout,
                },
                CaseResult {
                    suite: SuiteKind::Holdout,
                    name: "c".into(),
                    outcome: Outcome::Inadmissible {
                        step: 3,
                        control: Control {
                            thrust: 1.0000000000000002,
                            torque: -0.25,
                        },
                    },
                },
                CaseResult {
                    suite: SuiteKind::Holdout,
                    name: "d".into(),
                    outcome: Outcome::Panic { step: 7 },
                },
            ],
        };
        let text = format!("noise from the solution\n{}more noise\n", report.to_text());
        assert_eq!(Report::parse(&text), Ok(report));
    }

    #[test]
    fn trajectory_round_trips() {
        let case = suite::find("spinning_moving").unwrap();
        struct Coast;
        impl Controller for Coast {
            fn new(_: &crate::Task) -> Self {
                Coast
            }
            fn control(&mut self, k: usize, _: &State) -> Control {
                Control {
                    thrust: 0.1 * (k % 3) as f64,
                    torque: -0.3,
                }
            }
        }
        let mut short = case.clone();
        short.task.max_steps = 25;
        let trajectory = simulate::<Coast>(&short);
        assert_eq!(trajectory.outcome, Outcome::Timeout);
        let text = format!("noise\n{}", trajectory_to_text(&trajectory));
        assert_eq!(parse_trajectory(&text), Ok(trajectory));
    }

    #[test]
    fn truncated_report_is_rejected() {
        let text = Report { results: vec![] }
            .to_text()
            .replace("@ship2d\tend\n", "");
        assert!(Report::parse(&text).is_err());
    }
}
