#[path = "diff_coverage_common.rs"]
mod diff_coverage_common;

use diff_coverage_common::{FixtureRepo, lcov_record, repo_root, report_lines, script_path};
use std::process::Command;

#[test]
fn usage_names_base_ref() {
    // Catches the script succeeding silently, or failing without naming its argument.
    let output = Command::new("bash")
        .arg(script_path())
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("base-ref"),
        "usage names <base-ref>: {combined:?}"
    );
}

#[test]
fn reports_uncovered_rust_change_as_untested() {
    // Catches the script dropping an uncovered Rust line, or labelling it e2e-only.
    let repo = FixtureRepo::new(&[("src/lib.rs", "pub fn present() {}\n")]);
    repo.commit_change(&[("src/lib.rs", "pub fn present() {}\npub fn added() {}\n")]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/lib.rs", &[(1, 1), (2, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/lib.rs:2".to_string(), "untested".to_string())]
    );
}

#[test]
fn omits_covered_rust_change() {
    // Catches the script listing a changed line that unit tests do execute.
    let repo = FixtureRepo::new(&[("src/lib.rs", "pub fn present() {}\n")]);
    repo.commit_change(&[("src/lib.rs", "pub fn present() {}\npub fn added() {}\n")]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/lib.rs", &[(1, 1), (2, 4)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    assert!(report_lines(&repo.run(&rust_lcov, &ui_lcov)).is_empty());
}

#[test]
fn marks_spin_runtime_users_e2e_only() {
    // Catches each of the four Spin-only patterns falling back to untested,
    // and a plain function being promoted to e2e-only. Spellings match the
    // tree (`Connection::open`, `variables::get`, `http::send`, `http_service`).
    let base = "pub fn load_pg() {\n    let _c = spin_sdk::pg::Connection::open(\"postgres://x\").unwrap();\n}\n\npub fn read_var() {\n    let _v = spin_sdk::variables::get(\"db_url\").unwrap();\n}\n\npub fn fetch_ext() {\n    let _r = spin_sdk::http::send(\"https://example.invalid\").await;\n}\n\n#[spin_sdk::http_service]\npub fn handle() {\n    route();\n}\n\npub fn plain() {\n}\n";
    let changed = "pub fn load_pg() {\n    let _c = spin_sdk::pg::Connection::open(\"postgres://x\").unwrap();\n    let touched_pg = 1;\n}\n\npub fn read_var() {\n    let _v = spin_sdk::variables::get(\"db_url\").unwrap();\n    let touched_var = 1;\n}\n\npub fn fetch_ext() {\n    let _r = spin_sdk::http::send(\"https://example.invalid\").await;\n    let touched_http = 1;\n}\n\n#[spin_sdk::http_service]\npub fn handle() {\n    route();\n    let touched_entry = 1;\n}\n\npub fn plain() {\n    let touched_plain = 1;\n}\n";
    let repo = FixtureRepo::new(&[("src/spin.rs", base)]);
    repo.commit_change(&[("src/spin.rs", changed)]);
    let rust_lcov = repo.write_lcov(
        "rust.lcov",
        &lcov_record("src/spin.rs", &[(3, 0), (8, 0), (13, 0), (19, 0), (23, 0)]),
    );
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let mut lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    lines.sort();
    assert_eq!(
        lines,
        vec![
            ("src/spin.rs:13".to_string(), "e2e-only".to_string()),
            ("src/spin.rs:19".to_string(), "e2e-only".to_string()),
            ("src/spin.rs:23".to_string(), "untested".to_string()),
            ("src/spin.rs:3".to_string(), "e2e-only".to_string()),
            ("src/spin.rs:8".to_string(), "e2e-only".to_string()),
        ]
    );
}

#[test]
fn reports_uncovered_ui_change_as_untested_only() {
    // Catches UI rows going missing, and the Rust e2e-only rule leaking into
    // UI files: the added comment names a Spin API yet must stay untested.
    let base = "export function widget() {\n}\n";
    let changed = "export function widget() {\n    // mentions spin_sdk::pg::Connection::open but stays plain UI code\n    return 1;\n}\n";
    let repo = FixtureRepo::new(&[("ui/src/widget.ts", base)]);
    repo.commit_change(&[("ui/src/widget.ts", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", "TN:\n");
    let ui_lcov = repo.write_lcov(
        "ui.lcov",
        &lcov_record("ui/src/widget.ts", &[(1, 1), (2, 0), (3, 0)]),
    );
    let mut lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    lines.sort();
    assert_eq!(
        lines,
        vec![
            ("ui/src/widget.ts:2".to_string(), "untested".to_string()),
            ("ui/src/widget.ts:3".to_string(), "untested".to_string()),
        ]
    );
}

#[test]
fn script_measures_host_rust_and_vitest_coverage() {
    // Catches the script bypassing either coverage source (e.g. git diff only).
    let body = std::fs::read_to_string(script_path()).unwrap();
    assert!(body.contains("cargo llvm-cov"), "rust unit-test coverage");
    assert!(body.contains("vitest"), "ui coverage");
}

#[test]
fn ui_coverage_tooling_is_present() {
    // Catches the script having nothing to run for UI coverage with.
    let package = std::fs::read_to_string(repo_root().join("ui/package.json")).unwrap();
    assert!(
        package.contains("@vitest/coverage-"),
        "vitest coverage provider devDependency"
    );
    let vite = std::fs::read_to_string(repo_root().join("ui/vite.config.ts")).unwrap();
    assert!(vite.contains("coverage"), "vitest coverage settings");
}
