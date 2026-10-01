use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(300000);

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
            "home-rss-diff-coverage-fix-{}-{id}",
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
fn fix_bodyless_fn_followed_by_default_method_stays_separate() {
    // Catches the bodyless-`fn` guard being lost: without it the declarations
    // leak past the trait's closing brace, and the added function's header
    // line is judged by a leaked declaration range instead of its own body.
    let base = "trait T {\n    fn decl();\n    fn added();\n}\n";
    let changed = "trait T {\n    fn decl();\n    fn added();\n}\nfn real_fn() {\n    spin_sdk::pg::Connection::open(\"x\").unwrap();\n}\n";
    let repo = FixtureRepo::new(&[("src/decl3.rs", base)]);
    repo.commit_change(&[("src/decl3.rs", changed)]);
    let rust_lcov = repo.write_lcov(
        "rust.lcov",
        &lcov_record("src/decl3.rs", &[(5, 0), (6, 0), (7, 0)]),
    );
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let mut lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    lines.sort();
    assert_eq!(
        lines,
        vec![
            ("src/decl3.rs:5".to_string(), "e2e-only".to_string()),
            ("src/decl3.rs:6".to_string(), "e2e-only".to_string()),
            ("src/decl3.rs:7".to_string(), "e2e-only".to_string()),
        ]
    );
}

#[test]
fn fix_changed_line_inside_nested_fn_uses_inner_range() {
    // Catches the innermost-range choice being lost: without it a changed line
    // inside the nested fn is judged by the outer body and misses the Spin use.
    let base = "pub fn outer() {\n    fn helper() {\n        spin_sdk::pg::Connection::open(\"x\").unwrap();\n    }\n}\n";
    let changed = "pub fn outer() {\n    fn helper() {\n        spin_sdk::pg::Connection::open(\"x\").unwrap();\n        let touched_inner = 1;\n    }\n}\n";
    let repo = FixtureRepo::new(&[("src/nested2.rs", base)]);
    repo.commit_change(&[("src/nested2.rs", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/nested2.rs", &[(4, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/nested2.rs:4".to_string(), "e2e-only".to_string())]
    );
}

#[test]
fn fix_empty_paren_uri_query_stays_untested() {
    // Catches the pg-query guard being lost: the http Uri accessor takes no
    // arguments, so it must not count as a pg query.
    let base = "pub fn path_of(uri: &Uri) -> String {\n    uri.query().unwrap_or_default().to_owned()\n}\n";
    let changed = "pub fn path_of(uri: &Uri) -> String {\n    let touched = 1;\n    uri.query().unwrap_or_default().to_owned()\n}\n";
    let repo = FixtureRepo::new(&[("src/uriq.rs", base)]);
    repo.commit_change(&[("src/uriq.rs", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/uriq.rs", &[(2, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/uriq.rs:2".to_string(), "untested".to_string())]
    );
}

#[test]
fn fix_real_tool_command_lines_are_pinned() {
    // Catches the coverage command lines being weakened beyond the four
    // lcov-output flags: each command must keep selecting the host target,
    // the workspace, the vitest provider, and actually running.
    let body = std::fs::read_to_string(script_path()).unwrap();
    assert!(body.contains("--workspace"), "rust workspace flag");
    assert!(body.contains("--target"), "rust host target flag");
    assert!(
        body.contains("--coverage.provider=v8"),
        "vitest coverage provider"
    );
    assert!(body.contains("vitest run"), "vitest run invocation");
    assert!(body.contains("cargo llvm-cov"), "rust coverage invocation");
}
