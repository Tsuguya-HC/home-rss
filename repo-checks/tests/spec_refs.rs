use home_rss_repo_checks::spec_refs::{
    check_spec, parse_spec_refs, rust_test_names, vitest_test_names,
};

#[test]
fn parses_semicolon_separated_groups_without_merging_paths() {
    let refs = parse_spec_refs(
        "— `server/src/lib.rs`: `a_test`（note）、`ui/src/api.test.ts`: `some ui name`",
    );
    assert_eq!(refs.len(), 2);
    assert_eq!(refs[0].path, "server/src/lib.rs");
    assert_eq!(refs[0].name, "a_test");
    assert_eq!(refs[1].path, "ui/src/api.test.ts");
    assert_eq!(refs[1].name, "some ui name");
}

#[test]
fn parses_comma_separated_names_for_one_path() {
    let refs = parse_spec_refs("— `shared/src/feed.rs`: `first_test`, `second_test`");
    assert_eq!(refs.len(), 2);
    assert!(refs.iter().all(|r| r.path == "shared/src/feed.rs"));
}

#[test]
fn excludes_parenthesised_mentions_and_bare_e2e_ok() {
    // Bare `e2e:` (no backticks, as in docs/spec.md) resolves to the e2e file,
    // and `（...）` asides contribute no refs.
    let refs = parse_spec_refs("— e2e: `a_e2e_ok`（note `not_a_test`）");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].path, "e2e/tests/api.rs");
    assert_eq!(refs[0].name, "a_e2e_ok");
}

#[test]
fn finds_test_fns_regardless_of_attribute_and_ignores_helpers() {
    // `#[test]` / `#[tokio::test]` are both plain fns; the plain helper `fresh_db` must not match.
    let names = rust_test_names(
        "#[tokio::test]\nasync fn e2e_ok() {}\n#[test]\nfn unit_ok() {}\nasync fn fresh_db() {}",
    );
    assert!(names.contains(&"e2e_ok".to_string()));
    assert!(names.contains(&"unit_ok".to_string()));
    assert!(!names.contains(&"fresh_db".to_string()));
}

#[test]
fn finds_vitest_names_in_both_quote_styles() {
    let names =
        vitest_test_names("it('single quoted', () => {});\ntest(\"double quoted\", () => {});");
    assert!(names.contains(&"single quoted".to_string()));
    assert!(names.contains(&"double quoted".to_string()));
}

#[test]
fn check_reports_missing_tests_with_spec_line() {
    let missing = check_spec("line one\n— `server/src/lib.rs`: `gone_test`\n", &|_| {
        Some("fn other_test() {}".to_string())
    });
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].line, 2);
    assert_eq!(missing[0].path, "server/src/lib.rs");
    assert_eq!(missing[0].name, "gone_test");
}

#[test]
fn check_accepts_present_tests() {
    let missing = check_spec("— `server/src/lib.rs`: `here_test`\n", &|_| {
        Some("fn here_test() {}".to_string())
    });
    assert!(missing.is_empty());
}

#[test]
fn repo_spec_cites_only_tests_that_exist() {
    // Catches stale citations like the #196 / #201 renames.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let spec = std::fs::read_to_string(root.join("docs/spec.md")).unwrap();
    let missing = check_spec(&spec, &|path| std::fs::read_to_string(root.join(path)).ok());
    assert!(
        missing.is_empty(),
        "{}",
        missing
            .iter()
            .map(|m| format!("spec line {}: {}: {}", m.line, m.path, m.name))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn ignores_pathless_name_colon_pairs_outside_citations() {
    let refs = parse_spec_refs("— `server/src/lib.rs`: `real_test`, note: `not_a_test`");
    assert_eq!(refs.len(), 2);
    assert!(refs.iter().all(|r| r.path == "server/src/lib.rs"));
    assert!(refs.iter().any(|r| r.name == "real_test"));
    assert!(refs.iter().any(|r| r.name == "not_a_test"));
}

#[test]
fn reads_backticked_e2e_path_as_the_e2e_file() {
    let refs = parse_spec_refs("— `e2e`: `an_e2e_ok`");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].path, "e2e/tests/api.rs");
    assert_eq!(refs[0].name, "an_e2e_ok");
}

#[test]
fn reads_space_before_colon_in_bare_e2e_prefix() {
    let refs = parse_spec_refs("— e2e : `spaced_e2e_ok`");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].path, "e2e/tests/api.rs");
    assert_eq!(refs[0].name, "spaced_e2e_ok");
}

#[test]
fn skips_missing_files_as_missing_refs() {
    let missing = check_spec("— `server/src/lib.rs`: `gone_test`\n", &|_| None);
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].path, "server/src/lib.rs");
    assert_eq!(missing[0].name, "gone_test");
}

#[test]
fn skips_vitest_entries_without_string_literals() {
    let names = vitest_test_names("it(variable, () => {});");
    assert!(names.is_empty());
}

#[test]
fn picks_same_line_fn_after_test_attribute() {
    let names = rust_test_names("#[test] fn same_line_ok() {}");
    assert!(names.contains(&"same_line_ok".to_string()));
}

#[test]
fn ignores_citation_shaped_text_before_the_dash() {
    let refs = parse_spec_refs("see `server/src/lib.rs`: `pre_test` — text");
    assert!(refs.is_empty());
}

#[test]
fn ignores_plain_fn_without_attribute() {
    let names = rust_test_names("fn plain_helper() {}");
    assert!(names.is_empty());
}

#[test]
fn reads_escaped_quote_inside_vitest_name() {
    let names = vitest_test_names("it('don\\'t stop', () => {});");
    assert!(names.contains(&"don't stop".to_string()));
}

#[test]
fn skips_word_prefixed_fake_fn() {
    let names = rust_test_names("#[test]\nmyfn not_a_test() {}");
    assert!(names.is_empty());
}

#[test]
fn ignores_non_test_attributes() {
    let names = rust_test_names("#[allow(dead_code)]\nfn allowed_helper() {}");
    assert!(names.is_empty());
}

#[test]
fn ignores_identifier_prefixed_bare_e2e() {
    let refs = parse_spec_refs("— xe2e: `x_ok`");
    assert!(refs.is_empty());
}

#[test]
fn ignores_vitest_calls_prefixed_by_identifier() {
    let names = vitest_test_names("denyit('x_ok', () => {});\nlatest('y_ok', () => {});");
    assert!(names.is_empty());
}

#[test]
fn ignores_test_prefixed_attributes() {
    let names = rust_test_names("#[testing]\nfn testing_helper() {}");
    assert!(names.is_empty());
}

#[test]
fn ignores_vitest_calls_followed_by_identifier() {
    let names = vitest_test_names("its('x_ok', () => {});\ntests('y_ok', () => {});");
    assert!(names.is_empty());
}
