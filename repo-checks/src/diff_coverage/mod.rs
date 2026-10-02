//! Lines a change adds that no test executes. Rust lines are told apart by
//! whether a unit test could reach them at all; test code is not reported.

pub mod diff;
pub mod lcov;
pub mod rust;

use std::collections::BTreeMap;
use std::path::Path;

use diff::AddedLines;
use lcov::LineHits;
use rust::{FileAnalysis, Reach};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// A unit test could run the line and none does.
    Untested,
    /// Only e2e can run the line, and the unit tests did not.
    E2eOnly,
    /// A changed source file the coverage report does not mention at all, so
    /// nothing about its lines is known.
    NoCoverageData,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub path: String,
    pub line: Option<u32>,
    pub kind: Kind,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.kind {
            Kind::Untested => "untested",
            Kind::E2eOnly => "e2e-only",
            Kind::NoCoverageData => "no-coverage-data",
        };
        match self.line {
            Some(n) => write!(f, "{}:{n}\t{kind}", self.path),
            None => write!(f, "{}\t{kind}", self.path),
        }
    }
}

const UI_ROOT: &str = "ui/";

/// Whether `dir` (repo-relative) holds no production Rust: build output,
/// dependencies, hidden directories, and test code (including the e2e crate).
fn skipped_tree(dir: &str) -> bool {
    dir.split('/').next() == Some("e2e")
        || dir.split('/').any(|c| {
            c.starts_with('.')
                || matches!(
                    c,
                    "target" | "node_modules" | "tests" | "benches" | "examples"
                )
        })
}

fn is_rust_source(path: &str) -> bool {
    path.ends_with(".rs")
        && path
            .rsplit_once('/')
            .is_none_or(|(dir, _)| !skipped_tree(dir))
}

/// Mirrors `coverage.include` / `coverage.exclude` in ui/vite.config.ts.
fn is_ui_source(path: &str) -> bool {
    path.strip_prefix(UI_ROOT).is_some_and(|p| {
        p.starts_with("src/")
            && (p.ends_with(".ts") || p.ends_with(".tsx"))
            && !p.ends_with(".d.ts")
            && !p.contains(".test.")
            && p != "src/test-setup.ts"
    })
}

pub fn report(
    added: &AddedLines,
    hits: &LineHits,
    rust: &BTreeMap<String, FileAnalysis>,
) -> Result<Vec<Finding>, String> {
    let mut out = Vec::new();
    for (path, lines) in added {
        let file_hits = hits.get(path);
        if is_rust_source(path) {
            let analysis = rust
                .get(path)
                .ok_or_else(|| format!("{path}: a Rust source the analysis did not read"))?;
            let prod: Vec<(u32, Option<Reach>)> = lines
                .iter()
                .filter(|&&n| !analysis.is_test_line(n))
                .map(|&n| (n, analysis.reach(n)))
                .collect();
            match file_hits {
                // A file with no function in the change (only `mod` lines,
                // constants) legitimately has no coverage record.
                None if prod.iter().any(|(_, r)| r.is_some()) => {
                    out.push(Finding {
                        path: path.clone(),
                        line: None,
                        kind: Kind::NoCoverageData,
                    });
                }
                None => {}
                Some(h) => {
                    for (n, reach) in prod {
                        if h.get(&n) == Some(&0) {
                            let kind = match reach {
                                Some(Reach::E2eOnly) => Kind::E2eOnly,
                                _ => Kind::Untested,
                            };
                            out.push(Finding {
                                path: path.clone(),
                                line: Some(n),
                                kind,
                            });
                        }
                    }
                }
            }
        } else if is_ui_source(path) {
            match file_hits {
                None => out.push(Finding {
                    path: path.clone(),
                    line: None,
                    kind: Kind::NoCoverageData,
                }),
                Some(h) => {
                    for &n in lines {
                        if h.get(&n) == Some(&0) {
                            out.push(Finding {
                                path: path.clone(),
                                line: Some(n),
                                kind: Kind::Untested,
                            });
                        }
                    }
                }
            }
        }
    }
    out.sort();
    Ok(out)
}

pub fn run(
    root: &Path,
    diff: &Path,
    rust_lcov: &Path,
    ui_lcov: &Path,
) -> Result<Vec<Finding>, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("{}: {e}", root.display()))?;
    let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));

    let added = diff::parse_diff(&read(diff)?)?;
    let mut hits = LineHits::new();
    lcov::parse_lcov(&read(rust_lcov)?, &root, "", &mut hits)?;
    lcov::parse_lcov(&read(ui_lcov)?, &root, UI_ROOT, &mut hits)?;

    let mut sources = BTreeMap::new();
    collect_rust_sources(&root, &root, &mut sources)?;
    let analysis = rust::analyze(&sources)?;
    report(&added, &hits, &analysis)
}

fn collect_rust_sources(
    root: &Path,
    dir: &Path,
    out: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        let ty = entry.file_type().map_err(|e| format!("{rel}: {e}"))?;
        if ty.is_dir() {
            if skipped_tree(&rel) {
                continue;
            }
            collect_rust_sources(root, &path, out)?;
        } else if ty.is_file() && is_rust_source(&rel) {
            let source = std::fs::read_to_string(&path).map_err(|e| format!("{rel}: {e}"))?;
            out.insert(rel, source);
        }
    }
    Ok(())
}
