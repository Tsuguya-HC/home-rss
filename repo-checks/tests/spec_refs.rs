use home_rss_repo_checks::spec_refs::check_spec;

#[test]
fn repo_spec_cites_only_tests_that_exist() {
    // Catches stale citations like the #196 / #201 renames.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let spec = std::fs::read_to_string(root.join("docs/spec.md")).unwrap();
    let missing = check_spec(&spec, &|path| std::fs::read_to_string(root.join(path)).ok());
    assert!(
        missing.is_empty(),
        "{}",
        missing
            .iter()
            .map(|m| format!("spec line {}: {}: {}", m.line, m.path, m.name))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
