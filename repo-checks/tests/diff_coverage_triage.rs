use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(200000);

// Same seam as the existing diff-coverage tests: the script reads coverage
// from these LCOV files instead of running the real tools.
const RUST_LCOV_ENV: &str = "DIFF_COVERAGE_RUST_LCOV";
const UI_LCOV_ENV: &str = "DIFF_COVERAGE_UI_LCOV";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn script_path() -> PathBuf {
    repo_root().join("scripts/diff-coverage.sh")
}

struct FixtureRepo {
    dir: PathBuf,
    base: String,
}

impl FixtureRepo {
    fn new(base_files: &[(&str, &str)]) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "home-rss-diff-coverage-triage-{}-{id}",
            std::process::id()
        ));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        let mut repo = Self {
            dir,
            base: String::new(),
        };
        repo.git(&["init", "-q"]);
        repo.git(&["config", "user.email", "diff-coverage-test@example.com"]);
        repo.git(&["config", "user.name", "diff-coverage-test"]);
        repo.write_all(base_files);
        repo.git(&["add", "-A"]);
        repo.git(&["commit", "-qm", "base"]);
        let base = repo.git_stdout(&["rev-parse", "HEAD"]);
        repo.base = base.trim().to_string();
        repo
    }

    fn git(&self, args: &[&str]) -> Output {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn git_stdout(&self, args: &[&str]) -> String {
        String::from_utf8(self.git(args).stdout).unwrap()
    }

    fn write_all(&self, files: &[(&str, &str)]) {
        for (rel, content) in files {
            let path = self.dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, content).unwrap();
        }
    }

    fn commit_change(&self, files: &[(&str, &str)]) {
        self.write_all(files);
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", "change"]);
    }

    fn write_lcov(&self, name: &str, body: &str) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    fn run(&self, rust_lcov: &Path, ui_lcov: &Path) -> Output {
        Command::new("bash")
            .arg(script_path())
            .arg(&self.base)
            .current_dir(&self.dir)
            .env(RUST_LCOV_ENV, rust_lcov)
            .env(UI_LCOV_ENV, ui_lcov)
            .output()
            .unwrap()
    }
}

impl Drop for FixtureRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn lcov_record(sf: &str, hits: &[(u32, u32)]) -> String {
    let mut body = format!("TN:\nSF:{sf}\n");
    for (line, hit) in hits {
        body.push_str(&format!("DA:{line},{hit}\n"));
    }
    body.push_str("end_of_record\n");
    body
}

// Every output line must be `path:line<TAB>untested|e2e-only`; malformed lines
// fail the test instead of slipping through the assertions.
fn report_lines(output: &Output) -> Vec<(String, String)> {
    assert!(
        output.status.success(),
        "diff-coverage failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout.clone()).unwrap();
    let mut lines = Vec::new();
    for raw in stdout.lines() {
        if raw.is_empty() {
            continue;
        }
        let (loc, status) = raw
            .split_once('\t')
            .unwrap_or_else(|| panic!("output line without TAB: {raw:?}"));
        assert!(
            status == "untested" || status == "e2e-only",
            "unknown status in {raw:?}"
        );
        let (_, num) = loc
            .rsplit_once(':')
            .unwrap_or_else(|| panic!("location without :line in {raw:?}"));
        num.parse::<u32>()
            .unwrap_or_else(|_| panic!("bad line number in {raw:?}"));
        lines.push((loc.to_string(), status.to_string()));
    }
    lines
}

#[test]
fn triage1_covered_ui_line_with_vitest_relative_sf_is_omitted() {
    // Catches the UI coverage lookup missing vitest's ui-relative SF paths:
    // vitest runs with cwd ui/, so SF is `src/api.ts` while git diff reports
    // `ui/src/api.ts`. A covered UI line must not be reported.
    let repo = FixtureRepo::new(&[("ui/src/api.ts", "export const a = 1;\n")]);
    repo.commit_change(&[(
        "ui/src/api.ts",
        "export const a = 1;\nexport const b = 2;\n",
    )]);
    let rust_lcov = repo.write_lcov("rust.lcov", "TN:\n");
    let ui_lcov = repo.write_lcov("ui.lcov", &lcov_record("src/api.ts", &[(1, 1), (2, 3)]));
    assert!(report_lines(&repo.run(&rust_lcov, &ui_lcov)).is_empty());
}

#[test]
fn triage2_pg_query_users_are_e2e_only() {
    // Catches the e2e-only rule missing spin_sdk::pg queries: connect() lives
    // in another function, so only `.query()`/`.execute()` name the Spin API.
    let base = "pub fn list() {\n    let conn = db::connect().await;\n    let rows = conn.query(\"SELECT 1\", vec![]).await;\n}\n\npub fn clear() {\n    let conn = db::connect().await;\n    conn.execute(\"DELETE FROM t\", vec![]).await;\n}\n";
    let changed = "pub fn list() {\n    let conn = db::connect().await;\n    let touched_q = 1;\n    let rows = conn.query(\"SELECT 1\", vec![]).await;\n}\n\npub fn clear() {\n    let conn = db::connect().await;\n    let touched_x = 1;\n    conn.execute(\"DELETE FROM t\", vec![]).await;\n}\n";
    let repo = FixtureRepo::new(&[("src/q.rs", base)]);
    repo.commit_change(&[("src/q.rs", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/q.rs", &[(3, 0), (9, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let mut lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    lines.sort();
    assert_eq!(
        lines,
        vec![
            ("src/q.rs:3".to_string(), "e2e-only".to_string()),
            ("src/q.rs:9".to_string(), "e2e-only".to_string()),
        ]
    );
}

#[test]
fn triage3_spin_names_in_comments_and_strings_stay_untested() {
    // Catches comment/string text promoting a plain function to e2e-only:
    // neither the comment nor the string literal calls a Spin API.
    let base = "pub fn plain_helper(x: i32) -> i32 {\n    x + 1\n}\n";
    let changed = "pub fn plain_helper(x: i32) -> i32 {\n    // we used to call spin_sdk::pg::Connection::open here but removed it\n    let label = \"spin_sdk::http::send is not used\";\n    x + 1\n}\n";
    let repo = FixtureRepo::new(&[("src/plain.rs", base)]);
    repo.commit_change(&[("src/plain.rs", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/plain.rs", &[(2, 0), (3, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let mut lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    lines.sort();
    assert_eq!(
        lines,
        vec![
            ("src/plain.rs:2".to_string(), "untested".to_string()),
            ("src/plain.rs:3".to_string(), "untested".to_string()),
        ]
    );
}

#[test]
fn triage4_bodyless_fn_does_not_reach_later_braces() {
    // Catches a bodyless `fn` declaration swallowing later lines: the added
    // top-level comment is outside every function, so it must stay untested
    // even though a later `fn` opens a brace.
    let base = "trait T {\n    fn decl();\n}\n\nfn real_fn() {\n    other();\n}\n";
    let changed = "trait T {\n    fn decl();\n}\n// note spin_sdk::pg::Connection::open\nfn real_fn() {\n    other();\n}\n";
    let repo = FixtureRepo::new(&[("src/decl2.rs", base)]);
    repo.commit_change(&[("src/decl2.rs", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/decl2.rs", &[(4, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/decl2.rs:4".to_string(), "untested".to_string())]
    );
}

#[test]
fn triage5_outer_fn_is_not_e2e_only_for_nested_fn_spin_use() {
    // Catches a nested `fn`'s Spin API use leaking into the outer function:
    // the outer body never calls the API directly.
    let base = "pub fn outer() {\n    fn helper() {\n        spin_sdk::pg::Connection::open(\"x\").unwrap();\n    }\n}\n";
    let changed = "pub fn outer() {\n    let touched = 1;\n    fn helper() {\n        spin_sdk::pg::Connection::open(\"x\").unwrap();\n    }\n}\n";
    let repo = FixtureRepo::new(&[("src/nested.rs", base)]);
    repo.commit_change(&[("src/nested.rs", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/nested.rs", &[(2, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/nested.rs:2".to_string(), "untested".to_string())]
    );
}

#[test]
fn triage6_real_tool_invocations_are_pinned() {
    // Catches the real coverage command lines being weakened: breaking any of
    // these flags must fail this test (verified with a mutation patch).
    let body = std::fs::read_to_string(script_path()).unwrap();
    assert!(body.contains("--lcov"), "rust lcov output flag");
    assert!(body.contains("--output-path"), "rust lcov output path");
    assert!(
        body.contains("--coverage.reporter=lcov"),
        "vitest lcov reporter"
    );
    assert!(
        body.contains("--coverage.reportsDirectory"),
        "vitest reports directory"
    );
}

#[test]
fn triage7_duplicate_da_entries_keep_max_hits() {
    // Catches max-merge degrading to plain overwrite: the first DA hit must
    // survive a later zero-count entry for the same line (verified with a
    // mutation patch).
    let repo = FixtureRepo::new(&[("src/lib.rs", "pub fn present() {}\n")]);
    repo.commit_change(&[("src/lib.rs", "pub fn present() {}\npub fn added() {}\n")]);
    let body = "TN:\nSF:src/lib.rs\nDA:2,3\nDA:2,0\nend_of_record\n";
    let rust_lcov = repo.write_lcov("rust.lcov", body);
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    assert!(report_lines(&repo.run(&rust_lcov, &ui_lcov)).is_empty());
}
