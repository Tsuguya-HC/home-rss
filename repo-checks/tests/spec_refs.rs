use home_rss_repo_checks::spec_refs::{
    check_workspace_specs, find_missing_refs, parse_spec_citations, rust_test_names,
    vitest_test_names,
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
    assert_eq!(
        got[0].tests,
        vec!["accepts_a".to_string(), "rejects_b".to_string()]
    );
}

#[test]
fn parses_bare_e2e_citations() {
    let line = "- text — e2e: `adding_a_feed_rejects_urls_the_fetcher_must_not_reach`";
    let got = parse_spec_citations(line);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].file, "e2e/tests/api.rs");
    assert_eq!(
        got[0].tests,
        vec!["adding_a_feed_rejects_urls_the_fetcher_must_not_reach".to_string()]
    );
}

#[test]
fn parses_semicolon_separated_groups_sharing_one_line_number() {
    let line = "- text — `shared/src/feed.rs`: `parses_a`; `shared/tests/feed_image_regressions.rs`: `regression_b`";
    let got = parse_spec_citations(line);
    assert_eq!(got.len(), 2);
    assert!(got.iter().all(|c| c.line == 1));
    assert_eq!(got[0].file, "shared/src/feed.rs");
    assert_eq!(got[0].tests, vec!["parses_a".to_string()]);
    assert_eq!(got[1].file, "shared/tests/feed_image_regressions.rs");
    assert_eq!(got[1].tests, vec!["regression_b".to_string()]);
}

#[test]
fn ignores_prose_paths_mentions_and_untested_markers() {
    let spec = "テストの場所は、Rust の単体テストと `shared/tests/` の統合テストがファイル名、e2e が `e2e/tests/api.rs`、UI が `*.test.ts(x)`。\n\
        - ガードは追加時だけでなく `fetch_and_store()` の入口で毎回かかる — テスト無し\n";
    assert!(parse_spec_citations(spec).is_empty());
}

#[test]
fn ignores_prose_path_without_colon() {
    let line = "即時取得と fetcher の取得は同じ `shared/src/fetch.rs` の `fetch_only()` が行い、保存は同じ `store()` を呼ぶ。";
    assert!(parse_spec_citations(line).is_empty());
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
        "spec cites tests that do not exist: {}",
        missing
            .iter()
            .map(|m| format!("{}:{} `{}`", m.file, m.line, m.test))
            .collect::<Vec<_>>()
            .join(", ")
    );
    assert!(
        start.elapsed().as_secs() < 10,
        "spec check took too long for an offline check"
    );
}

#[test]
fn parses_spaced_display_names_and_ideographic_comma_lists() {
    let line = "- text — `ui/src/api.test.ts`: `gives adding a feed 45 seconds before timing out`、`does b`";
    let got = parse_spec_citations(line);
    assert_eq!(got.len(), 1);
    assert_eq!(
        got[0].tests,
        vec![
            "gives adding a feed 45 seconds before timing out".to_string(),
            "does b".to_string()
        ]
    );
}

#[test]
fn parses_semicolon_separated_tests_sharing_one_file() {
    let line = "- text — `shared/src/ssrf.rs`: `accepts_a`; `rejects_b`";
    let got = parse_spec_citations(line);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].file, "shared/src/ssrf.rs");
    assert_eq!(
        got[0].tests,
        vec!["accepts_a".to_string(), "rejects_b".to_string()]
    );
}

#[test]
fn ignores_backtick_path_without_trailing_colon() {
    let line = "- text — `shared/src/ssrf.rs` `accepts_a`";
    assert!(parse_spec_citations(line).is_empty());
}

#[test]
fn ignores_slash_or_paren_inside_test_name() {
    let line = "- text — `shared/src/ssrf.rs`: `not/a_test`";
    assert!(parse_spec_citations(line).is_empty());
    let line = "- text — `shared/src/ssrf.rs`: `not_a_test()`";
    assert!(parse_spec_citations(line).is_empty());
}

#[test]
fn ignores_e2e_word_prefix_and_missing_files() {
    let line = "- text — see2e: `whatever`";
    assert!(parse_spec_citations(line).is_empty());
    let spec = "- text — `no/such.rs`: `missing_test`\n";
    let missing = find_missing_refs(spec, &|_| None);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].file, "no/such.rs");
    assert_eq!(missing[0].test, "missing_test");
}

#[test]
fn ignores_e2e_inside_backticks() {
    let line = "- text `e2e: whatever` — `shared/src/ssrf.rs`: `accepts_a`";
    let got = parse_spec_citations(line);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].file, "shared/src/ssrf.rs");
}

#[test]
fn accepts_tsx_citations_and_rejects_bare_filenames() {
    let got = parse_spec_citations("- text — `ui/src/api.test.tsx`: `does_a`");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].file, "ui/src/api.test.tsx");
    assert!(parse_spec_citations("- text — `ssrf.rs`: `does_a`").is_empty());
}

#[test]
fn rejects_e2e_without_cited_name_and_missing_source_files() {
    assert!(parse_spec_citations("- text — e2e whatever `x`").is_empty());
    let missing = find_missing_refs("- text — `no/such.rs`: `x`\n", &|_| None);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].line, 1);
    let empty = find_missing_refs("- text — `shared/src/ssrf.rs`: `x`\n", &|_| {
        Some(String::new())
    });
    assert_eq!(empty.len(), 1);
}

#[test]
fn missing_spec_file_yields_no_missing_refs() {
    let missing = check_workspace_specs(std::path::Path::new("/nonexistent-root-for-spec-check"));
    assert!(missing.is_empty());
}
