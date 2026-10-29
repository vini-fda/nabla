//! Every stored solution, compiled into the library (feature `registry`).
//!
//! `build.rs` generates one module per `solutions/<name>.rs` plus the table [`SOLUTIONS`], and
//! embeds `solutions/index.tsv`. Use [`valid`] to get the solutions whose index entry matches
//! their source.

use crate::db::{Entry, parse_index};
use nabla_core::problem::{Case, Trajectory};
use nabla_optimization::harness::source_hash;

/// A stored solution, compiled in.
pub struct StoredSolution {
    pub name: &'static str,
    pub source: &'static str,
    /// Run this solution on a case, recording the trajectory.
    pub simulate: fn(&Case) -> Trajectory,
}

include!(concat!(env!("OUT_DIR"), "/registry.rs"));

/// Stored solutions whose index entry exists and matches their source hash, best score first.
pub fn valid() -> Vec<(&'static StoredSolution, Entry)> {
    let index = parse_index(INDEX).unwrap_or_default();
    let mut valid: Vec<_> = SOLUTIONS
        .iter()
        .filter_map(|sol| {
            let entry = index.iter().find(|e| e.name == sol.name)?;
            (entry.source_hash == source_hash(sol.source)).then(|| (sol, entry.clone()))
        })
        .collect();
    valid.sort_by_key(|(_, e)| (e.score, e.name.clone()));
    valid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stored_solution_is_valid() {
        assert_eq!(valid().len(), SOLUTIONS.len());
    }

    #[test]
    fn simulate_agrees_with_the_index() {
        let cases = nabla_core::suite::standard();
        for (sol, entry) in valid() {
            let total: usize = cases
                .iter()
                .map(|c| (sol.simulate)(c).outcome.steps().unwrap())
                .sum();
            assert_eq!(total, entry.score, "{}", sol.name);
        }
    }
}
