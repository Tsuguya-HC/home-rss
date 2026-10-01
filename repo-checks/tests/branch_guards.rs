use home_rss_repo_checks::spec_refs::{
    check_workspace_specs, find_missing_refs, parse_spec_citations, rust_test_names,
    vitest_test_names,
};
use std::path::PathBuf;

#[test]
fn unknown_citation_file_is_reported_not_skipped() {
    let citations = parse_spec_citations("- text — `shared/src/ssrf.rs`: `gone_a`\n");
    assert_eq!(citations.len(), 1);
    let missing = find_missing_refs(&citations, |_| None);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].test, "gone_a");
}

#[test]
fn unreadable_spec_is_reported_not_silent() {
    let missing = check_workspace_specs(&PathBuf::from("/nonexistent-repo-root-215"));
    assert!(!missing.is_empty());
    assert_eq!(missing[0].file, "docs/spec.md");
}

#[test]
fn rust_source_without_test_attributes_yields_no_names() {
    assert!(rust_test_names("fn helper() {}\n").is_empty());
}

#[test]
fn vitest_source_without_calls_yields_no_names() {
    assert!(vitest_test_names("const c = 1;\n").is_empty());
}

#[test]
fn quoted_list_with_trailing_comma_is_ignored() {
    let citations = parse_spec_citations("- text — `shared/src/ssrf.rs`: `accepts_a`, `gone_b`,\n");
    assert!(citations.is_empty());
}

#[test]
fn backticked_e2e_group_resolves_to_api_rs() {
    let line = "- text — `e2e`: `adding_a_feed_rejects_urls_the_fetcher_must_not_reach`";
    let got = parse_spec_citations(line);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].file, "e2e/tests/api.rs");
    assert_eq!(
        got[0].tests,
        vec!["adding_a_feed_rejects_urls_the_fetcher_must_not_reach"]
    );
}

#[test]
fn rust_test_fn_after_blank_and_comment_lines_is_found() {
    let source = "#[test]\n\n// comment\nfn blank_then_a() {}\n";
    let names = rust_test_names(source);
    assert!(names.contains(&"blank_then_a".to_string()));
}

#[test]
fn pub_test_fn_after_marker_is_found() {
    let source = "#[test]\nfn unit_a() {}\n#[test]\npub fn pub_b() {}\n";
    let names = rust_test_names(source);
    assert!(names.contains(&"pub_b".to_string()));
}

#[test]
fn vitest_dotted_only_call_is_found() {
    let source = "it.only('does a', () => {});\ntest.skip(\"does b\", () => {});\n";
    let names = vitest_test_names(source);
    assert!(names.contains(&"does a".to_string()));
    assert!(names.contains(&"does b".to_string()));
}

#[test]
fn vitest_backtick_name_is_found() {
    let source = "it(`does a`, () => {});\n";
    let names = vitest_test_names(source);
    assert!(names.contains(&"does a".to_string()));
}

#[test]
fn vitest_unquoted_name_is_ignored() {
    let source = "it(name, () => {});\n";
    assert!(vitest_test_names(source).is_empty());
}

#[test]
fn full_e2e_path_group_resolves_to_api_rs() {
    let line =
        "- text — `e2e/tests/api.rs`: `adding_a_feed_rejects_urls_the_fetcher_must_not_reach`";
    let got = parse_spec_citations(line);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].file, "e2e/tests/api.rs");
    assert_eq!(
        got[0].tests,
        vec!["adding_a_feed_rejects_urls_the_fetcher_must_not_reach"]
    );
}

#[test]
fn backticked_name_without_test_file_path_is_ignored() {
    let line = "- text — `adding_a_feed`: `some_name`";
    assert!(parse_spec_citations(line).is_empty());
}

#[test]
fn prose_with_only_test_dir_path_is_ignored() {
    let spec = "テストの場所は、Rust の単体テストと `shared/tests/` の統合テストがファイル名。\n";
    assert!(parse_spec_citations(spec).is_empty());
}

#[test]
fn prose_e2e_path_mention_without_tests_is_ignored() {
    let line = "テストの場所は、e2e が `e2e/tests/api.rs`。";
    assert!(parse_spec_citations(line).is_empty());
}

#[test]
fn stacked_test_attributes_yield_one_name() {
    let source = "#[test]\n#[ignore]\nfn stacked_a() {}\n";
    let names = rust_test_names(source);
    assert_eq!(names, vec!["stacked_a".to_string()]);
}

#[test]
fn pub_async_test_fn_is_found() {
    let source = "#[tokio::test]\npub async fn pub_async_b() {}\n";
    let names = rust_test_names(source);
    assert!(names.contains(&"pub_async_b".to_string()));
}

#[test]
fn eol_comment_after_test_name_is_ignored() {
    let source = "#[test]\nfn unit_a() {} // trailing\n";
    let names = rust_test_names(source);
    assert!(names.contains(&"unit_a".to_string()));
}

#[test]
fn test_ts_path_citation_is_parsed() {
    let line = "- text — `ui/src/api.test.ts`: `gives adding a feed 45 seconds before timing out`";
    let got = parse_spec_citations(line);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].file, "ui/src/api.test.ts");
}

#[test]
fn bare_e2e_without_colon_is_ignored() {
    let line = "- text e2e `adding_a_feed`";
    assert!(parse_spec_citations(line).is_empty());
}

#[test]
fn unterminated_backtick_is_ignored() {
    let line = "- text — `shared/src/ssrf.rs: `accepts_a`";
    assert!(parse_spec_citations(line).is_empty());
}

#[test]
fn path_without_colon_after_is_ignored() {
    let line = "- text — `shared/src/ssrf.rs` `accepts_a`";
    assert!(parse_spec_citations(line).is_empty());
}

#[test]
fn indented_rust_marker_is_found() {
    let source = "    #[test]\n    fn indented_a() {}\n";
    let names = rust_test_names(source);
    assert!(names.contains(&"indented_a".to_string()));
}

#[test]
fn marker_at_end_without_fn_is_skipped() {
    let source = "fn helper() {}\n#[test]\n";
    assert!(rust_test_names(source).is_empty());
}

#[test]
fn non_fn_item_after_marker_is_skipped() {
    let source = "#[test]\nconst A: u32 = 1;\nfn later() {}\n";
    let names = rust_test_names(source);
    assert!(!names.contains(&"later".to_string()));
}

#[test]
fn vitest_double_quoted_name_is_found() {
    let source = "test(\"does b\", () => {});\n";
    let names = vitest_test_names(source);
    assert!(names.contains(&"does b".to_string()));
}

#[test]
fn vitest_unterminated_quote_is_ignored() {
    let source = "it('does a, () => {});\n";
    assert!(vitest_test_names(source).is_empty());
}

#[test]
fn all_cited_tests_present_yields_no_missing() {
    let citations = parse_spec_citations("- text — `shared/src/ssrf.rs`: `accepts_a`\n");
    let missing = find_missing_refs(&citations, |_| Some(vec!["accepts_a".to_string()]));
    assert!(missing.is_empty());
}

#[test]
fn missing_source_file_is_reported() {
    let root = PathBuf::from("/nonexistent-repo-root-215");
    let missing = check_workspace_specs(&root);
    assert!(!missing.is_empty());
}

#[test]
fn line_starting_with_bare_e2e_is_parsed() {
    let got = parse_spec_citations("e2e: `adding_a_feed_rejects_urls_the_fetcher_must_not_reach`");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].file, "e2e/tests/api.rs");
}

#[test]
fn missing_colon_after_group_is_ignored() {
    let line = "- text — `shared/src/ssrf.rs` accepts_a";
    assert!(parse_spec_citations(line).is_empty());
}

#[test]
fn line_without_backticks_yields_nothing() {
    assert!(parse_spec_citations("- plain prose without code\n").is_empty());
}

#[test]
fn rust_source_ending_at_marker_yields_no_names() {
    assert!(rust_test_names("#[test]").is_empty());
}

#[test]
fn rust_marker_with_comment_only_after_yields_no_names() {
    let source = "#[test]\n// only a comment\n";
    assert!(rust_test_names(source).is_empty());
}

#[test]
fn rust_pub_fn_without_name_yields_no_panic() {
    let source = "#[test]\nfn \n";
    assert!(rust_test_names(source).is_empty());
}

#[test]
fn indented_vitest_call_is_found() {
    let source = "  it('does a', () => {});\n";
    let names = vitest_test_names(source);
    assert!(names.contains(&"does a".to_string()));
}

#[test]
fn vitest_dotted_without_paren_is_ignored() {
    let source = "it.only 'does a';\n";
    assert!(vitest_test_names(source).is_empty());
}

#[test]
fn underscore_prefixed_e2e_is_not_a_citation() {
    let line = "- text — some_e2e: `adding_a_feed`";
    let got = parse_spec_citations(line);
    assert!(got.iter().all(|c| c.file != "e2e/tests/api.rs"));
}

#[test]
fn colon_then_prose_before_backtick_is_ignored() {
    let line = "- text — `shared/src/ssrf.rs`: prose then `accepts_a`";
    assert!(parse_spec_citations(line).is_empty());
}

#[test]
fn unterminated_test_name_is_ignored() {
    let line = "- text — `shared/src/ssrf.rs`: `accepts_a";
    assert!(parse_spec_citations(line).is_empty());
}

#[test]
fn bare_fn_name_without_parens_is_found() {
    let source = "#[test]\nfn bare_name";
    let names = rust_test_names(source);
    assert!(names.contains(&"bare_name".to_string()));
}

#[test]
fn vitest_empty_parens_is_ignored() {
    let source = "it();\n";
    assert!(vitest_test_names(source).is_empty());
}

#[test]
fn bare_e2e_without_any_backtick_is_ignored() {
    assert!(parse_spec_citations("e2e: foo\n").is_empty());
}

#[test]
fn cited_file_missing_on_disk_is_reported() {
    let root = std::env::temp_dir().join(format!("repo-checks-215-{}", std::process::id()));
    std::fs::create_dir_all(root.join("docs")).unwrap();
    std::fs::write(
        root.join("docs/spec.md"),
        "- text — `nope/missing.rs`: `gone_a`\n",
    )
    .unwrap();
    let missing = check_workspace_specs(&root);
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].file, "nope/missing.rs");
    assert_eq!(missing[0].test, "gone_a");
}
