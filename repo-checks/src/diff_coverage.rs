use std::collections::{HashMap, HashSet};

#[derive(Debug, PartialEq, Eq)]
pub struct ChangedLine {
    pub path: String,
    pub line: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub struct UncoveredLine {
    pub path: String,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageClass {
    Untested,
    E2eOnly,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ClassifiedLine {
    pub path: String,
    pub line: u32,
    pub class: CoverageClass,
}

fn strip_diff_prefix(path: &str) -> &str {
    path.strip_prefix("a/").unwrap_or(path).trim()
}

fn strip_b_prefix(path: &str) -> &str {
    path.strip_prefix("b/").unwrap_or(path).trim()
}

fn parse_hunk_new_start(header: &str) -> Option<u32> {
    let plus = header.find('+')?;
    let rest = &header[plus + 1..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse::<u32>().ok()
}

pub fn parse_changed_lines(diff: &str) -> Vec<ChangedLine> {
    let mut changed = Vec::new();
    let mut path = String::new();
    let mut new_line: u32 = 0;
    let mut in_hunk = false;
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            let trimmed = rest.trim();
            if trimmed == "/dev/null" {
                path.clear();
                in_hunk = false;
            } else {
                path = strip_b_prefix(trimmed).to_string();
            }
            continue;
        }
        if line.starts_with("--- ") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("@@ ") {
            if path.is_empty() {
                in_hunk = false;
                continue;
            }
            match parse_hunk_new_start(rest) {
                Some(start) => {
                    new_line = start;
                    in_hunk = true;
                }
                None => in_hunk = false,
            }
            continue;
        }
        if !in_hunk || path.is_empty() {
            continue;
        }
        if let Some(added) = line.strip_prefix('+') {
            if line.starts_with("+++") {
                continue;
            }
            let _ = added;
            changed.push(ChangedLine {
                path: path.clone(),
                line: new_line,
            });
            new_line += 1;
        } else if line.starts_with('-') {
            continue;
        } else if line.starts_with(' ') || line.is_empty() {
            new_line += 1;
        } else if line.starts_with('\\') {
            continue;
        } else {
            in_hunk = false;
        }
    }
    changed
}

pub fn parse_uncovered_lines(lcov: &str) -> Vec<UncoveredLine> {
    let mut uncovered = Vec::new();
    let mut path = String::new();
    let mut have_sf = false;
    for line in lcov.lines() {
        if let Some(sf) = line.strip_prefix("SF:") {
            path = sf.trim().to_string();
            have_sf = true;
            continue;
        }
        if line == "end_of_record" {
            have_sf = false;
            path.clear();
            continue;
        }
        if !have_sf {
            continue;
        }
        if let Some(rest) = line.strip_prefix("DA:") {
            let mut parts = rest.split(',');
            let line_no: Option<u32> = parts.next().and_then(|s| s.trim().parse().ok());
            let count: Option<u64> = parts
                .next()
                .and_then(|s| s.split(';').next().and_then(|n| n.trim().parse().ok()));
            if let (Some(line_no), Some(0)) = (line_no, count) {
                uncovered.push(UncoveredLine {
                    path: path.clone(),
                    line: line_no,
                });
            }
        }
    }
    uncovered
}

fn normalize_lcov_path(sf: &str) -> &str {
    sf.trim()
}

fn strip_leading_slash(path: &str) -> &str {
    path.strip_prefix('/').unwrap_or(path)
}

pub fn lcov_path_matches(lcov_sf: &str, diff_path: &str) -> bool {
    let sf = normalize_lcov_path(lcov_sf);
    let diff = strip_diff_prefix(diff_path);
    if sf == diff {
        return true;
    }
    if sf.starts_with('/') && strip_leading_slash(sf).ends_with(diff) {
        let rest = strip_leading_slash(sf);
        if rest == diff || rest.ends_with(&format!("/{diff}")) {
            return true;
        }
    }
    if let Some(ui_relative) = diff.strip_prefix("ui/") {
        return sf == ui_relative;
    }
    false
}

#[derive(Debug, Default)]
struct ImportMap {
    single: HashMap<String, String>,
    glob: Vec<String>,
}

fn path_to_string(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn collect_imports(file: &syn::File) -> ImportMap {
    let mut map = ImportMap::default();
    for item in &file.items {
        if let syn::Item::Use(u) = item {
            let mut local = Vec::new();
            record_use_tree_inner(&u.tree, &mut local, &mut map);
        }
    }
    map
}

fn record_use_tree_inner(tree: &syn::UseTree, prefix: &mut Vec<String>, map: &mut ImportMap) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            record_use_tree_inner(&p.tree, prefix, map);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            let mut full = prefix.clone();
            full.push(n.ident.to_string());
            map.single.insert(n.ident.to_string(), full.join("::"));
        }
        syn::UseTree::Rename(r) => {
            let mut full = prefix.clone();
            full.push(r.ident.to_string());
            map.single.insert(r.rename.to_string(), full.join("::"));
        }
        syn::UseTree::Glob(_) => {
            map.glob.push(prefix.join("::"));
        }
        syn::UseTree::Group(g) => {
            for item in &g.items {
                record_use_tree_inner(item, prefix, map);
            }
        }
    }
}

fn resolve_expr_path(path: &syn::Path, imports: &ImportMap) -> Option<String> {
    if path.leading_colon.is_some() {
        return Some(path_to_string(path));
    }
    let first = path.segments.first()?.ident.to_string();
    if first == "spin_sdk" || first == "crate" || first == "self" || first == "super" {
        return Some(path_to_string(path));
    }
    if let Some(mapped) = imports.single.get(&first) {
        let rest: Vec<String> = path
            .segments
            .iter()
            .skip(1)
            .map(|s| s.ident.to_string())
            .collect();
        if rest.is_empty() {
            return Some(mapped.clone());
        }
        return Some(format!("{mapped}::{}", rest.join("::")));
    }
    for glob in &imports.glob {
        if glob == "spin_sdk" || glob.starts_with("spin_sdk::") {
            return Some(format!("{glob}::{}", path_to_string(path)));
        }
    }
    None
}

fn resolve_type_path(path: &syn::Path, imports: &ImportMap) -> Option<String> {
    resolve_expr_path(path, imports)
}

fn type_is_spin_request(ty: &syn::Type, imports: &ImportMap) -> bool {
    match ty {
        syn::Type::Path(p) => {
            if p.qself.is_some() {
                return false;
            }
            resolve_type_path(&p.path, imports).as_deref() == Some("spin_sdk::http::Request")
        }
        syn::Type::Reference(r) => type_is_spin_request(&r.elem, imports),
        _ => false,
    }
}

fn attr_is_http_service(attr: &syn::Attribute, imports: &ImportMap) -> bool {
    if !matches!(attr.style, syn::AttrStyle::Outer) {
        return false;
    }
    let segments: Vec<String> = attr
        .path()
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect();
    if segments == ["http_service"] {
        if let Some(mapped) = imports.single.get("http_service") {
            return mapped == "spin_sdk" || mapped == "spin_sdk::http_service";
        }
        for glob in &imports.glob {
            if glob == "spin_sdk" {
                return true;
            }
        }
        return false;
    }
    let full = segments.join("::");
    full == "spin_sdk::http_service"
}

fn call_is_spin_runtime(path: &syn::Path, imports: &ImportMap) -> bool {
    let resolved = match resolve_expr_path(path, imports) {
        Some(r) => r,
        None => return false,
    };
    if resolved == "spin_sdk::variables::get" {
        return true;
    }
    if resolved == "spin_sdk::http::send" {
        return true;
    }
    if resolved.starts_with("spin_sdk::pg::") || resolved.starts_with("spin_sdk::pg ::") {
        return true;
    }
    false
}

struct SpinCallVisitor<'a> {
    imports: &'a ImportMap,
    non_connection_params: HashSet<String>,
    found: bool,
}

impl<'ast, 'a> syn::visit::Visit<'ast> for SpinCallVisitor<'a> {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if !self.found
            && let syn::Expr::Path(p) = node.func.as_ref()
            && p.qself.is_none()
            && call_is_spin_runtime(&p.path, self.imports)
        {
            self.found = true;
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_item_fn(&mut self, _node: &'ast syn::ItemFn) {}

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        // Receiver types need resolution, so declared parameter types decide.
        // Spin's query/execute always take arguments, so an argument-less
        // call is not one.
        if !self.found {
            let name = node.method.to_string();
            if (name == "query" || name == "execute")
                && !node.args.is_empty()
                && pg_imported(self.imports)
                && !receiver_is_non_connection_param(&node.receiver, &self.non_connection_params)
            {
                self.found = true;
            }
        }
        syn::visit::visit_expr_method_call(self, node);
    }
}

fn receiver_is_non_connection_param(
    receiver: &syn::Expr,
    non_connection: &HashSet<String>,
) -> bool {
    if let syn::Expr::Path(p) = receiver
        && p.qself.is_none()
        && p.path.segments.len() == 1
    {
        return non_connection.contains(&p.path.segments[0].ident.to_string());
    }
    false
}

fn type_is_spin_connection(ty: &syn::Type, imports: &ImportMap) -> bool {
    match ty {
        syn::Type::Path(p) => {
            if p.qself.is_some() {
                return false;
            }
            resolve_type_path(&p.path, imports).as_deref() == Some("spin_sdk::pg::Connection")
        }
        syn::Type::Reference(r) => type_is_spin_connection(&r.elem, imports),
        _ => false,
    }
}

fn collect_non_connection_params(func: &syn::ItemFn, imports: &ImportMap) -> HashSet<String> {
    let mut out = HashSet::new();
    for arg in &func.sig.inputs {
        if let syn::FnArg::Typed(t) = arg
            && !type_is_spin_connection(&t.ty, imports)
            && let syn::Pat::Ident(name) = t.pat.as_ref()
        {
            out.insert(name.ident.to_string());
        }
    }
    out
}

fn pg_imported(imports: &ImportMap) -> bool {
    imports
        .single
        .values()
        .any(|mapped| mapped == "spin_sdk::pg" || mapped.starts_with("spin_sdk::pg::"))
        || imports
            .glob
            .iter()
            .any(|g| g == "spin_sdk" || g == "spin_sdk::pg")
}

struct FnInfo {
    start: usize,
    end: usize,
    e2e_only: bool,
}

fn fn_body_range(func: &syn::ItemFn) -> Option<(usize, usize)> {
    let brace = func.block.brace_token.span;
    let start = brace.open().start().line;
    let end = brace.close().start().line;
    if end < start || start == 0 {
        return None;
    }
    Some((start, end))
}

fn fn_is_e2e_only(func: &syn::ItemFn, imports: &ImportMap) -> bool {
    if func.attrs.iter().any(|a| attr_is_http_service(a, imports)) {
        return true;
    }
    if func.sig.inputs.iter().any(|arg| match arg {
        syn::FnArg::Typed(t) => type_is_spin_request(&t.ty, imports),
        syn::FnArg::Receiver(_) => false,
    }) {
        return true;
    }
    let non_connection_params = collect_non_connection_params(func, imports);
    let mut visitor = SpinCallVisitor {
        imports,
        non_connection_params,
        found: false,
    };
    syn::visit::visit_block(&mut visitor, &func.block);
    visitor.found
}

fn collect_fn_infos(source: &str) -> Vec<FnInfo> {
    let file: syn::File = match syn::parse_str(source) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let imports = collect_imports(&file);
    let mut infos = Vec::new();
    collect_fn_items(&file.items, &imports, &mut infos);
    infos
}

fn collect_fn_items(items: &[syn::Item], imports: &ImportMap, out: &mut Vec<FnInfo>) {
    for item in items {
        match item {
            syn::Item::Fn(f) => {
                if let Some((start, end)) = fn_body_range(f) {
                    out.push(FnInfo {
                        start,
                        end,
                        e2e_only: fn_is_e2e_only(f, imports),
                    });
                }
                collect_fn_items_in_block(&f.block, imports, out);
            }
            syn::Item::Mod(m) => {
                if let Some((_, content)) = &m.content {
                    collect_fn_items(content, imports, out);
                }
            }
            syn::Item::Impl(i) => {
                for impl_item in &i.items {
                    if let syn::ImplItem::Fn(m) = impl_item {
                        let converted = syn::ItemFn {
                            attrs: m.attrs.clone(),
                            vis: m.vis.clone(),
                            sig: m.sig.clone(),
                            block: Box::new(m.block.clone()),
                        };
                        if let Some((start, end)) = fn_body_range(&converted) {
                            out.push(FnInfo {
                                start,
                                end,
                                e2e_only: fn_is_e2e_only(&converted, imports),
                            });
                        }
                        collect_fn_items_in_block(&m.block, imports, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_fn_items_in_block(block: &syn::Block, imports: &ImportMap, out: &mut Vec<FnInfo>) {
    struct BlockFnCollector<'a> {
        imports: &'a ImportMap,
        out: &'a mut Vec<FnInfo>,
    }
    impl<'ast, 'a> syn::visit::Visit<'ast> for BlockFnCollector<'a> {
        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            if let Some((start, end)) = fn_body_range(node) {
                self.out.push(FnInfo {
                    start,
                    end,
                    e2e_only: fn_is_e2e_only(node, self.imports),
                });
            }
            syn::visit::visit_item_fn(self, node);
        }
    }
    let mut collector = BlockFnCollector { imports, out };
    for stmt in &block.stmts {
        syn::visit::visit_stmt(&mut collector, stmt);
    }
}

pub fn classify_line(source: &str, line: u32) -> CoverageClass {
    let infos = collect_fn_infos(source);
    classify_with_infos(line, &infos)
}

fn is_ui_path(path: &str) -> bool {
    path == "ui" || path.starts_with("ui/")
}

fn classify_with_infos(line: u32, infos: &[FnInfo]) -> CoverageClass {
    let line = line as usize;
    let mut best: Option<&FnInfo> = None;
    for info in infos.iter() {
        if line >= info.start && line <= info.end {
            let narrower = match best {
                None => true,
                Some(current) => {
                    info.start >= current.start
                        && info.end <= current.end
                        && (info.start, info.end) != (current.start, current.end)
                }
            };
            if narrower {
                best = Some(info);
            }
        }
    }
    match best {
        Some(info) if info.e2e_only => CoverageClass::E2eOnly,
        _ => CoverageClass::Untested,
    }
}

pub fn classify_report(
    changed: &[ChangedLine],
    uncovered: &[UncoveredLine],
    load: &dyn Fn(&str) -> Option<String>,
) -> Vec<ClassifiedLine> {
    let mut out = Vec::new();
    let mut cache: HashMap<String, Option<Vec<FnInfo>>> = HashMap::new();
    for change in changed {
        let matches = uncovered
            .iter()
            .any(|u| u.line == change.line && lcov_path_matches(&u.path, &change.path));
        if !matches {
            continue;
        }
        let class = if is_ui_path(&change.path) {
            CoverageClass::Untested
        } else {
            let infos = cache
                .entry(change.path.clone())
                .or_insert_with(|| load(&change.path).map(|source| collect_fn_infos(&source)));
            match infos {
                Some(infos) => classify_with_infos(change.line, infos),
                None => CoverageClass::Untested,
            }
        };
        out.push(ClassifiedLine {
            path: change.path.clone(),
            line: change.line,
            class,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    out
}

pub fn render_report(lines: &[ClassifiedLine]) -> String {
    let mut sorted: Vec<&ClassifiedLine> = lines.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    let mut out = String::new();
    for line in sorted {
        let class = match line.class {
            CoverageClass::Untested => "untested",
            CoverageClass::E2eOnly => "e2e-only",
        };
        out.push_str(&format!("{}:{}\t{class}\n", line.path, line.line));
    }
    out
}

pub fn run_with_root(
    diff_file: &str,
    rust_lcov_file: &str,
    ui_lcov_file: &str,
    root: &str,
) -> String {
    let diff = std::fs::read_to_string(diff_file).unwrap_or_default();
    let rust_lcov = std::fs::read_to_string(rust_lcov_file).unwrap_or_default();
    let ui_lcov = std::fs::read_to_string(ui_lcov_file).unwrap_or_default();
    let changed = parse_changed_lines(&diff);
    let mut uncovered = parse_uncovered_lines(&rust_lcov);
    uncovered.extend(parse_uncovered_lines(&ui_lcov));
    let root = root.to_string();
    let classified = classify_report(&changed, &uncovered, &|path| {
        let full = std::path::Path::new(&root).join(path);
        std::fs::read_to_string(full).ok()
    });
    render_report(&classified)
}

pub fn run(diff_file: &str, rust_lcov_file: &str, ui_lcov_file: &str) -> String {
    let root = env!("CARGO_MANIFEST_DIR");
    let root = std::path::Path::new(root)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string());
    run_with_root(diff_file, rust_lcov_file, ui_lcov_file, &root)
}
