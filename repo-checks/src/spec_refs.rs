pub struct SpecRef {
    pub line: u32,
    pub path: String,
    pub name: String,
}

pub struct MissingRef {
    pub line: u32,
    pub path: String,
    pub name: String,
}

fn is_ident_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '$'
}

fn without_parenthesised_asides(line: &str) -> String {
    let mut kept = String::with_capacity(line.len());
    let mut depth = 0u32;
    for ch in line.chars() {
        match ch {
            '（' => depth += 1,
            '）' => depth = depth.saturating_sub(1),
            _ if depth == 0 => kept.push(ch),
            _ => {}
        }
    }
    kept
}

fn bare_e2e_end(segment: &[char], i: usize) -> Option<usize> {
    if i + 3 > segment.len() || segment[i] != 'e' || segment[i + 1] != '2' || segment[i + 2] != 'e'
    {
        return None;
    }
    if i > 0 && is_ident_char(segment[i - 1]) {
        return None;
    }
    let mut after = i + 3;
    while after < segment.len() && (segment[after] == ' ' || segment[after] == '\t') {
        after += 1;
    }
    if after < segment.len() && segment[after] == ':' {
        Some(after + 1)
    } else {
        None
    }
}

pub fn parse_spec_refs(spec: &str) -> Vec<SpecRef> {
    let mut refs = Vec::new();
    for (index, line) in spec.lines().enumerate() {
        let visible = without_parenthesised_asides(line);
        let Some(dash) = visible.find('—') else {
            continue;
        };
        let segment: Vec<char> = visible[dash + '—'.len_utf8()..].chars().collect();
        let mut path: Option<String> = None;
        let mut i = 0;
        while i < segment.len() {
            if segment[i] == '`' {
                let mut end = i + 1;
                while end < segment.len() && segment[end] != '`' {
                    end += 1;
                }
                if end >= segment.len() {
                    break;
                }
                let token: String = segment[i + 1..end].iter().collect();
                let mut after = end + 1;
                while after < segment.len() && (segment[after] == ' ' || segment[after] == '\t') {
                    after += 1;
                }
                if after < segment.len() && segment[after] == ':' {
                    if token.contains('/') {
                        path = Some(token);
                    } else if token == "e2e" {
                        path = Some("e2e/tests/api.rs".to_string());
                    } else if let Some(current) = &path {
                        refs.push(SpecRef {
                            line: (index + 1) as u32,
                            path: current.clone(),
                            name: token,
                        });
                    }
                    i = after + 1;
                } else {
                    if let Some(current) = &path {
                        refs.push(SpecRef {
                            line: (index + 1) as u32,
                            path: current.clone(),
                            name: token,
                        });
                    }
                    i = end + 1;
                }
                continue;
            }
            if let Some(after) = bare_e2e_end(&segment, i) {
                path = Some("e2e/tests/api.rs".to_string());
                i = after;
                continue;
            }
            i += 1;
        }
    }
    refs
}

fn is_test_attribute(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with("#[") {
        return false;
    }
    let inner = trimmed[2..].trim_start();
    ["test", "tokio::test"].iter().any(|marker| {
        inner
            .strip_prefix(marker)
            .is_some_and(|rest| rest.is_empty() || !is_ident_char(rest.chars().next().unwrap()))
    })
}

fn fn_name_in(line: &str) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i + 1 < chars.len() {
        if chars[i] == 'f' && chars[i + 1] == 'n' && (i == 0 || !is_ident_char(chars[i - 1])) {
            let mut name_start = i + 2;
            while name_start < chars.len()
                && (chars[name_start] == ' ' || chars[name_start] == '\t')
            {
                name_start += 1;
            }
            if name_start == i + 2 {
                i += 1;
                continue;
            }
            let mut name_end = name_start;
            while name_end < chars.len()
                && (chars[name_end].is_alphanumeric() || chars[name_end] == '_')
            {
                name_end += 1;
            }
            if name_end > name_start {
                return Some(chars[name_start..name_end].iter().collect());
            }
        }
        i += 1;
    }
    None
}

pub fn rust_test_names(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut armed = false;
    for line in source.lines() {
        if is_test_attribute(line) {
            armed = true;
            if let Some(bracket) = line.find(']')
                && let Some(name) = fn_name_in(&line[bracket + 1..])
            {
                names.push(name);
                armed = false;
            }
            continue;
        }
        if !armed {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(name) = fn_name_in(line) {
            names.push(name);
        }
        armed = false;
    }
    names
}

fn word_at(chars: &[char], i: usize, word: &str) -> bool {
    let wanted: Vec<char> = word.chars().collect();
    if i + wanted.len() > chars.len() || chars[i..i + wanted.len()] != wanted[..] {
        return false;
    }
    if i > 0 && is_ident_char(chars[i - 1]) {
        return false;
    }
    true
}

pub fn vitest_test_names(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut names = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let call = if word_at(&chars, i, "it") {
            2
        } else if word_at(&chars, i, "test") {
            4
        } else {
            0
        };
        if call == 0 {
            i += 1;
            continue;
        }
        let mut k = i + call;
        while k < chars.len() && (chars[k] == ' ' || chars[k] == '\t' || chars[k] == '\n') {
            k += 1;
        }
        if k < chars.len() && chars[k] == '(' {
            k += 1;
            while k < chars.len() && (chars[k] == ' ' || chars[k] == '\t' || chars[k] == '\n') {
                k += 1;
            }
            if k < chars.len() && (chars[k] == '\'' || chars[k] == '"') {
                let quote = chars[k];
                k += 1;
                let mut literal = String::new();
                let mut closed = false;
                while k < chars.len() && chars[k] != '\n' {
                    if chars[k] == '\\' && k + 1 < chars.len() {
                        literal.push(chars[k + 1]);
                        k += 2;
                        continue;
                    }
                    if chars[k] == quote {
                        closed = true;
                        k += 1;
                        break;
                    }
                    literal.push(chars[k]);
                    k += 1;
                }
                if closed {
                    names.push(literal);
                }
                i = k;
                continue;
            }
        }
        i += 1;
    }
    names
}

fn defines_rust_fn(source: &str, name: &str) -> bool {
    source
        .lines()
        .any(|line| fn_name_in(line).is_some_and(|found| found == name))
}

pub fn check_spec(spec: &str, load: &dyn Fn(&str) -> Option<String>) -> Vec<MissingRef> {
    let mut missing = Vec::new();
    for cited in parse_spec_refs(spec) {
        let present = match load(&cited.path) {
            Some(source) if cited.path.ends_with(".rs") => defines_rust_fn(&source, &cited.name),
            Some(source) => vitest_test_names(&source)
                .iter()
                .any(|found| found == &cited.name),
            None => false,
        };
        if !present {
            missing.push(MissingRef {
                line: cited.line,
                path: cited.path,
                name: cited.name,
            });
        }
    }
    missing
}
