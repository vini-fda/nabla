//! Generates code for the stored solutions (`solutions/*.rs` at the workspace root):
//!
//! * `solution_tests.rs`: one test per solution, included by `tests/solutions.rs`.
//! * `registry.rs`: one module per solution plus the `SOLUTIONS` table and the embedded index,
//!   included by `src/registry.rs` (feature `registry`).

use std::{env, fs, path::Path};

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
    // `crates/nabla-solutions/../..`
    let root = Path::new(&manifest).ancestors().nth(2).unwrap();
    let dir = root.join("solutions");
    println!("cargo:rerun-if-changed={}", dir.display());

    let mut paths: Vec<_> = fs::read_dir(&dir)
        .map(|entries| entries.filter_map(|e| Some(e.ok()?.path())).collect())
        .unwrap_or_default();
    paths.retain(|p| p.extension().is_some_and(|e| e == "rs"));
    paths.sort();

    let mut tests = String::new();
    let mut registry = String::new();
    let mut table = String::new();
    for path in &paths {
        let name = path.file_stem().unwrap().to_string_lossy();
        tests += &format!(
            "#[path = {path:?}]\nmod sol_{name};\n\n\
             #[test]\nfn {name}() {{\n    check_solution::<sol_{name}::Solution>({name:?}, include_str!({path:?}));\n}}\n\n",
        );
        registry += &format!("#[path = {path:?}]\nmod sol_{name};\n");
        table += &format!(
            "    StoredSolution {{\n        name: {name:?},\n        source: include_str!({path:?}),\n        \
             simulate: nabla_core::problem::simulate::<sol_{name}::Solution>,\n    }},\n",
        );
    }
    let index = dir.join("index.tsv");
    let index = if index.exists() {
        format!("include_str!({index:?})")
    } else {
        "\"\"".to_string()
    };
    registry += &format!(
        "\n/// Every stored solution.\npub static SOLUTIONS: &[StoredSolution] = &[\n{table}];\n\n\
         /// The contents of `solutions/index.tsv`.\npub const INDEX: &str = {index};\n"
    );

    let out = Path::new(&env::var("OUT_DIR").unwrap()).to_path_buf();
    fs::write(out.join("solution_tests.rs"), tests).unwrap();
    fs::write(out.join("registry.rs"), registry).unwrap();
}
