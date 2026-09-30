use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitedTest {
    pub line: usize,
    pub file: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingTest {
    pub line: usize,
    pub file: String,
    pub name: String,
}

// 走査位置はすべてバイト位置で、ASCII の区切り（backtick・コロン・空白）からだけ
// 進める。スライスの両端は ASCII バイト上になるので、非 ASCII 行でも文字境界を割らない。
fn is_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn is_path_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b'+' | b'@' | b'~')
}

fn path_head_at(line: &str, from: usize) -> Option<(usize, usize, String)> {
    let bytes = line.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && bytes[j] != b'`' {
            j += 1;
        }
        if j >= bytes.len() {
            return None;
        }
        let inner = &line[i + 1..j];
        let mut k = j + 1;
        while k < bytes.len() && bytes[k] == b' ' {
            k += 1;
        }
        if k < bytes.len() && bytes[k] == b':' {
            let mut m = k + 1;
            while m < bytes.len() && bytes[m] == b' ' {
                m += 1;
            }
            if m < bytes.len()
                && bytes[m] == b'`'
                && inner.contains('/')
                && inner.bytes().all(is_path_char)
            {
                return Some((i, j, inner.to_string()));
            }
        }
        i = j + 1;
    }
    None
}

fn name_list_at(line: &str, from: usize) -> (Vec<String>, usize) {
    let bytes = line.as_bytes();
    let mut names = Vec::new();
    let mut i = from;
    loop {
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'`' {
            break;
        }
        let mut j = i + 1;
        while j < bytes.len() && bytes[j] != b'`' {
            j += 1;
        }
        if j >= bytes.len() {
            break;
        }
        let inner = &line[i + 1..j];
        if inner.is_empty() || !inner.bytes().all(is_name_char) {
            break;
        }
        names.push(inner.to_string());
        let mut k = j + 1;
        while k < bytes.len() && bytes[k] == b' ' {
            k += 1;
        }
        if k < bytes.len() && bytes[k] == b',' {
            i = k + 1;
            continue;
        }
        i = k;
        break;
    }
    (names, i)
}

fn bare_e2e_at(line: &str, from: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut i = from;
    while i + 4 <= bytes.len() {
        if bytes[i..].starts_with(b"e2e:") && (i == 0 || !is_name_char(bytes[i - 1])) {
            let ticks = bytes[..i].iter().filter(|b| **b == b'`').count();
            if ticks % 2 == 0 {
                return Some(i);
            }
            i += 4;
            continue;
        }
        i += 1;
    }
    None
}

fn bare_name_after(line: &str, bare_pos: usize) -> Option<(String, usize)> {
    let rest = &line[bare_pos + 4..];
    let trimmed = rest.trim_start();
    let stripped = trimmed.strip_prefix('`')?;
    let close = stripped.find('`')?;
    let name = &stripped[..close];
    if name.is_empty() || !name.bytes().all(is_name_char) {
        return None;
    }
    let next = bare_pos + 4 + (rest.len() - trimmed.len()) + close + 2;
    Some((name.to_string(), next))
}

pub fn parse_spec_refs(spec: &str) -> Vec<CitedTest> {
    let mut cited = Vec::new();
    for (index, line) in spec.lines().enumerate() {
        let line_no = index + 1;
        let mut pos = 0;
        while pos < line.len() {
            let head = path_head_at(line, pos);
            let bare = bare_e2e_at(line, pos);
            match (head, bare) {
                (Some((start, _, _)), Some(bare_pos)) if bare_pos < start => {
                    match bare_name_after(line, bare_pos) {
                        Some((name, next)) => {
                            cited.push(CitedTest {
                                line: line_no,
                                file: None,
                                name,
                            });
                            pos = next;
                        }
                        None => pos = bare_pos + 4,
                    }
                }
                (Some((_, end, file)), _) => {
                    let bytes = line.as_bytes();
                    let mut k = end + 1;
                    while k < bytes.len() && bytes[k] == b' ' {
                        k += 1;
                    }
                    let (names, next) = name_list_at(line, k + 1);
                    for name in names {
                        cited.push(CitedTest {
                            line: line_no,
                            file: Some(file.clone()),
                            name,
                        });
                    }
                    pos = next.max(end + 2);
                }
                (None, Some(bare_pos)) => match bare_name_after(line, bare_pos) {
                    Some((name, next)) => {
                        cited.push(CitedTest {
                            line: line_no,
                            file: None,
                            name,
                        });
                        pos = next;
                    }
                    None => pos = bare_pos + 4,
                },
                (None, None) => break,
            }
        }
    }
    cited
}

fn is_test_attribute(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("#[test") || trimmed.starts_with("#[tokio::test")
}

fn rust_fn_name(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 2 <= bytes.len() {
        if bytes[i] == b'f'
            && bytes[i + 1] == b'n'
            && (i == 0 || (!is_name_char(bytes[i - 1]) && bytes[i - 1] != b':'))
            && (i + 2 >= bytes.len() || !is_name_char(bytes[i + 2]))
        {
            let mut j = i + 2;
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
                j += 1;
            }
            let start = j;
            while j < bytes.len() && is_name_char(bytes[j]) {
                j += 1;
            }
            if start < j {
                return Some(&line[start..j]);
            }
            return None;
        }
        i += 1;
    }
    None
}

fn rust_test_names(content: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut armed = false;
    for line in content.lines() {
        if is_test_attribute(line) {
            armed = true;
            continue;
        }
        if !armed {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
            continue;
        }
        if let Some(name) = rust_fn_name(line) {
            names.push(name.to_string());
        }
        armed = false;
    }
    names
}

fn is_ts_word_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

fn vitest_test_names(content: &str) -> Vec<String> {
    let bytes = content.as_bytes();
    let mut names = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let len = if bytes[i..].starts_with(b"it") && i + 2 < bytes.len() {
            2
        } else if bytes[i..].starts_with(b"test") && i + 4 < bytes.len() {
            4
        } else {
            i += 1;
            continue;
        };
        if i > 0 && (is_ts_word_char(bytes[i - 1]) || bytes[i - 1] == b'.') {
            i += 1;
            continue;
        }
        if !matches!(bytes[i + len], b'(' | b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
            continue;
        }
        let mut j = i + len;
        while j < bytes.len() && bytes[j] != b'(' {
            j += 1;
        }
        if j >= bytes.len() {
            break;
        }
        j += 1;
        while j < bytes.len() && matches!(bytes[j], b' ' | b'\t' | b'\n' | b'\r') {
            j += 1;
        }
        if j >= bytes.len() {
            break;
        }
        let quote = bytes[j];
        if quote != b'\'' && quote != b'"' && quote != b'`' {
            i = j + 1;
            continue;
        }
        let mut k = j + 1;
        let mut name = String::new();
        while k < bytes.len() && bytes[k] != quote {
            if bytes[k] == b'\\' && k + 1 < bytes.len() {
                name.push(bytes[k + 1] as char);
                k += 2;
                continue;
            }
            name.push(bytes[k] as char);
            k += 1;
        }
        if k < bytes.len() {
            names.push(name);
            i = k + 1;
        } else {
            break;
        }
    }
    names
}

pub fn test_names(repo_path: &str, content: &str) -> Vec<String> {
    if repo_path.ends_with(".rs") {
        rust_test_names(content)
    } else if repo_path.ends_with(".test.ts") || repo_path.ends_with(".test.tsx") {
        vitest_test_names(content)
    } else {
        Vec::new()
    }
}

fn is_e2e_candidate(path: &str) -> bool {
    path.starts_with("e2e/tests/") && path.ends_with(".rs")
}

pub fn check_contents(spec: &str, files: &BTreeMap<String, String>) -> Vec<MissingTest> {
    let mut missing = Vec::new();
    for cited in parse_spec_refs(spec) {
        match cited.file {
            Some(file) => {
                let found = files
                    .get(&file)
                    .map(|content| test_names(&file, content).contains(&cited.name))
                    .unwrap_or(false);
                if !found {
                    missing.push(MissingTest {
                        line: cited.line,
                        file,
                        name: cited.name,
                    });
                }
            }
            None => {
                let found = files
                    .iter()
                    .filter(|(path, _)| is_e2e_candidate(path))
                    .any(|(path, content)| test_names(path, content).contains(&cited.name));
                if !found {
                    missing.push(MissingTest {
                        line: cited.line,
                        file: "e2e".to_string(),
                        name: cited.name,
                    });
                }
            }
        }
    }
    missing
}

pub fn check_repo(root: &Path) -> Vec<MissingTest> {
    let spec = match std::fs::read_to_string(root.join("docs/spec.md")) {
        Ok(spec) => spec,
        Err(_) => return Vec::new(),
    };
    let cited = parse_spec_refs(&spec);
    let mut paths: Vec<String> = Vec::new();
    for entry in &cited {
        if let Some(file) = &entry.file
            && !paths.contains(file)
        {
            paths.push(file.clone());
        }
    }
    let mut files = BTreeMap::new();
    for path in paths {
        match std::fs::read_to_string(root.join(&path)) {
            Ok(content) => {
                files.insert(path, content);
            }
            Err(_) => {
                files.insert(path, String::new());
            }
        }
    }
    if cited.iter().any(|entry| entry.file.is_none())
        && let Ok(entries) = std::fs::read_dir(root.join("e2e/tests"))
    {
        let mut names: Vec<String> = Vec::new();
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "rs")
                && let Some(name) = entry.file_name().to_str()
            {
                names.push(format!("e2e/tests/{name}"));
            }
        }
        names.sort();
        for path in names {
            if !files.contains_key(&path)
                && let Ok(content) = std::fs::read_to_string(root.join(&path))
            {
                files.insert(path, content);
            }
        }
    }
    check_contents(&spec, &files)
}

pub fn format_missing(missing: &[MissingTest]) -> String {
    missing
        .iter()
        .map(|entry| {
            format!(
                "docs/spec.md:{}: {}: `{}` not found",
                entry.line, entry.file, entry.name
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
