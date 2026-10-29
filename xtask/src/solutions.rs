//! The solution tasks: evaluate, submit, list and verify solutions (see `TASK.md`).

#![expect(clippy::print_stdout, reason = "these tasks report on stdout")]

use anyhow::{Context as _, Result, anyhow, bail};
use nabla_core::problem::Outcome;
use nabla_optimization::eval::{Report, SuiteKind};
use nabla_optimization::harness::{build_sources, source_hash};
use nabla_solutions::db;

fn read(path: &str) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {path}"))
}

/// Compile every source in parallel, then evaluate them one after the other: each evaluation
/// already runs its cases on every core.
fn evaluate_all(sources: &[&str]) -> Vec<Result<Report, String>> {
    build_sources(sources)
        .into_iter()
        .map(|candidate| {
            candidate
                .and_then(|c| c.evaluate())
                .map_err(|e| e.to_string())
        })
        .collect()
}

fn evaluate(src: &str) -> Result<Report> {
    let [report] = <[_; 1]>::try_from(evaluate_all(&[src])).expect("one source, one report");
    report.map_err(anyhow::Error::msg)
}

fn print_report(report: &Report) {
    for r in &report.results {
        let suite = match r.suite {
            SuiteKind::Standard => "standard",
            SuiteKind::Holdout => "holdout",
        };
        let outcome = match &r.outcome {
            Outcome::Reached(n) => format!("{n:>6} steps"),
            Outcome::Timeout => "FAILED: timeout".to_owned(),
            Outcome::Inadmissible { step, control } => {
                format!("FAILED: inadmissible control {control:?} at step {step}")
            }
            Outcome::Panic { step } => format!("FAILED: panic at step {step}"),
        };
        println!("{suite:<9} {:<20} {outcome}", r.name);
    }
    println!("{}", summary(report));
}

/// One line: cases solved, and the score if there is one.
fn summary(report: &Report) -> String {
    let solved = report
        .results
        .iter()
        .filter(|r| r.outcome.steps().is_some())
        .count();
    let total = report.results.len();
    match (report.score(), report.max_steps()) {
        (Some(score), Some(max)) => {
            format!("solved {solved}/{total} cases, score {score} (longest case: {max} steps)")
        }
        _ => format!("solved {solved}/{total} cases, no score (every case must be solved)"),
    }
}

/// `eval <solution.rs>...`: one file gets the per-case table; several build in parallel and
/// get one line each, best first.
pub fn eval(paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        bail!("eval needs at least one solution file");
    }
    let sources = paths.iter().map(|p| read(p)).collect::<Result<Vec<_>>>()?;
    let sources: Vec<&str> = sources.iter().map(String::as_str).collect();
    let reports = evaluate_all(&sources);

    if let [report] = reports.as_slice() {
        let report = report.as_ref().map_err(|e| anyhow!("{e}"))?;
        print_report(report);
        if !report.passed() {
            bail!("not every case is solved");
        }
        return Ok(());
    }

    let mut rows: Vec<_> = paths.iter().zip(&reports).collect();
    rows.sort_by_key(|(path, report)| {
        let score = report.as_ref().ok().and_then(Report::score);
        (score.is_none(), score, path.as_str())
    });
    let width = paths.iter().map(String::len).max().unwrap_or(0);
    for (path, report) in &rows {
        let status = match report {
            Ok(report) if report.passed() => summary(report),
            Ok(report) => {
                let failed: Vec<&str> = report.failures().map(|r| r.name.as_str()).collect();
                format!("{}; failed: {}", summary(report), failed.join(", "))
            }
            Err(e) => e.lines().next().unwrap_or_default().to_owned(),
        };
        println!("{path:<width$}  {status}");
    }
    for (path, report) in &rows {
        if let Err(e) = report {
            println!("\n{path}: {e}");
        }
    }
    let passed = reports
        .iter()
        .filter(|r| r.as_ref().is_ok_and(Report::passed))
        .count();
    if passed < reports.len() {
        bail!(
            "{} of {} solutions do not solve every case",
            reports.len() - passed,
            reports.len()
        );
    }
    Ok(())
}

/// `submit <name> <solution.rs>`: evaluate, and store if every case is solved.
pub fn submit(name: &str, path: &str) -> Result<()> {
    let src = read(path)?;
    let report = evaluate(&src)?;
    print_report(&report);
    let entry = db::submit(name, &src, &report).map_err(anyhow::Error::msg)?;
    println!("stored {} with score {}", entry.name, entry.score);
    Ok(())
}

/// `leaderboard`: the stored solutions, best first.
pub fn leaderboard() -> Result<()> {
    println!(
        "{:<4} {:<24} {:>8} {:>10}",
        "rank", "name", "score", "max_steps"
    );
    for (i, e) in db::load()?.iter().enumerate() {
        println!(
            "{:<4} {:<24} {:>8} {:>10}",
            i + 1,
            e.name,
            e.score,
            e.max_steps
        );
    }
    Ok(())
}

/// `verify`: re-evaluate every stored solution, all built in parallel, and compare with the
/// index.
pub fn verify() -> Result<()> {
    let entries = db::load()?;
    let sources = entries
        .iter()
        .map(|e| read(&db::source_path(&e.name).to_string_lossy()))
        .collect::<Result<Vec<_>>>()?;
    let sources: Vec<&str> = sources.iter().map(String::as_str).collect();
    let reports = evaluate_all(&sources);

    let mut ok = true;
    for ((e, src), report) in entries.iter().zip(&sources).zip(reports) {
        let status = if source_hash(src) != e.source_hash {
            "source changed since it was scored".to_owned()
        } else {
            match report {
                Ok(r) if (r.score(), r.max_steps()) == (Some(e.score), Some(e.max_steps)) => {
                    "ok".to_owned()
                }
                Ok(r) => format!(
                    "score is now {:?} (longest {:?}), index says {} (longest {})",
                    r.score(),
                    r.max_steps(),
                    e.score,
                    e.max_steps
                ),
                Err(err) => err,
            }
        };
        ok &= status == "ok";
        println!("{:<24} {status}", e.name);
    }
    if !ok {
        bail!("some stored solutions are not valid; resubmit them");
    }
    Ok(())
}
