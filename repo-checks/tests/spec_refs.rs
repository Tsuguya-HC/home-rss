use home_rss_repo_checks::spec_refs::{
    check_workspace_specs, find_missing_refs, parse_spec_citations, rust_test_names,
    vitest_test_names,
};
use std::path::PathBuf;

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
fn parses_ui_test_citations_with_test_tsx_paths() {
    let line = "- 画像の無い記事は — `ui/src/components/ArticleListItem.test.tsx`: `renders no thumbnail`, `hides the thumbnail`";
    let got = parse_spec_citations(line);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].file, "ui/src/components/ArticleListItem.test.tsx");
    assert_eq!(
        got[0].tests,
        vec!["renders no thumbnail", "hides the thumbnail"]
    );
}

#[test]
fn reports_a_cited_test_missing_from_its_file() {
    let citations = parse_spec_citations("- text — `shared/src/ssrf.rs`: `accepts_a`, `gone_b`\n");
    assert_eq!(citations.len(), 1);
    let missing = find_missing_refs(&citations, |file| {
        assert_eq!(file, "shared/src/ssrf.rs");
        Some(vec!["accepts_a".to_string()])
    });
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].line, 1);
    assert_eq!(missing[0].file, "shared/src/ssrf.rs");
    assert_eq!(missing[0].test, "gone_b");
}

#[test]
fn current_spec_has_no_missing_test_references() {
    let missing = check_workspace_specs(&repo_root());
    assert!(
        missing.is_empty(),
        "spec cites tests that do not exist: {}",
        missing
            .iter()
            .map(|m| format!("line {} {} `{}`", m.line, m.file, m.test))
            .collect::<Vec<_>>()
            .join(", ")
    );
}
