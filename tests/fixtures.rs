use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[path = "harness/invariants.rs"]
mod invariants;

use filecheck::{CheckerBuilder, NO_VARIABLES};
use isle_lsp::modes::{self, Input, Query};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "isle") {
            out.push(path);
        }
    }
}

fn modes_of(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix(';')?.trim_start().strip_prefix("mode:"))
        .flat_map(|rest| rest.split(',').map(|m| m.trim().to_owned()).collect::<Vec<_>>())
        .filter(|m| !m.is_empty())
        .collect()
}

fn queries_of(source: &str) -> Vec<Query> {
    let mut queries = Vec::new();
    let mut target = 0;
    for (idx, line) in source.lines().enumerate() {
        let after_semi = line.trim_start().strip_prefix(';').map(str::trim_start);
        match after_semi.filter(|rest| rest.starts_with('^')) {
            Some(rest) => {
                let col = line.find('^').unwrap();
                let (_, text) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                let text = text.trim();
                let (kind, args) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
                queries.push(Query {
                    line: target,
                    col,
                    kind: kind.to_owned(),
                    args: args.trim().to_owned(),
                });
            }
            None => target = idx,
        }
    }
    queries
}

fn run(relative: &str, path: &Path) -> Result<(), String> {
    let source = fs::read_to_string(path).map_err(|e| e.to_string())?;
    invariants::check(&source).map_err(|e| format!("invariant: {e}"))?;
    let queries = queries_of(&source);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let input = Input { source: &source, queries: &queries, path, root: &root };

    let mut output = String::new();
    for mode in modes_of(&source) {
        let rendered = modes::render(&mode, &input)?;
        output.push_str(&format!("=== {mode} ===\n{rendered}\n"));
    }

    let mut builder = CheckerBuilder::new();
    builder.text(&source).map_err(|e| format!("bad directive: {e}"))?;
    let checker = builder.finish();
    if checker.is_empty() {
        return Ok(());
    }
    let (ok, explanation) = checker
        .explain(&output, NO_VARIABLES)
        .map_err(|e| format!("bad directive: {e}"))?;
    if ok {
        Ok(())
    } else {
        if let Some(dir) = std::env::var_os("FIXTURES_ACTUAL_DIR") {
            let target = Path::new(&dir).join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, &output).unwrap();
        }
        Err(format!("{explanation}\n--- actual output ---\n{output}"))
    }
}

fn main() -> ExitCode {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let filter = std::env::args().skip(1).find(|a| !a.starts_with('-'));
    let mut files = Vec::new();
    collect(&root, &mut files);

    let (mut passed, mut failed) = (0, 0);
    for path in files {
        let name = path.strip_prefix(&root).unwrap().display().to_string();
        if filter.as_ref().is_some_and(|f| !name.contains(f.as_str())) {
            continue;
        }
        match run(&name, &path) {
            Ok(()) => passed += 1,
            Err(msg) => {
                failed += 1;
                eprintln!("FAIL {name}\n{msg}\n");
            }
        }
    }
    println!("fixtures: {passed} passed, {failed} failed");
    if failed == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
