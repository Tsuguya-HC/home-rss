use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(100000);

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
            "home-rss-diff-coverage-extra-{}-{id}",
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

    fn remove(&self, rel: &str) {
        std::fs::remove_file(self.dir.join(rel)).unwrap();
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", "remove"]);
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
        lines.push((loc.to_string(), status.to_string()));
    }
    lines
}

#[test]
fn ignores_non_code_change() {
    // Catches the script reporting changed lines in files neither coverage
    // source describes (docs, manifests, configs).
    let repo = FixtureRepo::new(&[("Cargo.toml", "[pkg]\n")]);
    repo.commit_change(&[("Cargo.toml", "[pkg]\nversion = 2\n")]);
    let rust_lcov = repo.write_lcov("rust.lcov", "TN:\n");
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    assert!(report_lines(&repo.run(&rust_lcov, &ui_lcov)).is_empty());
}

#[test]
fn ignores_non_ui_code_change() {
    // Catches lockfile, manifest, and config churn under ui/ showing up as
    // untested: only .ts/.tsx (and .js/.jsx/.mts/.cts) carry UI coverage.
    let repo = FixtureRepo::new(&[("ui/package.json", "{\"a\": 1}\n")]);
    repo.commit_change(&[("ui/package.json", "{\"a\": 1}\n{\"b\": 2}\n")]);
    let rust_lcov = repo.write_lcov("rust.lcov", "TN:\n");
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    assert!(report_lines(&repo.run(&rust_lcov, &ui_lcov)).is_empty());
}

#[test]
fn tolerates_missing_lcov_file() {
    // Catches the script crashing when a coverage file is absent instead of
    // treating its lines as uncovered.
    let repo = FixtureRepo::new(&[("src/lib.rs", "pub fn present() {}\n")]);
    repo.commit_change(&[("src/lib.rs", "pub fn present() {}\npub fn added() {}\n")]);
    let missing = repo.dir.join("no-such-rust.lcov");
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&missing, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/lib.rs:2".to_string(), "untested".to_string())]
    );
}

#[test]
fn skips_malformed_lcov_lines() {
    // Catches a corrupt DA entry or a DA before any SF aborting the parse or
    // misattributing hits.
    let repo = FixtureRepo::new(&[("src/lib.rs", "pub fn present() {}\n")]);
    repo.commit_change(&[("src/lib.rs", "pub fn present() {}\npub fn added() {}\n")]);
    let body = "TN:\nDA:9,1\nSF:src/lib.rs\nDA:abc\nDA:2\nDA:2,0\nend_of_record\n";
    let rust_lcov = repo.write_lcov("rust.lcov", body);
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/lib.rs:2".to_string(), "untested".to_string())]
    );
}

#[test]
fn braces_in_strings_and_comments_do_not_break_function_range() {
    // Catches brace counting that reads string or comment text: the stray
    // braces below must not end the function early and demote the Spin line
    // to untested.
    let base = "pub fn f<'a>(x: &'a str) {\n    let s = \"{ not a brace \\\" }\";\n    let raw = r#\"{ also not }\"#;\n    let c = '}';\n    // } stray comment brace\n    /* {\n    still comment\n    } */\n}\n";
    let changed = "pub fn f<'a>(x: &'a str) {\n    let s = \"{ not a brace \\\" }\";\n    let raw = r#\"{ also not }\"#;\n    let c = '}';\n    // } stray comment brace\n    /* {\n    still comment\n    } */\n    let _c = spin_sdk::pg::Connection::open(\"x\").unwrap();\n}\n";
    let repo = FixtureRepo::new(&[("src/quotes.rs", base)]);
    repo.commit_change(&[("src/quotes.rs", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/quotes.rs", &[(9, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/quotes.rs:9".to_string(), "e2e-only".to_string())]
    );
}

#[test]
fn braceless_fn_does_not_leak_its_range() {
    // Catches a bodyless `fn` (trait declaration) swallowing later lines:
    // the Spin mention below is a comment outside any function, so it must
    // stay untested instead of leaking into the declaration's range.
    let base = "trait T {\n    fn decl();\n}\n";
    let changed = "trait T {\n    fn decl();\n}\n// note spin_sdk::pg::Connection::open\n";
    let repo = FixtureRepo::new(&[("src/decl.rs", base)]);
    repo.commit_change(&[("src/decl.rs", changed)]);
    let rust_lcov = repo.write_lcov("rust.lcov", &lcov_record("src/decl.rs", &[(4, 0)]));
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/decl.rs:4".to_string(), "untested".to_string())]
    );
}

#[test]
fn deleted_file_reports_nothing() {
    // Catches the script choking on a diff whose new side is /dev/null.
    let repo = FixtureRepo::new(&[("src/gone.rs", "pub fn gone() {}\n")]);
    repo.remove("src/gone.rs");
    let rust_lcov = repo.write_lcov("rust.lcov", "TN:\n");
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    assert!(report_lines(&repo.run(&rust_lcov, &ui_lcov)).is_empty());
}

#[test]
fn absolute_sf_paths_match_by_suffix() {
    // Catches coverage lookups that only match the exact repo-relative path:
    // llvm-cov may emit absolute SF paths.
    let repo = FixtureRepo::new(&[("src/lib.rs", "pub fn present() {}\n")]);
    repo.commit_change(&[("src/lib.rs", "pub fn present() {}\npub fn added() {}\n")]);
    let rust_lcov = repo.write_lcov(
        "rust.lcov",
        &lcov_record("/build/root/src/lib.rs", &[(1, 1), (2, 0)]),
    );
    let ui_lcov = repo.write_lcov("ui.lcov", "TN:\n");
    let lines = report_lines(&repo.run(&rust_lcov, &ui_lcov));
    assert_eq!(
        lines,
        vec![("src/lib.rs:2".to_string(), "untested".to_string())]
    );
    let covered_lcov = repo.write_lcov(
        "covered.lcov",
        &lcov_record("/build/root/src/lib.rs", &[(1, 1), (2, 3)]),
    );
    assert!(report_lines(&repo.run(&covered_lcov, &ui_lcov)).is_empty());
}
