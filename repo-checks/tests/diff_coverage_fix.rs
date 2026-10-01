#[path = "diff_coverage_common.rs"]
mod diff_coverage_common;

use diff_coverage_common::{FixtureRepo, lcov_record, report_lines, script_path};

#[test]
fn fix_bodyless_fn_followed_by_default_method_stays_separate() {
    // Catches the bodyless-`fn` guard being lost: without it the declarations
    // leak past the trait's closing brace, and the added function's header
    // line is judged by a leaked declaration range instead of its own body.
    let base = "trait T {\n    fn decl();\n    fn added();\n}\n";
    let changed = "trait T {\n    fn decl();\n    fn added();\n}\nfn real_fn() {\n    spin_sdk::pg::Connection::open(\"x\").unwrap();\n}\n";
    let repo = FixtureRepo::new(&[("src/decl3.rs", base)]);
    repo.commit_change(&[("src/decl3.rs", changed)]);
    let rust_lcov = repo.write_lcov(
        "rust.lcov",
        &lcov_record("src/decl3.rs", &[(5, 0), (6, 0), (7, 0)]),
    );
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let mut lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    lines.sort();
    assert_eq!(
        lines,
        vec![
            ("src/decl3.rs:5".to_string(), "e2e-only".to_string()),
            ("src/decl3.rs:6".to_string(), "e2e-only".to_string()),
            ("src/decl3.rs:7".to_string(), "e2e-only".to_string()),
        ]
    );
}

#[test]
fn fix_changed_line_inside_nested_fn_uses_inner_range() {
    // Catches the innermost-range choice being lost: without it a changed line
    // inside the nested fn is judged by the outer body and misses the Spin use.
    let base = "pub fn outer() {\n    fn helper() {\n        spin_sdk::pg::Connection::open(\"x\").unwrap();\n    }\n}\n";
    let changed = "pub fn outer() {\n    fn helper() {\n        spin_sdk::pg::Connection::open(\"x\").unwrap();\n        let touched_inner = 1;\n    }\n}\n";
    let repo = FixtureRepo::new(&[("src/nested2.rs", base)]);
    repo.commit_change(&[("src/nested2.rs", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/nested2.rs", &[(4, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/nested2.rs:4".to_string(), "e2e-only".to_string())]
    );
}

#[test]
fn fix_empty_paren_uri_query_stays_untested() {
    // Catches the pg-query guard being lost: the http Uri accessor takes no
    // arguments, so it must not count as a pg query.
    let base = "pub fn path_of(uri: &Uri) -> String {\n    uri.query().unwrap_or_default().to_owned()\n}\n";
    let changed = "pub fn path_of(uri: &Uri) -> String {\n    let touched = 1;\n    uri.query().unwrap_or_default().to_owned()\n}\n";
    let repo = FixtureRepo::new(&[("src/uriq.rs", base)]);
    repo.commit_change(&[("src/uriq.rs", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/uriq.rs", &[(2, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/uriq.rs:2".to_string(), "untested".to_string())]
    );
}

#[test]
fn fix_real_tool_command_lines_are_pinned() {
    // Catches the coverage command lines being weakened beyond the four
    // lcov-output flags: each command must keep selecting the host target,
    // the workspace, the vitest provider, and actually running.
    let body = std::fs::read_to_string(script_path()).unwrap();
    assert!(body.contains("--workspace"), "rust workspace flag");
    assert!(body.contains("--target"), "rust host target flag");
    assert!(
        body.contains("--coverage.provider=v8"),
        "vitest coverage provider"
    );
    assert!(body.contains("vitest run"), "vitest run invocation");
    assert!(body.contains("cargo llvm-cov"), "rust coverage invocation");
}
