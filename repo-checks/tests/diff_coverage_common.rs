// 各テストバイナリに取り込まれ、使う項目がバイナリごとに違う共有 fixture の
// ため、使わない項目があっても警告にしない（外すと clippy -D warnings が落ちる）。
#![allow(dead_code)]
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

// Fixture seam the script must honour: when these point at LCOV files, the
// script reads coverage from them instead of running the real tools, so the
// tests stay hermetic (no llvm-cov, vitest, or network).
pub const RUST_LCOV_ENV: &str = "DIFF_COVERAGE_RUST_LCOV";
pub const UI_LCOV_ENV: &str = "DIFF_COVERAGE_UI_LCOV";

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

pub fn script_path() -> PathBuf {
    repo_root().join("scripts/diff-coverage.sh")
}

pub struct FixtureRepo {
    pub dir: PathBuf,
    pub base: String,
}

impl FixtureRepo {
    pub fn new(base_files: &[(&str, &str)]) -> Self {
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

    pub fn commit_change(&self, files: &[(&str, &str)]) {
        self.write_all(files);
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", "change"]);
    }

    pub fn remove(&self, rel: &str) {
        std::fs::remove_file(self.dir.join(rel)).unwrap();
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", "remove"]);
    }

    pub fn write_lcov(&self, name: &str, body: &str) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    pub fn run(&self, rust_lcov: &Path, ui_lcov: &Path) -> Output {
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

pub fn lcov_record(sf: &str, hits: &[(u32, u32)]) -> String {
    let mut body = format!("TN:\nSF:{sf}\n");
    for (line, hit) in hits {
        body.push_str(&format!("DA:{line},{hit}\n"));
    }
    body.push_str("end_of_record\n");
    body
}

// Every output line must be `path:line<TAB>untested|e2e-only`; malformed lines
// fail the test instead of slipping through the assertions.
pub fn report_lines(output: &Output) -> Vec<(String, String)> {
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
