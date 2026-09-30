//! #215 の spec 引用検査を、検査本体の未実装で落とす。

use home_rss_shared::spec_refs::{
    CitedTest, MissingTest, check_contents, check_repo, format_missing, parse_spec_refs, test_names,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn files_of(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(path, content)| ((*path).to_string(), (*content).to_string()))
        .collect()
}

#[test]
fn parses_a_path_scoped_rust_reference() {
    // `` `path`: `name` `` の組を1件の引用として拾う。
    let cited =
        parse_spec_refs("- foo — `shared/src/ssrf.rs`: `accepts_https_url_with_public_host`");
    assert_eq!(
        cited,
        vec![CitedTest {
            line: 1,
            file: Some("shared/src/ssrf.rs".to_string()),
            name: "accepts_https_url_with_public_host".to_string(),
        }]
    );
}

#[test]
fn parses_a_bare_e2e_reference_without_a_path() {
    // `e2e: \`name\`` はファイルを持たない引用になる。
    let cited =
        parse_spec_refs("- foo — e2e: `adding_a_feed_rejects_urls_the_fetcher_must_not_reach`");
    assert_eq!(
        cited,
        vec![CitedTest {
            line: 1,
            file: None,
            name: "adding_a_feed_rejects_urls_the_fetcher_must_not_reach".to_string(),
        }]
    );
}

#[test]
fn splits_comma_separated_names_after_one_path() {
    // 1つの path の後のカンマ区切りは複数の引用になる。
    let cited = parse_spec_refs("- foo — `shared/src/ssrf.rs`: `first_name`, `second_name`");
    assert_eq!(
        cited,
        vec![
            CitedTest {
                line: 1,
                file: Some("shared/src/ssrf.rs".to_string()),
                name: "first_name".to_string(),
            },
            CitedTest {
                line: 1,
                file: Some("shared/src/ssrf.rs".to_string()),
                name: "second_name".to_string(),
            },
        ]
    );
}

#[test]
fn covers_multiple_paths_on_one_line() {
    // 1行に path の組が2つあるとき両方拾う。2つ目の区切りを見落とすと落ちる。
    let cited = parse_spec_refs(
        "- foo — `shared/src/feed.rs`: `feed_one`; \
         `shared/tests/feed_image_regressions.rs`: `regression_one`, `regression_two`",
    );
    assert_eq!(
        cited,
        vec![
            CitedTest {
                line: 1,
                file: Some("shared/src/feed.rs".to_string()),
                name: "feed_one".to_string(),
            },
            CitedTest {
                line: 1,
                file: Some("shared/tests/feed_image_regressions.rs".to_string()),
                name: "regression_one".to_string(),
            },
            CitedTest {
                line: 1,
                file: Some("shared/tests/feed_image_regressions.rs".to_string()),
                name: "regression_two".to_string(),
            },
        ]
    );
}

#[test]
fn ignores_backtick_pairs_without_a_citation_shape() {
    // コロンで結ばれていない `` `x` `` の並びは引用ではない。
    let cited = parse_spec_refs("- 記事一覧は `feed_id` と `unread=true` で絞れる");
    assert!(cited.is_empty());
}

#[test]
fn rust_test_names_require_a_test_attribute() {
    // 属性の無い `fn helper` はテストではない。存在だけ見ると落ちる。
    let names = test_names(
        "shared/src/ssrf.rs",
        "#[test]\nfn pinned() {}\nfn helper() {}\n#[tokio::test]\nasync fn async_pinned() {}\n",
    );
    assert_eq!(
        names,
        vec!["pinned".to_string(), "async_pinned".to_string()]
    );
}

#[test]
fn vitest_names_cover_it_and_test_but_not_describe() {
    // `describe` はテストではない。`it` / `test` のどちらかだけ見ると落ちる。
    let names = test_names(
        "ui/src/api.test.ts",
        "describe('api', () => {\n  it('does one thing', async () => {\n  test(\"does another\", () => {\n});\n",
    );
    assert_eq!(
        names,
        vec!["does one thing".to_string(), "does another".to_string()]
    );
}

#[test]
fn missing_report_names_the_spec_line_file_and_test() {
    // 失敗表示に spec の行・ファイル・無いテスト名の3点が載る。
    let missing = check_contents(
        "- foo — `shared/src/ssrf.rs`: `gone_name`\n",
        &files_of(&[("shared/src/ssrf.rs", "#[test]\nfn stays() {}\n")]),
    );
    assert_eq!(
        missing,
        vec![MissingTest {
            line: 1,
            file: "shared/src/ssrf.rs".to_string(),
            name: "gone_name".to_string(),
        }]
    );
    let shown = format_missing(&missing);
    assert!(
        shown.contains("shared/src/ssrf.rs"),
        "must name the file: {shown}"
    );
    assert!(shown.contains("gone_name"), "must name the test: {shown}");
    assert!(shown.contains('1'), "must name the spec line: {shown}");
}

#[test]
fn bare_e2e_references_search_every_file_under_e2e_tests() {
    // `e2e:` の path 無し引用は e2e/tests/ 配下の全ファイルから探す。
    let missing = check_contents(
        "- foo — e2e: `second_one`\n",
        &files_of(&[
            (
                "e2e/tests/api.rs",
                "#[tokio::test]\nasync fn first_one() {}\n",
            ),
            (
                "e2e/tests/extra.rs",
                "#[tokio::test]\nasync fn second_one() {}\n",
            ),
        ]),
    );
    assert!(missing.is_empty());
}

#[test]
fn checked_out_spec_has_no_missing_tests() {
    // 今の spec は検査を通る。数秒以内に終わることもここで見る。
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let start = Instant::now();
    let missing = check_repo(&root);
    assert!(
        missing.is_empty(),
        "current spec must pass: {}",
        format_missing(&missing)
    );
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "the check must finish in a few seconds"
    );
}

#[test]
fn whole_suite_invokes_the_spec_check() {
    // issue の「scripts/test.sh が回す」。呼び名を変えたらこのテストも直す。
    let test_sh = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/test.sh"),
    )
    .unwrap();
    assert!(
        test_sh.contains("spec_refs"),
        "scripts/test.sh must invoke the spec reference check"
    );
}
