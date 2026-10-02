fn main() {
    let args: Vec<String> = std::env::args().collect();
    let report = home_rss_repo_checks::diff_coverage::run(&args[1], &args[2], &args[3]);
    print!("{report}");
}
