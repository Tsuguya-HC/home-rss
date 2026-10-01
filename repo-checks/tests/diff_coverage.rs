use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

// Fixture seam the script must honour: when these point at LCOV files, the
// script reads coverage from them instead of running the real tools, so the
// tests stay hermetic (no llvm-cov, vitest, or network).
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
            "home-rss-diff-coverage-{}-{id}",
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
fn usage_names_base_ref() {
    // Catches the script succeeding silently, or failing without naming its argument.
    let output = Command::new("bash")
        .arg(script_path())
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(!output.status.success());
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

#[test]
fn reports_uncovered_rust_change_as_untested() {
    // Catches the script dropping an uncovered Rust line, or labelling it e2e-only.
    let repo = FixtureRepo::new(&[("src/lib.rs", "pub fn present() {}\n")]);
    repo.commit_change(&[("src/lib.rs", "pub fn present() {}\npub fn added() {}\n")]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/lib.rs", &[(1, 1), (2, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/lib.rs:2".to_string(), "untested".to_string())]
    );
}

#[test]
fn omits_covered_rust_change() {
    // Catches the script listing a changed line that unit tests do execute.
    let repo = FixtureRepo::new(&[("src/lib.rs", "pub fn present() {}\n")]);
    repo.commit_change(&[("src/lib.rs", "pub fn present() {}\npub fn added() {}\n")]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/lib.rs", &[(1, 1), (2, 4)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    assert!(report_lines(&repo.run(&rust_lcov, &ui_lcov)).is_empty());
}

#[test]
fn marks_spin_runtime_users_e2e_only() {
    // Catches each of the four Spin-only patterns falling back to untested,
    // and a plain function being promoted to e2e-only. Spellings match the
    // tree (`Connection::open`, `variables::get`, `http::send`, `http_service`).
    let base = "pub fn load_pg() {\n    let _c = spin_sdk::pg::Connection::open(\"postgres://x\").unwrap();\n}\n\npub fn read_var() {\n    let _v = spin_sdk::variables::get(\"db_url\").unwrap();\n}\n\npub fn fetch_ext() {\n    let _r = spin_sdk::http::send(\"https://example.invalid\").await;\n}\n\n#[spin_sdk::http_service]\npub fn handle() {\n    route();\n}\n\npub fn plain() {\n}\n";
    let changed = "pub fn load_pg() {\n    let _c = spin_sdk::pg::Connection::open(\"postgres://x\").unwrap();\n    let touched_pg = 1;\n}\n\npub fn read_var() {\n    let _v = spin_sdk::variables::get(\"db_url\").unwrap();\n    let touched_var = 1;\n}\n\npub fn fetch_ext() {\n    let _r = spin_sdk::http::send(\"https://example.invalid\").await;\n    let touched_http = 1;\n}\n\n#[spin_sdk::http_service]\npub fn handle() {\n    route();\n    let touched_entry = 1;\n}\n\npub fn plain() {\n    let touched_plain = 1;\n}\n";
    let repo = FixtureRepo::new(&[("src/spin.rs", base)]);
    repo.commit_change(&[("src/spin.rs", changed)]);
    let rust_lcov = repo.write_lcov(
        "rust.lcov",
        &lcov_record("src/spin.rs", &[(3, 0), (8, 0), (13, 0), (19, 0), (23, 0)]),
    );
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let mut lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    lines.sort();
    assert_eq!(
        lines,
        vec![
            ("src/spin.rs:13".to_string(), "e2e-only".to_string()),
            ("src/spin.rs:19".to_string(), "e2e-only".to_string()),
            ("src/spin.rs:23".to_string(), "untested".to_string()),
            ("src/spin.rs:3".to_string(), "e2e-only".to_string()),
            ("src/spin.rs:8".to_string(), "e2e-only".to_string()),
        ]
    );
}

#[test]
fn reports_uncovered_ui_change_as_untested_only() {
    // Catches UI rows going missing, and the Rust e2e-only rule leaking into
    // UI files: the added comment names a Spin API yet must stay untested.
    let base = "export function widget() {\n}\n";
    let changed = "export function widget() {\n    // mentions spin_sdk::pg::Connection::open but stays plain UI code\n    return 1;\n}\n";
    let repo = FixtureRepo::new(&[("ui/src/widget.ts", base)]);
    repo.commit_change(&[("ui/src/widget.ts", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", "TN:\n");
    let ui_lcov = repo.write_lcov(
        "ui.lcov",
        &lcov_record("ui/src/widget.ts", &[(1, 1), (2, 0), (3, 0)]),
    );
    let mut lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    lines.sort();
    assert_eq!(
        lines,
        vec![
            ("ui/src/widget.ts:2".to_string(), "untested".to_string()),
            ("ui/src/widget.ts:3".to_string(), "untested".to_string()),
        ]
    );
}

#[test]
fn script_measures_host_rust_and_vitest_coverage() {
    // Catches the script bypassing either coverage source (e.g. git diff only).
    let body = std::fs::read_to_string(script_path()).unwrap();
    assert!(body.contains("cargo llvm-cov"), "rust unit-test coverage");
    assert!(body.contains("vitest"), "ui coverage");
}

#[test]
fn ui_coverage_tooling_is_present() {
    // Catches the script having nothing to run for UI coverage with.
    let package = std::fs::read_to_string(repo_root().join("ui/package.json")).unwrap();
    assert!(
        package.contains("@vitest/coverage-"),
        "vitest coverage provider devDependency"
    );
    let vite = std::fs::read_to_string(repo_root().join("ui/vite.config.ts")).unwrap();
    assert!(vite.contains("coverage"), "vitest coverage settings");
}
