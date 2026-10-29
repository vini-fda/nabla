//! Lint: the viewer's sources contain only ASCII. LLM-written text tends to pick up Unicode
//! (Greek letters, arrows, math symbols, typographic dashes). Spell these out instead
//! (`theta`, `->`, `<=`, `-`, `...`): it keeps UI text independent of the font's glyph coverage.

use std::fs;
use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("readable directory") {
        let path = entry.expect("readable entry").path();
        if path.is_dir() {
            collect(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("rs" | "html" | "css" | "js" | "json" | "toml")
        ) {
            out.push(path);
        }
    }
}

fn suggestion(c: char) -> &'static str {
    match c {
        '\u{3b1}'..='\u{3c9}' | '\u{391}'..='\u{3a9}' => {
            "spell out the Greek letter (alpha, tau, ...)"
        }
        '\u{2190}'..='\u{21ff}' => "use ->, <-, <->",
        '\u{2264}' | '\u{2265}' | '\u{2260}' | '\u{2248}' => "use <=, >=, !=, ~=",
        '\u{2013}' | '\u{2014}' => "use -",
        '\u{2018}' | '\u{2019}' | '\u{201c}' | '\u{201d}' => "use straight quotes",
        '\u{2026}' => "use ...",
        '\u{d7}' => "use x or *",
        '\u{b1}' => "use +/-",
        _ => "replace with ASCII",
    }
}

#[test]
fn viewer_sources_are_ascii() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in ["src", "tests", "assets"] {
        collect(&root.join(dir), &mut files);
    }
    files.push(root.join("index.html"));

    let mut problems = Vec::new();
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            for (col, c) in line.char_indices().filter(|(_, c)| !c.is_ascii()) {
                problems.push(format!(
                    "{}:{}:{}: U+{:04X} '{}': {}",
                    file.strip_prefix(root).unwrap_or(&file).display(),
                    i + 1,
                    col + 1,
                    c as u32,
                    c,
                    suggestion(c)
                ));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "non-ASCII characters:\n{}",
        problems.join("\n")
    );
}
