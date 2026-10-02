use std::collections::BTreeMap;
use std::path::Path;

/// Execution count of every instrumented line, keyed by repo-relative path.
pub type LineHits = BTreeMap<String, BTreeMap<u32, u64>>;

/// Absolute `SF:` paths are taken relative to `root`, and ones outside it
/// (dependencies) are dropped; relative ones are taken relative to
/// `relative_base` (a repo-relative directory, empty for the root). Counts for
/// the same line add up, since one source file compiled into several test
/// binaries gets a record from each. A record with no `DA:` line (a file of
/// only types) still marks its file as reported.
pub fn parse_lcov(
    lcov: &str,
    root: &Path,
    relative_base: &str,
    hits: &mut LineHits,
) -> Result<(), String> {
    // None: no record open. Some(None): a record outside the repository.
    let mut file: Option<Option<String>> = None;

    for (i, line) in lcov.lines().enumerate() {
        let at = i + 1;
        let line = line.trim_end();
        if let Some(sf) = line.strip_prefix("SF:") {
            let path = repo_path(sf, root, relative_base);
            if let Some(p) = &path {
                hits.entry(p.clone()).or_default();
            }
            file = Some(path);
        } else if line == "end_of_record" {
            file = None;
        } else if let Some(da) = line.strip_prefix("DA:") {
            let Some(current) = &file else {
                return Err(format!("lcov line {at}: DA outside a record"));
            };
            let mut fields = da.split(',');
            let (Some(n), Some(count)) = (
                fields.next().and_then(|s| s.parse::<u32>().ok()),
                fields.next().and_then(|s| s.parse::<u64>().ok()),
            ) else {
                return Err(format!("lcov line {at}: malformed DA record"));
            };
            if let Some(path) = current {
                *hits.entry(path.clone()).or_default().entry(n).or_default() += count;
            }
        }
    }
    Ok(())
}

fn repo_path(sf: &str, root: &Path, relative_base: &str) -> Option<String> {
    let sf = Path::new(sf);
    let relative = if sf.is_absolute() {
        sf.strip_prefix(root).ok()?.to_path_buf()
    } else {
        Path::new(relative_base).join(sf)
    };
    let mut parts = Vec::new();
    for c in relative.components() {
        match c {
            std::path::Component::Normal(p) => parts.push(p.to_str()?.to_string()),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}
