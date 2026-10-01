use std::fs;
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
    while i < bytes.len() {
        if starts_e2e_group_at(bytes, i) {
            let mut j = advance_past_colon(bytes, i + 3);
            let mut tests = Vec::new();
            while let Some((name, next)) = take_backtick_at(bytes, &spans, j) {
                if !is_test_name(&name) {
                    break;
                }
                tests.push(name);
                j = next;
                j = match take_separator(line, j) {
                    Some(next) => next,
                    None => break,
                };
            }
            if !tests.is_empty() {
                out.push(SpecCitation {
                    line: line_no,
                    file: "e2e/tests/api.rs".to_string(),
                    tests,
                });
                i = j;
                continue;
            }
        }
        if let Some((path, after_path)) = take_backtick_at(bytes, &spans, i)
            && is_test_file_path(&path)
            && followed_by_colon(bytes, after_path)
        {
            let mut j = advance_past_colon(bytes, after_path);
            let mut tests = Vec::new();
            while let Some((name, next)) = take_backtick_at(bytes, &spans, j) {
                if !is_test_name(&name) {
                    break;
                }
                tests.push(name);
                j = next;
                j = match take_separator(line, j) {
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
    skip_gap(bytes, pos).and_then(|start| {
        spans
            .iter()
            .find(|&&(s, _)| s == start)
            .map(|&(s, e)| (String::from_utf8_lossy(&bytes[s + 1..e - 1]).to_string(), e))
    })
}

fn skip_gap(bytes: &[u8], mut pos: usize) -> Option<usize> {
    while pos < bytes.len() {
        match bytes[pos] {
            b' ' | b'\t' => pos += 1,
            _ => return Some(pos),
        }
    }
    None
}

fn take_separator(line: &str, pos: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut p = pos;
    while p < bytes.len() && (bytes[p] == b' ' || bytes[p] == b'\t') {
        p += 1;
    }
    let after = if p < bytes.len() && (bytes[p] == b',' || bytes[p] == b';') {
        p + 1
    } else if line
        .get(p..)
        .is_some_and(|rest| rest.starts_with('\u{3001}'))
    {
        p + '\u{3001}'.len_utf8()
    } else {
        return None;
    };
    let mut q = after;
    while q < bytes.len() && (bytes[q] == b' ' || bytes[q] == b'\t') {
        q += 1;
    }
    if q < bytes.len() && bytes[q] == b'`' {
        Some(q)
    } else {
        None
    }
}

fn starts_e2e_group_at(bytes: &[u8], pos: usize) -> bool {
    pos + 3 <= bytes.len()
        && bytes[pos..pos + 3] == b"e2e"[..]
        && (pos == 0 || !is_word_byte(bytes[pos - 1]))
        && (pos + 3 >= bytes.len() || !is_word_byte(bytes[pos + 3]))
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
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
        && name.trim() == name
        && name.chars().all(|c| {
            c.is_alphanumeric() || c == ' ' || c == '_' || c == '-' || c == ',' || c == ':'
        })
}

pub fn rust_test_names(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut pending_attr = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[test") || trimmed.starts_with("#[tokio::test") {
            pending_attr = true;
            if let Some(name) = fn_name_on_line(trimmed) {
                names.push(name);
                pending_attr = false;
            }
            continue;
        }
        if pending_attr {
            if let Some(name) = fn_name_on_line(trimmed) {
                names.push(name);
            }
            pending_attr = false;
        }
    }
    names
}

fn fn_name_on_line(line: &str) -> Option<String> {
    let pos = line.find("fn ")?;
    let rest = &line[pos + 3..];
    let mut end = rest.len();
    for (idx, c) in rest.char_indices() {
        if !(c.is_ascii_alphanumeric() || c == '_') {
            end = idx;
            break;
        }
    }
    let name = &rest[..end];
    if name.is_empty() {
        return None;
    }
    let first = name.as_bytes()[0];
    if !first.is_ascii_alphabetic() && first != b'_' {
        return None;
    }
    Some(name.to_string())
}

pub fn vitest_test_names(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut names = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !source.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let rest = &source[i..];
        let width = if rest.starts_with("it(") {
            2
        } else if rest.starts_with("test(") {
            4
        } else {
            i += 1;
            continue;
        };
        if i > 0
            && (bytes[i - 1].is_ascii_alphanumeric()
                || bytes[i - 1] == b'_'
                || bytes[i - 1] == b'.')
        {
            i += 1;
            continue;
        }
        let mut j = i + width + 1;
        while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
            j += 1;
        }
        if j < bytes.len() && (bytes[j] == b'\'' || bytes[j] == b'"') {
            let quote = bytes[j];
            let mut k = j + 1;
            while k < bytes.len() && bytes[k] != quote {
                if bytes[k] == b'\\' && k + 1 < bytes.len() {
                    k += 2;
                } else {
                    k += 1;
                }
            }
            names.push(String::from_utf8_lossy(&bytes[j + 1..k]).to_string());
            i = k + 1;
            continue;
        }
        i += 1;
    }
    names
}

pub fn find_missing_refs(spec: &str, load: &dyn Fn(&str) -> Option<String>) -> Vec<MissingRef> {
    let mut missing = Vec::new();
    for citation in parse_spec_citations(spec) {
        let source = load(&citation.file).unwrap_or_default();
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
