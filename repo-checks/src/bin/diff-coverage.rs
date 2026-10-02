use std::path::Path;
use std::process::ExitCode;

use home_rss_repo_checks::diff_coverage;

const USAGE: &str = "usage: diff-coverage <repo-root> <diff> <rust-lcov> <ui-lcov>

Prints the lines the diff adds that no test runs, one per line:
  path:line<TAB>untested          a unit test could run it
  path:line<TAB>e2e-only          only e2e can run it
  path<TAB>no-coverage-data       the coverage does not mention the file
Exit status: 0 nothing to print, 1 something printed, 2 error.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, diff, rust_lcov, ui_lcov] = args.as_slice() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    match diff_coverage::run(
        Path::new(root),
        Path::new(diff),
        Path::new(rust_lcov),
        Path::new(ui_lcov),
    ) {
        Ok(findings) if findings.is_empty() => ExitCode::SUCCESS,
        Ok(findings) => {
            for f in &findings {
                println!("{f}");
            }
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("diff-coverage: {e}");
            ExitCode::from(2)
        }
    }
}
