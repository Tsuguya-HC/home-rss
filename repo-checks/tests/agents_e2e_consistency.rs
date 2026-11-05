use std::path::PathBuf;

fn repo_root() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn agents_e2e_section_mentions_fetcher() {
    // Catches the e2e paragraph going stale when run.sh gains a service (#245).
    let root = repo_root();
    let agents = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
    let block = agents
        .split("\n\n")
        .find(|block| block.contains("e2e/run.sh"))
        .expect("AGENTS.md describes e2e/run.sh");
    assert!(
        block.contains("fetcher"),
        "e2e/run.sh starts the fetcher too, so this paragraph must say so:\n{block}"
    );
}

#[test]
fn fetch_posts_go_through_post_fetch_helper() {
    // Catches a second spelling of the /fetch POST drifting from the helper (#245).
    let root = repo_root();
    let api = std::fs::read_to_string(root.join("e2e/tests/api.rs")).unwrap();
    let uses = api.matches("E2E_FETCHER_URL").count();
    assert_eq!(
        uses, 1,
        "all /fetch POSTs must go through post_fetch(), found {uses} uses of E2E_FETCHER_URL"
    );
}
