//! Command-line front end: evaluate, submit and list solutions.

use ship2d_optimization::eval::{Report, SuiteKind};
use ship2d_optimization::harness::{evaluate_source, source_hash};
use ship2d_optimization::problem::Outcome;
use ship2d_optimization::{db, suite};
use std::process::ExitCode;

const USAGE: &str = "\
usage:
  ship2d_optimization eval <solution.rs>          compile and evaluate a solution
  ship2d_optimization submit <name> <solution.rs> evaluate, then store it if it solves every case
  ship2d_optimization leaderboard                 list stored solutions, best first
  ship2d_optimization verify                      re-evaluate every stored solution";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["eval", path] => eval(path),
        ["submit", name, path] => submit(name, path),
        ["leaderboard"] => leaderboard(),
        ["verify"] => verify(),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))
}

fn evaluate(src: &str) -> Result<Report, String> {
    evaluate_source(src).map_err(|e| e.to_string())
}

fn print_report(report: &Report) {
    for r in &report.results {
        let suite = match r.suite {
            SuiteKind::Standard => "standard",
            SuiteKind::Holdout => "holdout",
        };
        let outcome = match &r.outcome {
            Outcome::Reached(n) => format!("{n:>6} steps"),
            Outcome::Timeout => "FAILED: timeout".to_string(),
            Outcome::Inadmissible { step, control } => {
                format!("FAILED: inadmissible control {control:?} at step {step}")
            }
            Outcome::Panic { step } => format!("FAILED: panic at step {step}"),
        };
        println!("{suite:<9} {:<20} {outcome}", r.name);
    }
    let solved = report
        .results
        .iter()
        .filter(|r| r.outcome.steps().is_some())
        .count();
    println!(
        "solved {solved}/{} cases (suite v{})",
        report.results.len(),
        suite::SUITE_VERSION
    );
    match (report.score(), report.max_steps()) {
        (Some(score), Some(max)) => println!("score: {score} total steps (longest case: {max})"),
        _ => println!("score: none (every case of both suites must be solved)"),
    }
}

fn eval(path: &str) -> Result<(), String> {
    let report = evaluate(&read(path)?)?;
    print_report(&report);
    if report.passed() {
        Ok(())
    } else {
        Err("not every case is solved".into())
    }
}

fn submit(name: &str, path: &str) -> Result<(), String> {
    let src = read(path)?;
    let report = evaluate(&src)?;
    print_report(&report);
    let entry = db::submit(name, &src, &report)?;
    println!("stored {} with score {}", entry.name, entry.score);
    Ok(())
}

fn leaderboard() -> Result<(), String> {
    let entries = db::load().map_err(|e| e.to_string())?;
    println!(
        "{:<4} {:<24} {:>8} {:>10} {:>6}",
        "rank", "name", "score", "max_steps", "suite"
    );
    for (i, e) in entries.iter().enumerate() {
        println!(
            "{:<4} {:<24} {:>8} {:>10} {:>6}",
            i + 1,
            e.name,
            e.score,
            e.max_steps,
            format!("v{}", e.suite_version)
        );
    }
    Ok(())
}

fn verify() -> Result<(), String> {
    let mut ok = true;
    for e in db::load().map_err(|e| e.to_string())? {
        let src = read(&db::source_path(&e.name).to_string_lossy())?;
        let status = if source_hash(&src) != e.source_hash {
            "source changed since it was scored".to_string()
        } else if e.suite_version != suite::SUITE_VERSION {
            format!(
                "scored on suite v{}, current is v{}",
                e.suite_version,
                suite::SUITE_VERSION
            )
        } else {
            match evaluate(&src) {
                Ok(r) if r.score() == Some(e.score) => "ok".to_string(),
                Ok(r) => format!("score is now {:?}, index says {}", r.score(), e.score),
                Err(err) => err,
            }
        };
        ok &= status == "ok";
        println!("{:<24} {status}", e.name);
    }
    if ok {
        Ok(())
    } else {
        Err("some stored solutions are not valid".into())
    }
}
