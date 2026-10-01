use std::path::Path;

pub struct SpecCitation {
    pub line: usize,
    pub file: String,
    pub tests: Vec<String>,
}

pub struct MissingRef {
    pub line: usize,
    pub file: String,
    pub test: String,
}

fn parse_quoted_list(rest: &str) -> Option<Vec<String>> {
    let mut names = Vec::new();
    let mut rest = rest.trim_start();
    loop {
        rest = rest.trim_start();
        if !rest.starts_with('`') {
            return None;
        }
        let end = rest[1..].find('`')?;
        names.push(rest[1..1 + end].to_string());
        rest = rest[1 + end + 1..].trim_start();
        if let Some(after) = rest.strip_prefix(',') {
            rest = after;
            continue;
        }
        return Some(names);
    }
}

fn is_test_file_path(path: &str) -> bool {
    if path == "e2e" {
        return true;
    }
    if !path.contains('/') {
        return false;
    }
    path.ends_with(".rs") || path.ends_with(".test.ts") || path.ends_with(".test.tsx")
}

fn citation_after(path: &str, rest: &str, line_no: usize) -> Option<SpecCitation> {
    let rest = rest.strip_prefix(':')?.trim_start();
    let tests = parse_quoted_list(rest)?;
    Some(SpecCitation {
        line: line_no,
        file: if path == "e2e" {
            "e2e/tests/api.rs".to_string()
        } else {
            path.to_string()
        },
        tests,
    })
}

fn is_bare_e2e_at(line: &str, index: usize) -> bool {
    let prev = line[..index].chars().next_back();
    let before_ok = prev.is_none_or(|c| !c.is_alphanumeric() && c != '_');
    before_ok && line[index + 3..].starts_with(':')
}

pub fn parse_spec_citations(spec: &str) -> Vec<SpecCitation> {
    let mut citations = Vec::new();
    for (index, line) in spec.lines().enumerate() {
        let line_no = index + 1;
        let mut pos = 0;
        while pos < line.len() {
            let rest = &line[pos..];
            let next_tick = rest.find('`');
            let e2e_at = rest.find("e2e").map(|i| pos + i);
            match (next_tick.map(|i| pos + i), e2e_at) {
                (Some(tick), Some(e2e)) if e2e < tick => {
                    if is_bare_e2e_at(line, e2e)
                        && let Some(citation) = citation_after("e2e", &line[e2e + 3..], line_no)
                    {
                        citations.push(citation);
                    }
                    pos = e2e + 3;
                }
                (Some(tick), _) => {
                    let after_open = &line[tick + 1..];
                    let Some(close) = after_open.find('`') else {
                        break;
                    };
                    let path = &after_open[..close];
                    if is_test_file_path(path)
                        && let Some(citation) =
                            citation_after(path, after_open[close + 1..].trim_start(), line_no)
                    {
                        citations.push(citation);
                    }
                    pos = tick + 1 + close + 1;
                }
                (None, _) => break,
            }
        }
    }
    citations
}

fn next_fn_name(lines: &[&str], mut index: usize) -> Option<String> {
    while index < lines.len() {
        let line = lines[index].trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            index += 1;
            continue;
        }
        let rest = line
            .strip_prefix("pub async fn ")
            .or_else(|| line.strip_prefix("async fn "))
            .or_else(|| line.strip_prefix("pub fn "))
            .or_else(|| line.strip_prefix("fn "))?;
        let end = rest
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        return Some(rest[..end].to_string());
    }
    None
}

pub fn rust_test_names(source: &str) -> Vec<String> {
    let lines: Vec<&str> = source.lines().collect();
    let mut names = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let marker = line.trim();
        if (marker == "#[test]" || marker == "#[tokio::test]")
            && let Some(name) = next_fn_name(&lines, index + 1)
        {
            names.push(name);
        }
    }
    names
}

fn quoted_name(after: &str) -> Option<String> {
    let after = after.trim_start();
    let quote = after.chars().next()?;
    if quote != '\'' && quote != '"' && quote != '`' {
        return None;
    }
    let end = after[1..].find(quote)?;
    Some(after[1..1 + end].to_string())
}

pub fn vitest_test_names(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        let rest = trimmed
            .strip_prefix("it(")
            .or_else(|| trimmed.strip_prefix("test("))
            .or_else(|| {
                trimmed
                    .strip_prefix("it.")
                    .and_then(|r| r.split_once('(').map(|(_, args)| args))
            })
            .or_else(|| {
                trimmed
                    .strip_prefix("test.")
                    .and_then(|r| r.split_once('(').map(|(_, args)| args))
            });
        if let Some(rest) = rest
            && let Some(name) = quoted_name(rest)
        {
            names.push(name);
        }
    }
    names
}

pub fn find_missing_refs(
    citations: &[SpecCitation],
    test_names_in: impl Fn(&str) -> Option<Vec<String>>,
) -> Vec<MissingRef> {
    let mut missing = Vec::new();
    for citation in citations {
        let names = test_names_in(&citation.file);
        for test in &citation.tests {
            let found = names
                .as_ref()
                .is_some_and(|names| names.iter().any(|name| name == test));
            if !found {
                missing.push(MissingRef {
                    line: citation.line,
                    file: citation.file.clone(),
                    test: test.clone(),
                });
            }
        }
    }
    missing
}

pub fn check_workspace_specs(repo_root: &Path) -> Vec<MissingRef> {
    let spec_path = repo_root.join("docs/spec.md");
    let spec = match std::fs::read_to_string(&spec_path) {
        Ok(spec) => spec,
        Err(_) => {
            return vec![MissingRef {
                line: 0,
                file: "docs/spec.md".to_string(),
                test: "(unreadable spec)".to_string(),
            }];
        }
    };
    let citations = parse_spec_citations(&spec);
    find_missing_refs(&citations, |file| {
        let source = std::fs::read_to_string(repo_root.join(file)).ok()?;
        Some(if file.ends_with(".tsx") || file.ends_with(".ts") {
            vitest_test_names(&source)
        } else {
            rust_test_names(&source)
        })
    })
}
