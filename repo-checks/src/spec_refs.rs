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
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
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

pub fn check_spec(spec: &str, load: &dyn Fn(&str) -> Option<String>) -> Vec<MissingRef> {
    let mut missing = Vec::new();
    for cited in parse_spec_refs(spec) {
        let present = match load(&cited.path) {
            Some(source) if cited.path.ends_with(".rs") => rust_test_names(&source)
                .iter()
                .any(|found| found == &cited.name),
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

#[cfg(test)]
mod tests {
    use super::{check_spec, parse_spec_refs, rust_test_names, vitest_test_names};

    #[test]
    fn parses_semicolon_separated_groups_without_merging_paths() {
        let refs = parse_spec_refs(
            "— `server/src/lib.rs`: `a_test`（note）; `ui/src/api.test.ts`: `some ui name`",
        );
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].path, "server/src/lib.rs");
        assert_eq!(refs[0].name, "a_test");
        assert_eq!(refs[1].path, "ui/src/api.test.ts");
        assert_eq!(refs[1].name, "some ui name");
    }

    #[test]
    fn parses_comma_separated_names_for_one_path() {
        let refs = parse_spec_refs("— `shared/src/feed.rs`: `first_test`, `second_test`");
        assert_eq!(refs.len(), 2);
        assert!(refs.iter().all(|r| r.path == "shared/src/feed.rs"));
    }

    #[test]
    fn excludes_parenthesised_mentions_and_bare_e2e_ok() {
        // Bare `e2e:` (no backticks, as in docs/spec.md) resolves to the e2e file,
        // and `（...）` asides contribute no refs.
        let refs = parse_spec_refs("— e2e: `a_e2e_ok`（note `not_a_test`）");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, "e2e/tests/api.rs");
        assert_eq!(refs[0].name, "a_e2e_ok");
    }

    #[test]
    fn finds_test_fns_regardless_of_attribute_and_ignores_helpers() {
        // `#[test]` / `#[tokio::test]` are both plain fns; the plain helper `fresh_db` must not match.
        let names = rust_test_names(
            "#[tokio::test]\nasync fn e2e_ok() {}\n#[test]\nfn unit_ok() {}\nasync fn fresh_db() {}",
        );
        assert!(names.contains(&"e2e_ok".to_string()));
        assert!(names.contains(&"unit_ok".to_string()));
        assert!(!names.contains(&"fresh_db".to_string()));
    }

    #[test]
    fn finds_vitest_names_in_both_quote_styles() {
        let names =
            vitest_test_names("it('single quoted', () => {});\ntest(\"double quoted\", () => {});");
        assert!(names.contains(&"single quoted".to_string()));
        assert!(names.contains(&"double quoted".to_string()));
    }

    #[test]
    fn check_reports_missing_tests_with_spec_line() {
        let missing = check_spec("line one\n— `server/src/lib.rs`: `gone_test`\n", &|_| {
            Some("fn other_test() {}".to_string())
        });
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].line, 2);
        assert_eq!(missing[0].path, "server/src/lib.rs");
        assert_eq!(missing[0].name, "gone_test");
    }

    #[test]
    fn check_accepts_present_tests() {
        let missing = check_spec("— `server/src/lib.rs`: `here_test`\n", &|_| {
            Some("#[test]\nfn here_test() {}".to_string())
        });
        assert!(missing.is_empty());
    }

    #[test]
    fn ignores_pathless_name_colon_pairs_outside_citations() {
        let refs = parse_spec_refs("— `server/src/lib.rs`: `real_test`, note: `not_a_test`");
        assert_eq!(refs.len(), 2);
        assert!(refs.iter().all(|r| r.path == "server/src/lib.rs"));
        assert!(refs.iter().any(|r| r.name == "real_test"));
        assert!(refs.iter().any(|r| r.name == "not_a_test"));
    }

    #[test]
    fn reads_backticked_e2e_path_as_the_e2e_file() {
        let refs = parse_spec_refs("— `e2e`: `an_e2e_ok`");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, "e2e/tests/api.rs");
        assert_eq!(refs[0].name, "an_e2e_ok");
    }

    #[test]
    fn reads_space_before_colon_in_bare_e2e_prefix() {
        let refs = parse_spec_refs("— e2e : `spaced_e2e_ok`");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, "e2e/tests/api.rs");
        assert_eq!(refs[0].name, "spaced_e2e_ok");
    }

    #[test]
    fn skips_missing_files_as_missing_refs() {
        let missing = check_spec("— `server/src/lib.rs`: `gone_test`\n", &|_| None);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].path, "server/src/lib.rs");
        assert_eq!(missing[0].name, "gone_test");
    }

    #[test]
    fn skips_vitest_entries_without_string_literals() {
        let names = vitest_test_names("it(variable, () => {});");
        assert!(names.is_empty());
    }

    #[test]
    fn picks_same_line_fn_after_test_attribute() {
        let names = rust_test_names("#[test] fn same_line_ok() {}");
        assert!(names.contains(&"same_line_ok".to_string()));
    }

    #[test]
    fn ignores_citation_shaped_text_before_the_dash() {
        let refs = parse_spec_refs("see `server/src/lib.rs`: `pre_test` — text");
        assert!(refs.is_empty());
    }

    #[test]
    fn ignores_plain_fn_without_attribute() {
        let names = rust_test_names("fn plain_helper() {}");
        assert!(names.is_empty());
    }

    #[test]
    fn reads_escaped_quote_inside_vitest_name() {
        let names = vitest_test_names("it('don\\'t stop', () => {});");
        assert!(names.contains(&"don't stop".to_string()));
    }

    #[test]
    fn skips_word_prefixed_fake_fn() {
        let names = rust_test_names("#[test]\nmyfn not_a_test() {}");
        assert!(names.is_empty());
    }

    #[test]
    fn ignores_non_test_attributes() {
        let names = rust_test_names("#[allow(dead_code)]\nfn allowed_helper() {}");
        assert!(names.is_empty());
    }

    #[test]
    fn ignores_identifier_prefixed_bare_e2e() {
        let refs = parse_spec_refs("— xe2e: `x_ok`");
        assert!(refs.is_empty());
    }

    #[test]
    fn ignores_vitest_calls_prefixed_by_identifier() {
        let names = vitest_test_names("denyit('x_ok', () => {});\nlatest('y_ok', () => {});");
        assert!(names.is_empty());
    }

    #[test]
    fn ignores_test_prefixed_attributes() {
        let names = rust_test_names("#[testing]\nfn testing_helper() {}");
        assert!(names.is_empty());
    }

    #[test]
    fn ignores_vitest_calls_followed_by_identifier() {
        let names = vitest_test_names("its('x_ok', () => {});\ntests('y_ok', () => {});");
        assert!(names.is_empty());
    }

    #[test]
    fn check_rejects_present_fn_without_test_attribute() {
        // Catches check_spec accepting a plain fn without #[test] / #[tokio::test] as present.
        let missing = check_spec("— `server/src/lib.rs`: `renamed_test`\n", &|_| {
            Some("fn renamed_test() {}".to_string())
        });
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].path, "server/src/lib.rs");
        assert_eq!(missing[0].name, "renamed_test");
    }

    #[test]
    fn records_backticked_name_followed_by_colon_under_current_path() {
        // Catches parse_spec_refs dropping a `` `label`: `` pair instead of recording it
        // under the current path.
        let refs =
            parse_spec_refs("— `server/src/lib.rs`: `real_test`, `pending_label` : trailing");
        assert_eq!(refs.len(), 2);
        assert!(refs.iter().all(|r| r.path == "server/src/lib.rs"));
        assert!(refs.iter().any(|r| r.name == "real_test"));
        assert!(refs.iter().any(|r| r.name == "pending_label"));
    }

    #[test]
    fn keeps_armed_across_attribute_line_before_fn() {
        // Catches rust_test_names disarming on an attribute line between #[test] and fn.
        let names = rust_test_names("#[test]\n#[allow(dead_code)]\nfn through_attr_ok() {}");
        assert!(names.contains(&"through_attr_ok".to_string()));
    }

    #[test]
    fn keeps_armed_across_comment_line_before_fn() {
        // Catches rust_test_names disarming on a comment line between #[test] and fn.
        let names = rust_test_names("#[test]\n// explains the fixture\nfn comment_ok() {}");
        assert!(names.contains(&"comment_ok".to_string()));
    }

    #[test]
    fn keeps_armed_across_blank_line_before_fn() {
        // Catches rust_test_names disarming on a blank line between #[test] and fn.
        let names = rust_test_names("#[test]\n\nfn blank_ok() {}");
        assert!(names.contains(&"blank_ok".to_string()));
    }

    #[test]
    fn check_reports_missing_vitest_names() {
        // Catches check_spec treating every vitest citation as present.
        let missing = check_spec("— `ui/src/api.test.ts`: `absent`\n", &|_| {
            Some("it('present', () => {});".to_string())
        });
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].path, "ui/src/api.test.ts");
        assert_eq!(missing[0].name, "absent");
    }

    #[test]
    fn ignores_vitest_call_prefixed_by_underscore() {
        // Catches is_ident_char not treating '_' as an identifier char.
        let names = vitest_test_names("my_test('should not be picked up', () => {});");
        assert!(names.is_empty());
    }
}
