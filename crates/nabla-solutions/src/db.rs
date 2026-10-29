//! The database of solutions: `solutions/<name>.rs` plus the index `solutions/index.tsv`.
//!
//! Every indexed solution solves every case of both suites. The index records its score and the
//! hash of its source, and `cargo test` checks that both are still accurate.

use nabla_optimization::eval::Report;
use nabla_optimization::harness::{source_dir, source_hash};
use std::io;
use std::path::PathBuf;

const HEADER: &str = "name\tscore\tmax_steps\tsource_hash";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// Total steps over the standard suite.
    pub score: usize,
    /// Largest step count over the standard suite.
    pub max_steps: usize,
    pub source_hash: String,
}

pub fn solutions_dir() -> PathBuf {
    source_dir().join("solutions")
}

fn index_path() -> PathBuf {
    solutions_dir().join("index.tsv")
}

pub fn source_path(name: &str) -> PathBuf {
    solutions_dir().join(format!("{name}.rs"))
}

/// Names are lowercase identifiers: they become file names and Rust module names.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Load the index, best score first. A missing index is an empty database.
pub fn load() -> io::Result<Vec<Entry>> {
    match std::fs::read_to_string(index_path()) {
        Ok(text) => parse_index(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Parse the contents of `index.tsv`.
pub fn parse_index(text: &str) -> io::Result<Vec<Entry>> {
    let bad = |line: &str| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("bad index line: {line:?}"),
        )
    };
    text.lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 4 {
                return Err(bad(line));
            }
            Ok(Entry {
                name: f[0].to_string(),
                score: f[1].parse().map_err(|_| bad(line))?,
                max_steps: f[2].parse().map_err(|_| bad(line))?,
                source_hash: f[3].to_string(),
            })
        })
        .collect()
}

fn save(entries: &mut [Entry]) -> io::Result<()> {
    entries.sort_by(|a, b| (a.score, &a.name).cmp(&(b.score, &b.name)));
    let mut text = format!("{HEADER}\n");
    for e in entries.iter() {
        text += &format!(
            "{}\t{}\t{}\t{}\n",
            e.name, e.score, e.max_steps, e.source_hash
        );
    }
    std::fs::write(index_path(), text)
}

/// Store a solution whose `report` came from evaluating exactly `src`. Replaces any solution of
/// the same name. Fails unless every case of both suites is solved.
pub fn submit(name: &str, src: &str, report: &Report) -> Result<Entry, String> {
    if !valid_name(name) {
        return Err(format!("invalid name {name:?}: use [a-z][a-z0-9_]*"));
    }
    let (Some(score), Some(max_steps)) = (report.score(), report.max_steps()) else {
        let failed: Vec<&str> = report.failures().map(|r| r.name.as_str()).collect();
        return Err(format!(
            "not every case is solved; failed: {}",
            failed.join(", ")
        ));
    };
    let entry = Entry {
        name: name.to_string(),
        score,
        max_steps,
        source_hash: source_hash(src),
    };
    let io = |e: io::Error| e.to_string();
    std::fs::create_dir_all(solutions_dir()).map_err(io)?;
    std::fs::write(source_path(name), src).map_err(io)?;
    let mut entries = load().map_err(io)?;
    entries.retain(|e| e.name != name);
    entries.push(entry.clone());
    save(&mut entries).map_err(io)?;
    Ok(entry)
}
