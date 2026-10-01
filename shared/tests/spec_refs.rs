use home_rss_shared::spec_refs::{
    check_workspace_specs, find_missing_refs, parse_spec_citations, rust_test_names,
    vitest_test_names, workspace_suite_invokes_spec_check,
};
use std::path::PathBuf;
use std::time::Instant;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn parses_backtick_path_with_comma_separated_tests() {
    let line = "- text — `shared/src/ssrf.rs`: `accepts_a`, `rejects_b`";
    let got = parse_spec_citations(line);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].line, 1);
    assert_eq!(got[0].file, "shared/src/ssrf.rs");
    assert_eq!(got[0].tests, vec!["accepts_a", "rejects_b"]);
}

#[test]
fn parses_semicolon_separated_groups_sharing_one_line_number() {
    let line = "- text — `shared/src/feed.rs`: `parses_a`; `shared/tests/feed_image_regressions.rs`: `regression_b`";
    let got = parse_spec_citations(line);
    assert_eq!(got.len(), 2);
    assert!(got.iter().all(|c| c.line == 1));
    assert_eq!(got[0].file, "shared/src/feed.rs");
    assert_eq!(got[0].tests, vec!["parses_a"]);
    assert_eq!(got[1].file, "shared/tests/feed_image_regressions.rs");
    assert_eq!(got[1].tests, vec!["regression_b"]);
}

#[test]
fn parses_bare_e2e_citations() {
    let line = "- text — e2e: `adding_a_feed_rejects_urls_the_fetcher_must_not_reach`";
    let got = parse_spec_citations(line);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].file, "e2e/tests/api.rs");
    assert_eq!(
        got[0].tests,
        vec!["adding_a_feed_rejects_urls_the_fetcher_must_not_reach"]
    );
}

#[test]
fn ignores_plain_prose_and_non_test_backticks() {
    let spec = "no citations here, just `articles` and `store()`\n";
    assert!(parse_spec_citations(spec).is_empty());
}

#[test]
fn finds_rust_test_and_tokio_test_functions() {
    let source =
        "#[test]\nfn unit_a() {}\n#[tokio::test]\nasync fn async_b() {}\nfn helper_c() {}\n";
    let names = rust_test_names(source);
    assert!(names.contains(&"unit_a".to_string()));
    assert!(names.contains(&"async_b".to_string()));
    assert!(!names.contains(&"helper_c".to_string()));
}

#[test]
fn finds_vitest_it_and_test_names() {
    let source = "it('does a', () => {});\ntest(\"does b\", () => {});\nconst c = 1;\n";
    let names = vitest_test_names(source);
    assert!(names.contains(&"does a".to_string()));
    assert!(names.contains(&"does b".to_string()));
}

#[test]
fn names_missing_test_with_spec_line_file_and_name() {
    let spec = "first\n- text — `shared/src/ssrf.rs`: `no_such_test`\n";
    let missing = find_missing_refs(spec, &|_| Some("#[test]\nfn real_test() {}\n".to_string()));
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].line, 2);
    assert_eq!(missing[0].file, "shared/src/ssrf.rs");
    assert_eq!(missing[0].test, "no_such_test");
}

#[test]
fn checked_out_spec_has_no_missing_tests() {
    let start = Instant::now();
    let missing = check_workspace_specs(&repo_root());
    assert!(
        missing.is_empty(),
        "spec cites tests that do not exist: {missing:?}"
    );
    assert!(
        start.elapsed().as_secs() < 10,
        "spec check took too long for an offline check"
    );
}

#[test]
fn whole_suite_invokes_the_spec_check() {
    assert!(workspace_suite_invokes_spec_check(&repo_root()));
}
