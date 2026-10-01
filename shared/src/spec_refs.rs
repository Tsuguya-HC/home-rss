use std::fs;
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub struct SpecCitation {
    pub line: usize,
    pub file: String,
    pub tests: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct MissingRef {
    pub line: usize,
    pub file: String,
    pub test: String,
}

pub fn parse_spec_citations(spec: &str) -> Vec<SpecCitation> {
    let mut out = Vec::new();
    for (index, line) in spec.lines().enumerate() {
        out.extend(parse_spec_line_citations(line, index + 1));
    }
    out
}

fn parse_spec_line_citations(line: &str, line_no: usize) -> Vec<SpecCitation> {
    let bytes = line.as_bytes();
    let spans = backtick_spans(line);
    let mut out = Vec::new();
    let mut i = 0;
    while i < line.len() {
        if starts_e2e_group_at(bytes, i) && followed_by_backtick_group(bytes, &spans, i + 3) {
            let mut tests = Vec::new();
            let mut j = advance_past_colon(bytes, i + 3);
            while let Some((name, next)) = take_backtick_at(bytes, &spans, j) {
                tests.push(name);
                j = next;
                j = match take_separator(bytes, &spans, j) {
                    Some(next) => next,
                    None => break,
                };
            }
            out.push(SpecCitation {
                line: line_no,
                file: "e2e/tests/api.rs".to_string(),
                tests,
            });
            i = j;
            continue;
        }
        if let Some((path, after_path)) = take_backtick_at(bytes, &spans, i) {
            if !(is_test_file_path(&path) && followed_by_colon(bytes, after_path)) {
                i += 1;
                continue;
            }
            let mut j = advance_past_colon(bytes, after_path);
            let mut tests = Vec::new();
            while let Some((name, next)) = take_backtick_at(bytes, &spans, j) {
                if !is_test_name(&name) {
                    break;
                }
                tests.push(name);
                j = next;
                j = match take_separator(bytes, &spans, j) {
                    Some(next) => next,
                    None => break,
                };
            }
            if !tests.is_empty() {
                out.push(SpecCitation {
                    line: line_no,
                    file: path,
                    tests,
                });
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn backtick_spans(line: &str) -> Vec<(usize, usize)> {
    let bytes = line.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            if let Some(rel) = line[i + 1..].find('`') {
                spans.push((i, i + 1 + rel + 1));
                i = i + 1 + rel + 1;
                continue;
            }
            break;
        }
        i += 1;
    }
    spans
}

fn take_backtick_at(bytes: &[u8], spans: &[(usize, usize)], pos: usize) -> Option<(String, usize)> {
    skip_gap(bytes, spans, pos).and_then(|start| {
        spans
            .iter()
            .find(|&&(s, _)| s == start)
            .map(|&(s, e)| (String::from_utf8_lossy(&bytes[s + 1..e - 1]).to_string(), e))
    })
}

fn skip_gap(bytes: &[u8], spans: &[(usize, usize)], mut pos: usize) -> Option<usize> {
    while pos < bytes.len() {
        if spans.iter().any(|&(s, e)| pos > s && pos < e) {
            return None;
        }
        match bytes[pos] {
            b' ' | b'\t' => pos += 1,
            _ => return Some(pos),
        }
    }
    None
}

fn take_separator(bytes: &[u8], spans: &[(usize, usize)], pos: usize) -> Option<usize> {
    let mut p = pos;
    while p < bytes.len() && (bytes[p] == b' ' || bytes[p] == b'\t') {
        p += 1;
    }
    if p < bytes.len() && (bytes[p] == b',' || bytes[p] == b';') {
        p += 1;
        while p < bytes.len() && (bytes[p] == b' ' || bytes[p] == b'\t') {
            p += 1;
        }
        if p < bytes.len() && spans.iter().any(|&(s, _)| s == p) && bytes[p] == b'`' {
            return Some(p);
        }
    }
    None
}

fn starts_e2e_group_at(bytes: &[u8], pos: usize) -> bool {
    pos + 3 <= bytes.len()
        && &bytes[pos..pos + 3] == b"e2e"
        && (pos == 0 || !is_word_byte(bytes[pos - 1]))
        && (pos + 3 >= bytes.len() || !is_word_byte(bytes[pos + 3]))
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn followed_by_backtick_group(bytes: &[u8], spans: &[(usize, usize)], pos: usize) -> bool {
    let mut p = pos;
    while p < bytes.len() && (bytes[p] == b' ' || bytes[p] == b'\t' || bytes[p] == b':') {
        p += 1;
    }
    spans.iter().any(|&(s, _)| s == p)
}

fn followed_by_colon(bytes: &[u8], pos: usize) -> bool {
    let mut p = pos;
    while p < bytes.len() && (bytes[p] == b' ' || bytes[p] == b'\t') {
        p += 1;
    }
    p < bytes.len() && bytes[p] == b':'
}

fn advance_past_colon(bytes: &[u8], pos: usize) -> usize {
    let mut p = pos;
    while p < bytes.len() && (bytes[p] == b' ' || bytes[p] == b'\t') {
        p += 1;
    }
    if p < bytes.len() && bytes[p] == b':' {
        p += 1;
    }
    p
}

fn is_test_file_path(path: &str) -> bool {
    if path == "e2e/tests/api.rs" {
        return true;
    }
    if path.ends_with(".test.ts") || path.ends_with(".test.tsx") {
        return path.contains('/');
    }
    path.ends_with(".rs") && path.contains('/')
}

fn is_test_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(char::is_whitespace)
        && !name.contains('/')
        && !name.contains('.')
        && !name.contains('(')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ' ')
        && name.trim() == name
}

pub fn rust_test_names(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut pending_attr = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[test]")
            || trimmed.starts_with("#[tokio::test")
            || trimmed.starts_with("#[rstest")
        {
            pending_attr = true;
            if let Some(name) = fn_name_on_same_line(trimmed) {
                names.push(name);
                pending_attr = false;
            }
            continue;
        }
        if pending_attr {
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                continue;
            }
            if let Some(name) = fn_name_on_same_line(trimmed) {
                names.push(name);
            }
            pending_attr = false;
        }
    }
    names
}

fn fn_name_on_same_line(line: &str) -> Option<String> {
    let mut rest = line;
    if let Some(pos) = rest.find("fn ") {
        rest = &rest[pos + 3..];
        if rest.starts_with('_') || rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
            let end = rest
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            return Some(rest[..end].to_string());
        }
    }
    None
}

pub fn vitest_test_names(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut names = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && !source.is_char_boundary(i) {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let matched = if source[i..].starts_with("it(") {
            Some(3)
        } else if source[i..].starts_with("test(") {
            Some(5)
        } else {
            None
        };
        if let Some(prefix) = matched {
            let is_call = i == 0
                || !(bytes[i - 1].is_ascii_alphanumeric()
                    || bytes[i - 1] == b'_'
                    || bytes[i - 1] == b'.');
            if is_call {
                let mut j = i + prefix;
                while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j += 1;
                }
                if j < bytes.len() && (bytes[j] == b'\'' || bytes[j] == b'"') {
                    let quote = bytes[j];
                    let mut k = j + 1;
                    let mut name = String::new();
                    while k < bytes.len() && bytes[k] != quote {
                        if bytes[k] == b'\\' && k + 1 < bytes.len() {
                            name.push(bytes[k + 1] as char);
                            k += 2;
                        } else {
                            name.push(bytes[k] as char);
                            k += 1;
                        }
                    }
                    names.push(name);
                    i = k + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    names
}

pub fn find_missing_refs(spec: &str, load: &dyn Fn(&str) -> Option<String>) -> Vec<MissingRef> {
    let mut missing = Vec::new();
    for citation in parse_spec_citations(spec) {
        let Some(source) = load(&citation.file) else {
            for test in &citation.tests {
                missing.push(MissingRef {
                    line: citation.line,
                    file: citation.file.clone(),
                    test: test.clone(),
                });
            }
            continue;
        };
        let defined = if citation.file.ends_with(".test.ts") || citation.file.ends_with(".test.tsx")
        {
            vitest_test_names(&source)
        } else {
            rust_test_names(&source)
        };
        for test in &citation.tests {
            if !defined.contains(test) {
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

pub fn check_workspace_specs(root: &Path) -> Vec<MissingRef> {
    let Ok(spec) = fs::read_to_string(root.join("docs/spec.md")) else {
        return Vec::new();
    };
    find_missing_refs(&spec, &|file| fs::read_to_string(root.join(file)).ok())
}

pub fn workspace_suite_invokes_spec_check(root: &Path) -> bool {
    fs::read_to_string(root.join("scripts/test.sh")).is_ok_and(|suite| suite.contains("spec_refs"))
}
