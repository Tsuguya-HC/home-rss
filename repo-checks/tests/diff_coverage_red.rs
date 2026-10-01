#[path = "diff_coverage_common.rs"]
mod diff_coverage_common;

use diff_coverage_common::{
    FixtureRepo, RUST_LCOV_ENV, UI_LCOV_ENV, lcov_record, report_lines, script_path,
};
use std::process::Command;

#[test]
fn array_sig_spin_use_is_e2e_only() {
    // Catches the `;` inside a fixed-length array type ending the function
    // range at the signature line: the Spin line below must stay e2e-only.
    let base = "pub fn checksum(data: [u8; 32]) -> u64 {\n    0\n}\n";
    let changed = "pub fn checksum(data: [u8; 32]) -> u64 {\n    let _c = spin_sdk::pg::Connection::open(\"x\").unwrap();\n    0\n}\n";
    let repo = FixtureRepo::new(&[("src/arr.rs", base)]);
    repo.commit_change(&[("src/arr.rs", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/arr.rs", &[(2, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/arr.rs:2".to_string(), "e2e-only".to_string())]
    );
}

#[test]
fn unsafe_fn_spin_use_is_e2e_only() {
    // Catches `unsafe` (with `async`) falling out of the fn pattern: the Spin
    // line below must stay e2e-only.
    let base = "pub async unsafe fn risky() {\n    0\n}\n";
    let changed = "pub async unsafe fn risky() {\n    let _c = spin_sdk::pg::Connection::open(\"x\").unwrap();\n    0\n}\n";
    let repo = FixtureRepo::new(&[("src/unsf.rs", base)]);
    repo.commit_change(&[("src/unsf.rs", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/unsf.rs", &[(2, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/unsf.rs:2".to_string(), "e2e-only".to_string())]
    );
}

#[test]
fn const_fn_spin_use_is_e2e_only() {
    // Catches `const` falling out of the fn pattern: the Spin line below must
    // stay e2e-only.
    let base = "pub const fn cached() -> i32 {\n    0\n}\n";
    let changed = "pub const fn cached() -> i32 {\n    let _c = spin_sdk::pg::Connection::open(\"x\").unwrap();\n    0\n}\n";
    let repo = FixtureRepo::new(&[("src/constf.rs", base)]);
    repo.commit_change(&[("src/constf.rs", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/constf.rs", &[(2, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/constf.rs:2".to_string(), "e2e-only".to_string())]
    );
}

#[test]
fn array_return_sig_spin_use_is_e2e_only() {
    // Catches the `;` inside a return-position array type ending the function
    // range at the signature line: the Spin line below must stay e2e-only.
    let base = "pub fn ret() -> [u8; 32] {\n    [0u8; 32]\n}\n";
    let changed = "pub fn ret() -> [u8; 32] {\n    let _c = spin_sdk::pg::Connection::open(\"x\").unwrap();\n    [0u8; 32]\n}\n";
    let repo = FixtureRepo::new(&[("src/arrret.rs", base)]);
    repo.commit_change(&[("src/arrret.rs", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/arrret.rs", &[(2, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/arrret.rs:2".to_string(), "e2e-only".to_string())]
    );
}

#[test]
fn unclosed_bracket_decl_does_not_swallow_later_spin_name() {
    // Catches the `]` arm being lost: without it an unclosed `[` in a
    // bodyless declaration leaks into the next fn, and its Spin-named
    // parameter promotes an unrelated comment line to e2e-only.
    let base =
        "trait T {\n    fn a(x: [u8; 32], http_service: u8);\n    fn c() {\n        0\n    }\n}\n";
    let changed = "trait T {\n    fn a(x: [u8; 32], http_service: u8);\n    // changed comment\n    fn c() {\n        0\n    }\n}\n";
    let repo = FixtureRepo::new(&[("src/unclosed.rs", base)]);
    repo.commit_change(&[("src/unclosed.rs", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/unclosed.rs", &[(3, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/unclosed.rs:3".to_string(), "untested".to_string())]
    );
}

#[test]
fn host_target_selection_is_pinned() {
    // Catches the host-target computation degrading to the wrong awk field:
    // verified with a mutation patch switching `$2` to `$1`.
    let body = std::fs::read_to_string(script_path()).unwrap();
    assert!(
        body.contains("awk '/^host:/{print $2}'"),
        "host target awk field"
    );
}

#[test]
fn extra_arg_is_rejected() {
    // Catches extra arguments being accepted: verified with a mutation patch
    // relaxing the arity check to `-lt 1`.
    let repo = FixtureRepo::new(&[("src/lib.rs", "pub fn present() {}\n")]);
    repo.commit_change(&[("src/lib.rs", "pub fn present() {}\npub fn added() {}\n")]);
    let rust_lcov = repo.write_lcov("rust.lcov", "TN:\n");
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let output = Command::new("bash")
        .arg(script_path())
        .arg(&repo.base)
        .arg("extra")
        .current_dir(&repo.dir)
        .env(RUST_LCOV_ENV, &rust_lcov)
        .env(UI_LCOV_ENV, &ui_lcov)
        .output()
        .unwrap();
    assert!(!output.status.success(), "two args must fail");
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
