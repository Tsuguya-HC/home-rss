use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use home_rss_repo_checks::diff_coverage::diff::parse_diff;
use home_rss_repo_checks::diff_coverage::lcov::{LineHits, parse_lcov};
use home_rss_repo_checks::diff_coverage::rust::{Reach, analyze};
use home_rss_repo_checks::diff_coverage::{Finding, Kind, report, run};

fn lines(v: &[u32]) -> BTreeSet<u32> {
    v.iter().copied().collect()
}

// ---- diff ----

#[test]
fn diff_records_added_lines_by_new_side_number() {
    let diff = "\
diff --git a/src/a.rs b/src/a.rs
index 1..2 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -3,2 +3,3 @@ fn x() {
 keep
-old
+new1
+new2
@@ -20 +21,0 @@
-gone
";
    let added = parse_diff(diff).unwrap();
    assert_eq!(added.len(), 1);
    assert_eq!(added["src/a.rs"], lines(&[4, 5]));
}

#[test]
fn diff_counts_are_what_end_a_hunk_not_line_prefixes() {
    // `++counter;` added reads `+++counter;`, a removed `-- note` reads
    // `--- note`; both look like file headers to a prefix check.
    let diff = "\
diff --git a/ui/src/c.ts b/ui/src/c.ts
--- a/ui/src/c.ts
+++ b/ui/src/c.ts
@@ -1,2 +1,2 @@
--- note
+++counter;
 x
";
    assert_eq!(parse_diff(diff).unwrap()["ui/src/c.ts"], lines(&[1]));
}

#[test]
fn diff_reads_several_files_and_a_missing_count_as_one() {
    let diff = "\
diff --git a/a.rs b/a.rs
--- a/a.rs
+++ b/a.rs
@@ -5 +5 @@
-a
+b
diff --git a/b.rs b/b.rs
new file mode 100644
--- /dev/null
+++ b/b.rs
@@ -0,0 +1,2 @@
+one
+two
";
    let added = parse_diff(diff).unwrap();
    assert_eq!(added["a.rs"], lines(&[5]));
    assert_eq!(added["b.rs"], lines(&[1, 2]));
}

#[test]
fn diff_ignores_deleted_files_and_no_newline_markers() {
    let diff = "\
diff --git a/old.rs b/old.rs
deleted file mode 100644
--- a/old.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-a
-b
diff --git a/k.rs b/k.rs
--- a/k.rs
+++ b/k.rs
@@ -1 +1 @@
-x
\\ No newline at end of file
+y
\\ No newline at end of file
";
    let added = parse_diff(diff).unwrap();
    assert!(!added.contains_key("old.rs"));
    assert_eq!(added["k.rs"], lines(&[1]));
}

#[test]
fn diff_treats_a_bare_empty_line_in_a_hunk_as_context() {
    let diff = "\
--- a/e.rs
+++ b/e.rs
@@ -1,3 +1,3 @@
 a

-b
+c
";
    assert_eq!(parse_diff(diff).unwrap()["e.rs"], lines(&[3]));
}

#[test]
fn diff_rejects_what_it_cannot_attribute() {
    let quoted = "--- \"a/x y.rs\"\n+++ \"b/x y.rs\"\n@@ -1 +1 @@\n-a\n+b\n";
    assert!(
        parse_diff(quoted)
            .unwrap_err()
            .contains("unsupported file header")
    );

    let truncated = "--- a/t.rs\n+++ b/t.rs\n@@ -1,2 +1,2 @@\n a\n";
    assert!(
        parse_diff(truncated)
            .unwrap_err()
            .contains("ends inside a hunk")
    );

    let longer = "--- a/o.rs\n+++ b/o.rs\n@@ -1 +1 @@\n-a\n+b\n+c\n";
    assert!(parse_diff(longer).unwrap_err().contains("outside a hunk"));

    for header in ["@@ -x +1 @@", "@@ -1 +1 x", "@@ -1 +1"] {
        let bad = format!("--- a/h.rs\n+++ b/h.rs\n{header}\n");
        let e = parse_diff(&bad).unwrap_err();
        assert!(e.contains("malformed hunk header"), "{header}: {e}");
    }

    let bad_line = "--- a/l.rs\n+++ b/l.rs\n@@ -1,2 +1,2 @@\n a\n?b\n";
    assert!(
        parse_diff(bad_line)
            .unwrap_err()
            .contains("unexpected line")
    );
}

#[test]
fn diff_reads_a_path_with_spaces_despite_the_trailing_tab_git_adds() {
    let diff = "--- a/a b.txt\t\n+++ b/a b.txt\t\n@@ -0,0 +1 @@\n+x\n";
    assert_eq!(parse_diff(diff).unwrap()["a b.txt"], lines(&[1]));
}

#[test]
fn diff_rejects_a_hunk_whose_lines_exceed_one_side_of_its_header() {
    let short_old = "--- a/o.rs\n+++ b/o.rs\n@@ -1,1 +1,3 @@\n a\n b\n";
    assert!(
        parse_diff(short_old)
            .unwrap_err()
            .contains("more lines than its header")
    );

    let short_new = "--- a/n.rs\n+++ b/n.rs\n@@ -1,2 +1,1 @@\n a\n+b\n";
    assert!(
        parse_diff(short_new)
            .unwrap_err()
            .contains("more lines than its header")
    );
}

// ---- lcov ----

#[test]
fn lcov_paths_become_repo_relative() {
    let root = Path::new("/repo");
    let lcov = "\
SF:/repo/shared/src/a.rs
DA:1,3
DA:2,0
end_of_record
SF:/home/u/.cargo/registry/dep/lib.rs
DA:1,0
end_of_record
SF:src/App.tsx
DA:7,0
end_of_record
SF:./src/x.ts
DA:2,1
end_of_record
";
    let mut hits = LineHits::new();
    parse_lcov(lcov, root, "ui/", &mut hits).unwrap();
    let keys: Vec<&str> = hits.keys().map(String::as_str).collect();
    assert_eq!(keys, ["shared/src/a.rs", "ui/src/App.tsx", "ui/src/x.ts"]);
    assert_eq!(hits["shared/src/a.rs"][&1], 3);
    assert_eq!(hits["shared/src/a.rs"][&2], 0);
    assert_eq!(hits["ui/src/App.tsx"][&7], 0);
}

#[test]
fn lcov_other_record_types_are_ignored() {
    let lcov =
        "TN:\nSF:/r/a.rs\nFN:1,f\nFNDA:0,f\nDA:1,0\nBRDA:1,0,0,-\nLF:1\nLH:0\nend_of_record\n";
    let mut hits = LineHits::new();
    parse_lcov(lcov, Path::new("/r"), "", &mut hits).unwrap();
    assert_eq!(hits["a.rs"].len(), 1);
    assert_eq!(hits["a.rs"][&1], 0);
}

#[test]
fn lcov_relative_paths_at_the_root_and_escaping_ones() {
    let lcov = "SF:./a.rs\nDA:1,0\nend_of_record\nSF:../elsewhere.rs\nDA:1,0\nend_of_record\n";
    let mut hits = LineHits::new();
    parse_lcov(lcov, Path::new("/r"), "", &mut hits).unwrap();
    // A path climbing out of its base is not a repository file.
    assert_eq!(hits.keys().collect::<Vec<_>>(), ["a.rs"]);
}

#[test]
fn lcov_counts_for_the_same_line_add_up() {
    // One source file compiled into two test binaries gets a record from each.
    let lcov = "SF:/r/a.rs\nDA:4,0\nend_of_record\nSF:/r/a.rs\nDA:4,2\nDA:5,0,abc\nend_of_record\n";
    let mut hits = LineHits::new();
    parse_lcov(lcov, Path::new("/r"), "", &mut hits).unwrap();
    assert_eq!(hits["a.rs"][&4], 2);
    assert_eq!(hits["a.rs"][&5], 0);
}

#[test]
fn lcov_a_later_zero_does_not_erase_an_earlier_count() {
    let lcov = "SF:/r/a.rs\nDA:4,2\nend_of_record\nSF:/r/a.rs\nDA:4,0\nend_of_record\n";
    let mut hits = LineHits::new();
    parse_lcov(lcov, Path::new("/r"), "", &mut hits).unwrap();
    assert_eq!(hits["a.rs"][&4], 2);
}

#[test]
fn lcov_a_record_without_lines_still_names_its_file() {
    let lcov = "SF:src/types.ts\nend_of_record\nSF:/elsewhere/x.rs\nend_of_record\n";
    let mut hits = LineHits::new();
    parse_lcov(lcov, Path::new("/r"), "ui/", &mut hits).unwrap();
    assert_eq!(hits.keys().collect::<Vec<_>>(), ["ui/src/types.ts"]);
    assert!(hits["ui/src/types.ts"].is_empty());
}

#[test]
fn lcov_reads_crlf_line_endings() {
    let lcov = "SF:/r/a.rs\r\nDA:1,0\r\nend_of_record\r\n";
    let mut hits = LineHits::new();
    parse_lcov(lcov, Path::new("/r"), "", &mut hits).unwrap();
    assert_eq!(hits["a.rs"][&1], 0);
}

#[test]
fn lcov_rejects_records_it_cannot_read() {
    let mut hits = LineHits::new();
    let e = parse_lcov("DA:1,0\n", Path::new("/r"), "", &mut hits).unwrap_err();
    assert!(e.contains("DA outside a record"), "{e}");
    let e = parse_lcov("SF:/r/a.rs\nDA:x,0\n", Path::new("/r"), "", &mut hits).unwrap_err();
    assert!(e.contains("malformed DA"), "{e}");
    let e = parse_lcov("SF:/r/a.rs\nDA:3\n", Path::new("/r"), "", &mut hits).unwrap_err();
    assert!(e.contains("malformed DA"), "{e}");
    let e = parse_lcov(
        "SF:/r/a.rs\nend_of_record\nDA:1,0\n",
        Path::new("/r"),
        "",
        &mut hits,
    )
    .unwrap_err();
    assert!(e.contains("DA outside a record"), "{e}");
}

// ---- rust ----

fn reach_at(files: &[(&str, &str)], path: &str, needle: &str) -> Option<Reach> {
    let sources: BTreeMap<String, String> = files
        .iter()
        .map(|(p, s)| (p.to_string(), s.to_string()))
        .collect();
    let analysis = analyze(&sources).unwrap();
    let src = &sources[path];
    let line = src
        .lines()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not in {path}")) as u32
        + 1;
    analysis[path].reach(line)
}

fn reach(src: &str, needle: &str) -> Option<Reach> {
    reach_at(&[("a.rs", src)], "a.rs", needle)
}

#[test]
fn rust_runtime_only_functions() {
    let src = r#"
use spin_sdk::http::{EmptyBody, FullBody, Request, Response, StatusCode};
use spin_sdk::http_service;
use spin_sdk::pg::Connection;
use spin_sdk::variables;

#[http_service]
async fn handle() -> u16 {
    1 // in handle
}
async fn takes_request(req: &Request) -> u16 {
    2 // in takes_request
}
async fn takes_conn(
    conn: &Connection, // param line
) -> u16 {
    3
}
async fn opens() {
    let _ = Connection::open("x").await; // in opens
}
async fn reads_var() {
    let _ = variables::get("k").await; // in reads_var
}
async fn sends() {
    let _ = spin_sdk::http::send(()).await; // in sends
}
fn status_only() -> StatusCode {
    StatusCode::OK // in status_only
}
fn builds_request() -> Request<EmptyBody> {
    Request::new(EmptyBody::new()) // in builds_request
}
fn names_empty_body(req: &Request<EmptyBody>) -> u16 {
    4 // in names_empty_body
}
fn names_full_body(resp: &Response<FullBody<Vec<u8>>>) -> u16 {
    5 // in names_full_body
}
fn runtime_all() -> Option<Vec<u8>> {
    spin_sdk::variables::get_all() // in runtime_all
}
"#;
    for needle in [
        "in handle",
        "in takes_request",
        "param line",
        "in opens",
        "in reads_var",
        "in sends",
    ] {
        assert_eq!(reach(src, needle), Some(Reach::E2eOnly), "{needle}");
    }
    // Types a unit test can build do not make a function runtime-only.
    for needle in [
        "in status_only",
        "in builds_request",
        "in names_empty_body",
        "in names_full_body",
        "in runtime_all",
    ] {
        assert_eq!(reach(src, needle), Some(Reach::Unit), "{needle}");
    }
}

#[test]
fn rust_incoming_http_values_make_a_function_runtime_only() {
    let src = r#"
use spin_sdk::http::{IncomingRequestBody, IncomingResponseBody, Request, Response};

fn header_string(resp: &Response, name: &str) -> Option<String> {
    None // default response
}
fn explicit_response(resp: Response<IncomingResponseBody>) {
    let _ = resp; // explicit response
}
fn explicit_request(req: Request<IncomingRequestBody>) {
    let _ = req; // explicit request
}
fn nested(items: Vec<Response>) {
    let _ = items; // nested response
}
"#;
    for needle in [
        "default response",
        "explicit response",
        "explicit request",
        "nested response",
    ] {
        assert_eq!(reach(src, needle), Some(Reach::E2eOnly), "{needle}");
    }
}

#[test]
fn rust_signature_lines_belong_to_the_function() {
    let src = "
#[spin_sdk::http_service]
async fn handle(
    a: u8,
) -> u8 {
    a
}
";
    assert_eq!(reach(src, "async fn handle("), Some(Reach::E2eOnly));
    assert_eq!(reach(src, "a: u8,"), Some(Reach::E2eOnly));
    assert_eq!(
        reach(src, "#[spin_sdk::http_service]"),
        Some(Reach::E2eOnly)
    );
}

#[test]
fn rust_imports_resolve_renames_self_and_only_known_glob_names() {
    let src = r#"
use spin_sdk::pg::{self as db};
use spin_sdk::variables::{self};
mod m {
    use spin_sdk::pg::*;
    pub fn uses_glob(c: &Connection) {
        let _ = c; // glob connection
    }
    pub fn plain() -> Result<u8, ()> {
        Ok(1) // glob ok
    }
}
fn renamed(c: &db::Connection) {
    let _ = c; // renamed module
}
fn self_import() {
    let _ = variables::get("k"); // self import
}
"#;
    assert_eq!(reach(src, "glob connection"), Some(Reach::E2eOnly));
    assert_eq!(reach(src, "glob ok"), Some(Reach::Unit));
    assert_eq!(reach(src, "renamed module"), Some(Reach::E2eOnly));
    assert_eq!(reach(src, "self import"), Some(Reach::E2eOnly));
}

#[test]
fn rust_callers_of_e2e_only_functions_are_e2e_only_across_files() {
    let shared = r#"
pub async fn open_db() -> u8 {
    let _ = spin_sdk::variables::get("db").await;
    0
}
pub fn pure(x: u8) -> u8 {
    x + 1
}
"#;
    let server = r#"
async fn list() -> u8 {
    let n = home_rss_shared::db::open_db().await; // direct caller
    n
}
async fn outer() -> u8 {
    list().await // caller of caller
}
fn as_value(v: Vec<u8>) {
    let _ = v.into_iter().map(open_db); // fn value
}
fn in_macro() {
    println!("{}", list_len(open_db)); // macro arg
}
fn list_len<T>(_: T) -> usize {
    0
}
fn only_pure() -> u8 {
    pure(1) // pure caller
}
"#;
    let files = [("shared/src/db.rs", shared), ("server/src/lib.rs", server)];
    for needle in ["direct caller", "caller of caller", "fn value", "macro arg"] {
        assert_eq!(
            reach_at(&files, "server/src/lib.rs", needle),
            Some(Reach::E2eOnly),
            "{needle}"
        );
    }
    assert_eq!(
        reach_at(&files, "server/src/lib.rs", "pure caller"),
        Some(Reach::Unit)
    );
    assert_eq!(
        reach_at(&files, "shared/src/db.rs", "x + 1"),
        Some(Reach::Unit)
    );
}

#[test]
fn rust_propagation_reaches_a_fixed_point_whatever_the_declaration_order() {
    let src = r#"
fn top() {
    outer(); // top
}
fn outer() {
    list(); // outer
}
fn list() {
    runtime(); // list
}
async fn runtime() {
    let _ = spin_sdk::variables::get("k").await; // runtime
}
"#;
    for needle in ["// top", "// outer", "// list", "// runtime"] {
        assert_eq!(reach(src, needle), Some(Reach::E2eOnly), "{needle}");
    }

    // Files are read in path order, so the caller comes before its callee at
    // every step.
    let files = [
        ("a.rs", "fn top() {\n    mid(); // top\n}\n"),
        ("b.rs", "fn mid() {\n    leaf(); // mid\n}\n"),
        (
            "c.rs",
            "async fn leaf() {\n    let _ = spin_sdk::variables::get(\"k\").await; // leaf\n}\n",
        ),
    ];
    for (path, needle) in [("a.rs", "// top"), ("b.rs", "// mid"), ("c.rs", "// leaf")] {
        assert_eq!(
            reach_at(&files, path, needle),
            Some(Reach::E2eOnly),
            "{path}"
        );
    }
}

#[test]
fn rust_an_ambiguous_name_propagates_only_when_every_candidate_is_e2e_only() {
    let a = r#"
pub async fn load() { let _ = spin_sdk::variables::get("k").await; }
"#;
    let b = r#"
pub fn load() {}
fn caller() {
    load(); // ambiguous
}
"#;
    let files = [("a.rs", a), ("b.rs", b)];
    assert_eq!(reach_at(&files, "b.rs", "ambiguous"), Some(Reach::Unit));
}

#[test]
fn rust_method_calls_propagate_only_through_self() {
    let src = r#"
struct S;
impl S {
    async fn fetch(&self) { let _ = spin_sdk::http::send(()).await; }
    async fn via_self(&self) {
        self.fetch().await; // self call
    }
}
async fn get() { let _ = spin_sdk::variables::get("k").await; }
fn other(m: std::collections::HashMap<u8, u8>, s: S) {
    let _ = m.get(&1); // map get
    let _ = s.fetch(); // receiver not self
}
"#;
    assert_eq!(reach(src, "self call"), Some(Reach::E2eOnly));
    assert_eq!(reach(src, "map get"), Some(Reach::Unit));
    assert_eq!(reach(src, "receiver not self"), Some(Reach::Unit));
}

#[test]
fn rust_self_calls_skip_names_that_are_not_uniquely_methods() {
    let src = r#"
struct A;
struct B;
impl A {
    async fn fetch(&self) { let _ = spin_sdk::http::send(()).await; }
    async fn via_free_name(&self) {
        self.fetch().await; // free name
    }
    async fn run(&self) { let _ = spin_sdk::http::send(()).await; }
}
impl B {
    fn run(&self) {}
    fn via_split(&self) {
        self.run(); // split name
    }
}
fn fetch() {}
"#;
    assert_eq!(reach(src, "free name"), Some(Reach::Unit));
    assert_eq!(reach(src, "split name"), Some(Reach::Unit));
}

#[test]
fn rust_self_calls_need_the_name_to_be_a_method() {
    let src = r#"
struct S;
async fn load() { let _ = spin_sdk::variables::get("k").await; }
impl S {
    fn x(&self) {
        self.load(); // method only
    }
}
"#;
    assert_eq!(reach(src, "method only"), Some(Reach::Unit));
}

#[test]
fn rust_incoming_types_through_a_glob_import_and_with_several_arguments() {
    let src = r#"
use spin_sdk::http::*;

fn f(r: Request) {
    let _ = r; // glob request
}
fn g(r: &Response) {
    let _ = r; // glob response
}
fn h(r: Request<A, IncomingRequestBody>) {
    let _ = r; // two arguments
}
"#;
    assert_eq!(reach(src, "glob request"), Some(Reach::E2eOnly));
    assert_eq!(reach(src, "glob response"), Some(Reach::E2eOnly));
    assert_eq!(reach(src, "two arguments"), Some(Reach::Unit));
}

#[test]
fn rust_a_name_the_function_binds_is_not_a_call() {
    let src = r#"
async fn load() { let _ = spin_sdk::variables::get("k").await; }
fn param(load: u8) -> u8 {
    load // param
}
fn local() -> u8 {
    let load = 1;
    load // local
}
fn closure() -> u8 {
    let f = |load: u8| load; // closure
    f(1)
}
fn matched(o: Option<u8>) -> u8 {
    match o {
        Some(load) => load, // matched
        None => 0,
    }
}
fn calls() {
    load(); // real call
}
"#;
    for needle in ["// param", "// local", "// closure", "// matched"] {
        assert_eq!(reach(src, needle), Some(Reach::Unit), "{needle}");
    }
    assert_eq!(reach(src, "real call"), Some(Reach::E2eOnly));
}

#[test]
fn rust_trait_default_bodies_and_nested_functions() {
    let src = r#"
trait Store {
    fn save(&self) {
        let _ = spin_sdk::variables::get("x"); // trait default
    }
}
fn outer() {
    fn inner() {
        let _ = spin_sdk::variables::get("y"); // nested use
    }
    inner(); // outer body
}
"#;
    assert_eq!(reach(src, "trait default"), Some(Reach::E2eOnly));
    // A nested function belongs to its outer one.
    assert_eq!(reach(src, "nested use"), Some(Reach::E2eOnly));
    assert_eq!(reach(src, "outer body"), Some(Reach::E2eOnly));
    assert_eq!(reach(src, "trait Store"), None);
}

#[test]
fn rust_test_code_is_marked() {
    let src = r#"
fn prod() {}
#[cfg(test)]
mod tests {
    #[test]
    fn t() {} // in cfg test
}
#[test]
fn top() {} // test fn
#[tokio::test]
async fn runner() {} // runner test
#[cfg(not(test))]
fn not_test() {} // cfg not test
#[cfg(all(test, feature = "x"))]
fn all_test() {} // cfg all test
"#;
    let sources = BTreeMap::from([("a.rs".to_string(), src.to_string())]);
    let a = &analyze(&sources).unwrap()["a.rs"];
    let line = |needle: &str| src.lines().position(|l| l.contains(needle)).unwrap() as u32 + 1;
    for needle in ["in cfg test", "test fn", "runner test", "cfg all test"] {
        assert!(a.is_test_line(line(needle)), "{needle}");
    }
    assert!(!a.is_test_line(line("cfg not test")));
    assert!(!a.is_test_line(line("fn prod")));
}

#[test]
fn rust_test_code_in_impls_and_traits() {
    let src = r#"
struct S;
#[cfg(test)]
impl S {
    fn helper() {} // test impl
}
impl S {
    const K: u8 = 1;
    #[test]
    fn method_test() {} // test method
    fn prod(&self) {} // prod method
}
#[cfg(test)]
trait T {
    fn d() {} // test trait
}
trait R {
    fn required(&self); // required method
}
"#;
    let sources = BTreeMap::from([("a.rs".to_string(), src.to_string())]);
    let a = &analyze(&sources).unwrap()["a.rs"];
    let line = |needle: &str| src.lines().position(|l| l.contains(needle)).unwrap() as u32 + 1;
    for needle in ["test impl", "test method", "test trait"] {
        assert!(a.is_test_line(line(needle)), "{needle}");
    }
    assert!(!a.is_test_line(line("prod method")));
    assert_eq!(a.reach(line("prod method")), Some(Reach::Unit));
    // A method without a body has no lines to run.
    assert_eq!(a.reach(line("required method")), None);
}

#[test]
fn rust_macros_whose_body_is_not_an_expression_list_are_skipped() {
    let src = r#"
async fn open_db() { let _ = spin_sdk::variables::get("k").await; }
fn repeat() -> Vec<u8> {
    vec![0; 3] // repeat form
}
"#;
    assert_eq!(reach(src, "repeat form"), Some(Reach::Unit));
}

#[test]
fn rust_a_file_that_does_not_parse_is_an_error() {
    let sources = BTreeMap::from([("bad.rs".to_string(), "fn (".to_string())]);
    let e = analyze(&sources).unwrap_err();
    assert!(e.starts_with("bad.rs:1:"), "{e}");
}

// ---- report ----

fn analysis(
    files: &[(&str, &str)],
) -> BTreeMap<String, home_rss_repo_checks::diff_coverage::rust::FileAnalysis> {
    analyze(
        &files
            .iter()
            .map(|(p, s)| (p.to_string(), s.to_string()))
            .collect(),
    )
    .unwrap()
}

fn hits(entries: &[(&str, &[(u32, u64)])]) -> LineHits {
    entries
        .iter()
        .map(|(p, l)| (p.to_string(), l.iter().copied().collect()))
        .collect()
}

fn added(entries: &[(&str, &[u32])]) -> BTreeMap<String, BTreeSet<u32>> {
    entries
        .iter()
        .map(|(p, l)| (p.to_string(), lines(l)))
        .collect()
}

fn finding(path: &str, line: Option<u32>, kind: Kind) -> Finding {
    Finding {
        path: path.to_string(),
        line,
        kind,
    }
}

const LIB: &str = "\
fn unit() -> u8 {
    1
}
async fn e2e() {
    let _ = spin_sdk::variables::get(\"k\").await;
}
const C: u8 = 1;
#[cfg(test)]
mod tests {
    fn helper() {}
}
";

#[test]
fn report_rust_lines() {
    let a = analysis(&[("shared/src/lib.rs", LIB)]);
    let h = hits(&[(
        "shared/src/lib.rs",
        &[(1, 0), (2, 0), (5, 0), (6, 3), (10, 0)],
    )]);
    let d = added(&[("shared/src/lib.rs", &[1, 2, 3, 5, 6, 7, 10])]);
    assert_eq!(
        report(&d, &h, &a).unwrap(),
        vec![
            finding("shared/src/lib.rs", Some(1), Kind::Untested),
            finding("shared/src/lib.rs", Some(2), Kind::Untested),
            finding("shared/src/lib.rs", Some(5), Kind::E2eOnly),
        ]
    );
}

#[test]
fn report_rust_file_missing_from_coverage() {
    let a = analysis(&[
        ("shared/src/lib.rs", LIB),
        ("shared/src/m.rs", "pub mod x;\nconst A: u8 = 1;\n"),
    ]);
    let h = LineHits::new();
    let d = added(&[("shared/src/lib.rs", &[2]), ("shared/src/m.rs", &[1, 2])]);
    // m.rs has no function among the changed lines, so no record is expected.
    assert_eq!(
        report(&d, &h, &a).unwrap(),
        vec![finding("shared/src/lib.rs", None, Kind::NoCoverageData)]
    );
    // Only test lines changed: nothing to say about coverage either.
    let d = added(&[("shared/src/lib.rs", &[10])]);
    assert!(report(&d, &h, &a).unwrap().is_empty());
}

#[test]
fn report_skips_paths_that_are_not_production_rust() {
    let h = hits(&[("target/gen.rs", &[(1, 0)]), (".hidden/h.rs", &[(1, 0)])]);
    let d = added(&[
        ("target/gen.rs", &[1]),
        (".hidden/h.rs", &[1]),
        ("ui/node_modules/x/y.rs", &[1]),
    ]);
    assert!(report(&d, &h, &BTreeMap::new()).unwrap().is_empty());
}

#[test]
fn report_fails_on_a_rust_source_the_analysis_did_not_read() {
    let d = added(&[("shared/src/new.rs", &[1])]);
    let e = report(&d, &LineHits::new(), &BTreeMap::new()).unwrap_err();
    assert!(e.contains("shared/src/new.rs"), "{e}");
}

#[test]
fn report_reads_rust_files_at_the_repository_root() {
    let a = analysis(&[("build.rs", "fn main() {\n    1;\n}\n")]);
    let h = hits(&[("build.rs", &[(2, 0)])]);
    let d = added(&[("build.rs", &[2])]);
    assert_eq!(
        report(&d, &h, &a).unwrap(),
        vec![finding("build.rs", Some(2), Kind::Untested)]
    );
    let e = report(&d, &h, &BTreeMap::new()).unwrap_err();
    assert!(e.contains("build.rs"), "{e}");
}

#[test]
fn report_skips_test_directories_and_e2e() {
    let a = analysis(&[
        ("shared/tests/t.rs", "fn t() {}\n"),
        ("e2e/tests/api.rs", "fn t() {}\n"),
    ]);
    let h = hits(&[
        ("shared/tests/t.rs", &[(1, 0)]),
        ("e2e/tests/api.rs", &[(1, 0)]),
    ]);
    let d = added(&[
        ("shared/tests/t.rs", &[1]),
        ("e2e/tests/api.rs", &[1]),
        ("docs/spec.md", &[1]),
    ]);
    assert!(report(&d, &h, &a).unwrap().is_empty());
}

#[test]
fn report_ui_lines() {
    let a = BTreeMap::new();
    let h = hits(&[("ui/src/App.tsx", &[(3, 0), (4, 1)])]);
    let d = added(&[
        ("ui/src/App.tsx", &[3, 4, 5]),
        ("ui/src/api.ts", &[1]),
        ("ui/src/App.test.tsx", &[1]),
        ("ui/src/vite-env.d.ts", &[1]),
        ("ui/vite.config.ts", &[1]),
        ("ui/src/test-setup.ts", &[1]),
    ]);
    assert_eq!(
        report(&d, &h, &a).unwrap(),
        vec![
            finding("ui/src/App.tsx", Some(3), Kind::Untested),
            finding("ui/src/api.ts", None, Kind::NoCoverageData),
        ]
    );
}

#[test]
fn report_ui_file_with_a_record_but_no_instrumented_line_is_covered() {
    let h = hits(&[("ui/src/types.ts", &[])]);
    let d = added(&[("ui/src/types.ts", &[1, 2])]);
    assert!(report(&d, &h, &BTreeMap::new()).unwrap().is_empty());
}

#[test]
fn findings_print_as_documented() {
    assert_eq!(
        finding("a.rs", Some(3), Kind::Untested).to_string(),
        "a.rs:3\tuntested"
    );
    assert_eq!(
        finding("a.rs", Some(4), Kind::E2eOnly).to_string(),
        "a.rs:4\te2e-only"
    );
    assert_eq!(
        finding("a.rs", None, Kind::NoCoverageData).to_string(),
        "a.rs\tno-coverage-data"
    );
}

// ---- run / command ----

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("diff-coverage-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Fixture {
            dir: dir.canonicalize().unwrap(),
        }
    }

    fn write(&self, rel: &str, body: &str) -> PathBuf {
        let p = self.dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        p
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn repo_fixture(name: &str) -> Fixture {
    let f = Fixture::new(name);
    f.write("repo/shared/src/lib.rs", LIB);
    f.write("repo/target/junk.rs", "this is not rust");
    f.write("repo/ui/src/App.tsx", "a\nb\n");
    f.write(
        "diff",
        "--- a/shared/src/lib.rs\n+++ b/shared/src/lib.rs\n@@ -0,0 +1,6 @@\n+a\n+b\n+c\n+d\n+e\n+f\n\
         --- a/ui/src/App.tsx\n+++ b/ui/src/App.tsx\n@@ -1 +1,2 @@\n-x\n+a\n+b\n",
    );
    let root = f.dir.join("repo");
    f.write(
        "rust.lcov",
        &format!(
            "SF:{}/shared/src/lib.rs\nDA:2,0\nDA:5,0\nDA:6,1\nend_of_record\n",
            root.display()
        ),
    );
    f.write("ui.lcov", "SF:src/App.tsx\nDA:2,0\nend_of_record\n");
    f
}

#[test]
fn run_reads_everything_from_disk() {
    let f = repo_fixture("run");
    let got = run(
        &f.dir.join("repo"),
        &f.dir.join("diff"),
        &f.dir.join("rust.lcov"),
        &f.dir.join("ui.lcov"),
    )
    .unwrap();
    assert_eq!(
        got,
        vec![
            finding("shared/src/lib.rs", Some(2), Kind::Untested),
            finding("shared/src/lib.rs", Some(5), Kind::E2eOnly),
            finding("ui/src/App.tsx", Some(2), Kind::Untested),
        ]
    );
}

#[test]
fn run_fails_on_unreadable_inputs_instead_of_reporting_nothing() {
    let f = repo_fixture("missing");
    let e = run(
        &f.dir.join("repo"),
        &f.dir.join("diff"),
        &f.dir.join("no-such.lcov"),
        &f.dir.join("ui.lcov"),
    )
    .unwrap_err();
    assert!(e.contains("no-such.lcov"), "{e}");

    f.write("repo/shared/src/broken.rs", "fn (");
    let e = run(
        &f.dir.join("repo"),
        &f.dir.join("diff"),
        &f.dir.join("rust.lcov"),
        &f.dir.join("ui.lcov"),
    )
    .unwrap_err();
    assert!(e.contains("shared/src/broken.rs"), "{e}");
}

#[test]
fn run_does_not_read_directories_that_hold_no_production_rust() {
    let f = repo_fixture("skipped");
    for rel in [
        "repo/e2e/x.rs",
        "repo/.hidden/x.rs",
        "repo/node_modules/x.rs",
        "repo/shared/benches/b.rs",
        "repo/shared/examples/e.rs",
        "repo/shared/tests/t.rs",
    ] {
        f.write(rel, "fn (");
    }
    let got = run(
        &f.dir.join("repo"),
        &f.dir.join("diff"),
        &f.dir.join("rust.lcov"),
        &f.dir.join("ui.lcov"),
    );
    assert!(got.is_ok(), "{got:?}");
}

fn command(args: &[&Path]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_diff-coverage"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn command_exit_status_tells_findings_from_none_and_errors() {
    let f = repo_fixture("cmd");
    let repo = f.dir.join("repo");
    let out = command(&[
        &repo,
        &f.dir.join("diff"),
        &f.dir.join("rust.lcov"),
        &f.dir.join("ui.lcov"),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "shared/src/lib.rs:2\tuntested\nshared/src/lib.rs:5\te2e-only\nui/src/App.tsx:2\tuntested\n"
    );

    let empty = f.write("empty.diff", "");
    let out = command(&[
        &repo,
        &empty,
        &f.dir.join("rust.lcov"),
        &f.dir.join("ui.lcov"),
    ]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());

    let out = command(&[
        &repo,
        &f.dir.join("nope"),
        &f.dir.join("rust.lcov"),
        &f.dir.join("ui.lcov"),
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8(out.stderr).unwrap().contains("nope"));

    let out = command(&[&repo]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8(out.stderr).unwrap().starts_with("usage:"));
}
