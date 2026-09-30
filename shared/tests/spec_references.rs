//! docs/spec.md が引用するテスト名が実在することの検査 (#215)。
//!
//! チェック自体がこのファイルで、`cargo test --workspace`
//! （scripts/test.sh の一部、したがって CI）で走る。読むのは
//! リポジトリ内のファイルだけで、通信も外部コマンドも使わない。

use std::path::{Path, PathBuf};

/// spec の引用1件。line は spec 内の 1 始まりの行番号、file は
/// リポジトリ相対パス（`e2e:` 記法は e2e/tests/api.rs に読み替えたもの）。
struct SpecRef {
    line: u32,
    file: String,
    test: String,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("workspace root exists")
}

fn is_file_token(token: &str) -> bool {
    token == "e2e"
        || token.contains('/')
            && (token.ends_with(".rs") || token.ends_with(".tsx") || token.ends_with(".ts"))
}

fn map_file(token: &str) -> String {
    if token == "e2e" {
        "e2e/tests/api.rs".to_string()
    } else {
        token.to_string()
    }
}

fn strip_parens(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut full = 0u32;
    let mut half = 0u32;
    for c in line.chars() {
        if c == '（' {
            full += 1;
            continue;
        }
        if c == '）' {
            full = full.saturating_sub(1);
            continue;
        }
        if full > 0 {
            continue;
        }
        if c == '(' {
            half += 1;
            continue;
        }
        if c == ')' {
            half = half.saturating_sub(1);
            continue;
        }
        if half > 0 {
            continue;
        }
        out.push(c);
    }
    out
}

/// spec 全文から `` `file`: `test`, ... `` 形式の引用を抜き出す。
/// `— テスト無し` の行は何も出さない。補足の括弧の中の
/// バッククォート（例: `SET title = NULL`）はテスト名にしない。
fn parse_refs(spec: &str) -> Vec<SpecRef> {
    let mut refs = Vec::new();
    for (index, line) in spec.lines().enumerate() {
        let clean = strip_parens(line);
        let mut spans: Vec<(usize, usize, &str)> = Vec::new();
        let mut pos = 0;
        while pos < clean.len() {
            if clean.as_bytes()[pos] != b'`' {
                pos += 1;
                continue;
            }
            match clean[pos + 1..].find('`') {
                Some(end) => {
                    spans.push((pos, pos + 1 + end, &clean[pos + 1..pos + 1 + end]));
                    pos += end + 2;
                }
                None => break,
            }
        }
        let mut bare: Vec<usize> = Vec::new();
        for (k, _) in clean.char_indices() {
            if !clean[k..].starts_with("e2e") {
                continue;
            }
            let bytes = clean.as_bytes();
            if k > 0 && is_word_byte(bytes[k - 1]) {
                continue;
            }
            if bytes.get(k + 3).is_some_and(|b| is_word_byte(*b)) {
                continue;
            }
            if spans.iter().any(|(s, e, _)| k > *s && k < *e) {
                continue;
            }
            let mut j = k + 3;
            while matches!(bytes.get(j), Some(b' ' | b'\t')) {
                j += 1;
            }
            if matches!(bytes.get(j), Some(b':')) || clean[j..].starts_with('：') {
                bare.push(k);
            }
        }
        let mut current: Option<String> = None;
        let mut cursor = 0;
        for (s, span) in spans.iter().enumerate() {
            while cursor < bare.len() && bare[cursor] < span.0 {
                current = Some(map_file("e2e"));
                cursor += 1;
            }
            if span.2.is_empty() {
                continue;
            }
            let next = spans.get(s + 1).map(|n| n.0).unwrap_or(clean.len());
            let between = &clean[span.1 + 1..next];
            if is_file_token(span.2) {
                if between.contains(':') || between.contains('：') {
                    current = Some(map_file(span.2));
                }
                continue;
            }
            if let Some(file) = current.as_ref() {
                refs.push(SpecRef {
                    line: (index + 1) as u32,
                    file: file.clone(),
                    test: span.2.to_string(),
                });
            }
        }
    }
    refs
}

fn fn_name(rest: &str) -> Option<String> {
    let end = rest
        .char_indices()
        .find(|(_, c)| !c.is_ascii_alphanumeric() && *c != '_')
        .map(|(i, _)| i)
        .unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(rest[..end].to_string())
    }
}

fn rust_test_names(content: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut pending = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[test") || trimmed.starts_with("#[tokio::test") {
            match trimmed.find("fn ") {
                Some(pos) => {
                    if let Some(name) = fn_name(&trimmed[pos + 3..]) {
                        names.push(name);
                    }
                    pending = false;
                }
                None => pending = true,
            }
            continue;
        }
        if !pending {
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }
        pending = false;
        if let Some(pos) = trimmed.find("fn ")
            && let Some(name) = fn_name(&trimmed[pos + 3..])
        {
            names.push(name);
        }
    }
    names
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn starts_word_at(content: &str, i: usize, word: &str) -> bool {
    let bytes = content.as_bytes();
    if !content[i..].starts_with(word) {
        return false;
    }
    if i > 0 && is_word_byte(bytes[i - 1]) {
        return false;
    }
    if bytes.get(i + word.len()).is_some_and(|b| is_word_byte(*b)) {
        return false;
    }
    true
}

fn skip_ws(content: &str, mut j: usize) -> usize {
    let bytes = content.as_bytes();
    while matches!(bytes.get(j), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        j += 1;
    }
    j
}

fn skip_balanced(content: &str, start: usize) -> usize {
    let bytes = content.as_bytes();
    let open = bytes[start];
    let close = if open == b'(' { b')' } else { b']' };
    let mut j = start + 1;
    let mut depth = 1;
    while j < bytes.len() && depth > 0 {
        match bytes[j] {
            b'\\' => j += 1,
            q @ (b'\'' | b'"' | b'`') => {
                j += 1;
                while j < bytes.len() && bytes[j] != q {
                    if bytes[j] == b'\\' {
                        j += 1;
                    }
                    j += 1;
                }
            }
            b if b == open => depth += 1,
            b if b == close => depth -= 1,
            _ => {}
        }
        j += 1;
    }
    j
}

fn vitest_test_names(content: &str) -> Vec<String> {
    let bytes = content.as_bytes();
    let mut names = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !content.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let keyword = if starts_word_at(content, i, "it") {
            Some(2)
        } else if starts_word_at(content, i, "test") {
            Some(4)
        } else {
            None
        };
        let Some(len) = keyword else {
            i += 1;
            continue;
        };
        let mut j = i + len;
        loop {
            j = skip_ws(content, j);
            if bytes.get(j) != Some(&b'.') {
                break;
            }
            j += 1;
            while bytes
                .get(j)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'$')
            {
                j += 1;
            }
            while matches!(bytes.get(j), Some(b'(' | b'[')) {
                j = skip_balanced(content, j);
            }
        }
        j = skip_ws(content, j);
        if bytes.get(j) != Some(&b'(') {
            i += 1;
            continue;
        }
        j = skip_ws(content, j + 1);
        match bytes.get(j) {
            Some(b'\'' | b'"' | b'`') => {}
            _ => {
                i += 1;
                continue;
            }
        }
        let quote = bytes[j];
        j += 1;
        let start = j;
        while j < bytes.len() && bytes[j] != quote {
            if bytes[j] == b'\\' {
                j += 1;
            }
            j += 1;
        }
        names.push(content[start..j.min(bytes.len())].to_string());
        i = j + 1;
    }
    names
}

/// 引用先ファイルの中身から実在するテスト名を抜き出す。
/// Rust は `#[test]` / `#[tokio::test]` の付いた関数名、
/// vitest は `it` / `test` の第一引数。
fn existing_test_names(file: &Path, content: &str) -> Vec<String> {
    if file.extension().is_some_and(|ext| ext == "rs") {
        rust_test_names(content)
    } else {
        vitest_test_names(content)
    }
}

/// spec 全文を検証し、引用先に存在しない引用だけを
/// `spec <行>: <file> の <test> が見つからない` 形式の行で返す。
/// 空なら spec の引用はすべて実在する。
fn verify_refs(root: &Path, spec: &str) -> Vec<String> {
    let refs = parse_refs(spec);
    let mut files: Vec<&str> = Vec::new();
    for r in &refs {
        if !files.iter().any(|f| **f == r.file) {
            files.push(&r.file);
        }
    }
    let mut existing: Vec<Vec<String>> = Vec::new();
    for file in &files {
        let content = std::fs::read_to_string(root.join(file)).unwrap_or_default();
        existing.push(existing_test_names(Path::new(file), &content));
    }
    refs.iter()
        .filter(|r| {
            let pos = files
                .iter()
                .position(|f| *f == r.file.as_str())
                .expect("cited file is collected");
            !existing[pos].contains(&r.test)
        })
        .map(|r| format!("spec {}: {} の {} が見つからない", r.line, r.file, r.test))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 削除・改名で引用だけ残ったテストを捕まえる。
    #[test]
    fn current_spec_cites_only_existing_tests() {
        let root = workspace_root();
        let spec =
            std::fs::read_to_string(root.join("docs/spec.md")).expect("docs/spec.md is readable");
        let missing = verify_refs(&root, &spec);
        assert!(
            missing.is_empty(),
            "docs/spec.md に存在しないテストの引用がある:\n{}",
            missing.join("\n")
        );
    }

    // 引用と無関係なバッククォート（テスト無し行、補足の括弧内の SQL）を
    // テスト名として拾う実装を捕まえる。書式ゆれ（`,` / `;` / 全角読点の
    // 複数ファイル併記、`e2e:` の省略形）もこの fixture で固定する。
    #[test]
    fn missing_reference_reports_spec_line_file_and_test_name() {
        let root = workspace_root();
        let spec = concat!(
            "- 1 行目 — `shared/src/db.rs`: `appends_when_no_query`, `no_such_test_does_not_exist`\n",
            "- 2 行目 — e2e: `adding_a_feed_rejects_urls_the_fetcher_must_not_reach`\n",
            "- 3 行目 — テスト無し\n",
            "- 4 行目 — `shared/src/db.rs`: `appends_when_no_query`; `shared/src/ssrf.rs`: `accepts_https_url_with_public_host`\n",
            "- 5 行目 — `server/src/lib.rs`: `only_failed_adds_of_new_feeds_roll_back`（`SET title = NULL` への変異で落ちることを確認済み）\n",
        );
        let missing = verify_refs(&root, spec);
        assert_eq!(
            missing.len(),
            1,
            "存在しない引用は 1 件のはず:\n{missing:?}"
        );
        let line = &missing[0];
        assert!(
            line.contains('1')
                && line.contains("shared/src/db.rs")
                && line.contains("no_such_test_does_not_exist"),
            "spec の行・ファイル・見つからなかったテスト名を出すはず: {line}"
        );
    }
}
