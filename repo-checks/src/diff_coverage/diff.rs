use std::collections::{BTreeMap, BTreeSet};

/// New-side line numbers each file gained, keyed by repo-relative path.
pub type AddedLines = BTreeMap<String, BTreeSet<u32>>;

struct Hunk {
    old_left: u32,
    new_left: u32,
    new_line: u32,
}

/// Hunk bodies are consumed by the counts in their headers rather than by line
/// prefixes: an added line whose content starts with `++` reads as `+++…`, and
/// a removed `-- comment` as `--- …`, exactly like file headers.
pub fn parse_diff(diff: &str) -> Result<AddedLines, String> {
    let mut added = AddedLines::new();
    let mut path: Option<String> = None;
    let mut hunk: Option<Hunk> = None;

    for (i, line) in diff.lines().enumerate() {
        let at = i + 1;
        if let Some(h) = hunk.as_mut() {
            match line.chars().next() {
                // Some tools strip the trailing space of an empty context line.
                Some(' ') | None => {
                    h.old_left = dec(h.old_left, at)?;
                    h.new_left = dec(h.new_left, at)?;
                    h.new_line += 1;
                }
                Some('-') => h.old_left = dec(h.old_left, at)?,
                Some('+') => {
                    h.new_left = dec(h.new_left, at)?;
                    if let Some(p) = &path {
                        added.entry(p.clone()).or_default().insert(h.new_line);
                    }
                    h.new_line += 1;
                }
                Some('\\') => {}
                Some(_) => return Err(format!("diff line {at}: unexpected line inside a hunk")),
            }
            if h.old_left == 0 && h.new_left == 0 {
                hunk = None;
            }
            continue;
        }

        if line.starts_with("diff --git ") {
            path = None;
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            path = parse_new_path(rest, at)?;
        } else if let Some(rest) = line.strip_prefix("@@ ") {
            let (old_left, new_start, new_left) = parse_hunk_header(rest)
                .ok_or_else(|| format!("diff line {at}: malformed hunk header"))?;
            if old_left > 0 || new_left > 0 {
                hunk = Some(Hunk {
                    old_left,
                    new_left,
                    new_line: new_start,
                });
            }
        } else if line.starts_with([' ', '+', '-']) && !line.starts_with("--- ") {
            return Err(format!(
                "diff line {at}: content line outside a hunk (hunk shorter than its lines?)"
            ));
        }
    }

    if hunk.is_some() {
        return Err("diff ends inside a hunk".to_string());
    }
    Ok(added)
}

fn dec(n: u32, at: usize) -> Result<u32, String> {
    n.checked_sub(1)
        .ok_or_else(|| format!("diff line {at}: hunk has more lines than its header says"))
}

fn parse_new_path(rest: &str, at: usize) -> Result<Option<String>, String> {
    let rest = rest.trim_end();
    if rest == "/dev/null" {
        return Ok(None);
    }
    match rest.strip_prefix("b/") {
        Some(p) if !p.is_empty() => Ok(Some(p.to_string())),
        // git quotes paths with unusual bytes; reading them as-is would
        // attribute lines to a file that does not exist.
        _ => Err(format!(
            "diff line {at}: unsupported file header `+++ {rest}`"
        )),
    }
}

fn parse_hunk_header(rest: &str) -> Option<(u32, u32, u32)> {
    let mut parts = rest.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    if parts.next()? != "@@" {
        return None;
    }
    let (_, old_count) = parse_range(old)?;
    let (new_start, new_count) = parse_range(new)?;
    Some((old_count, new_start, new_count))
}

fn parse_range(range: &str) -> Option<(u32, u32)> {
    match range.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((range.parse().ok()?, 1)),
    }
}
