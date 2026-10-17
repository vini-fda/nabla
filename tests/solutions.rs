//! Every stored solution solves every case, and its index entry is up to date.

use ship2d_optimization::harness::source_hash;
use ship2d_optimization::{Controller, db, eval};

fn check_solution<C: Controller>(name: &str, src: &str) {
    let report = eval::evaluate::<C>();
    let failed: Vec<_> = report.failures().map(|r| (&r.name, &r.outcome)).collect();
    assert!(failed.is_empty(), "{name} fails: {failed:?}");

    let entry = db::load()
        .unwrap()
        .into_iter()
        .find(|e| e.name == name)
        .unwrap_or_else(|| panic!("{name} is not in solutions/index.tsv; use the submit command"));
    assert_eq!(
        entry.source_hash,
        source_hash(src),
        "{name} changed since it was scored"
    );
    assert_eq!(
        Some(entry.score),
        report.score(),
        "{name}: stale score in the index"
    );
    assert_eq!(
        Some(entry.max_steps),
        report.max_steps(),
        "{name}: stale max_steps in the index"
    );
}

include!(concat!(env!("OUT_DIR"), "/solution_tests.rs"));
