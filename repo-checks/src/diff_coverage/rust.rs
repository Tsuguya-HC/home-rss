use std::collections::{BTreeMap, BTreeSet};

use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::Visit;

/// Paths whose use means the code needs the Spin runtime: values that only the
/// host hands out (a PostgreSQL connection) and calls that only the host
/// answers. A path also matches anything below it (`Connection::open`).
const RUNTIME_ONLY: &[&str] = &[
    "spin_sdk::pg::Connection",
    "spin_sdk::pg::OpenOptions",
    "spin_sdk::variables::get",
    "spin_sdk::http::send",
    "spin_sdk::time::sleep",
];
/// Requests and responses with the default body, or an incoming one named
/// explicitly, are only made by the host, so taking one as a parameter makes a
/// function runtime-only. A body type a unit test can build (`EmptyBody`, a
/// full body) does not.
const INCOMING_HTTP: &[&str] = &["spin_sdk::http::Request", "spin_sdk::http::Response"];
const INCOMING_BODIES: &[&str] = &["IncomingRequestBody", "IncomingResponseBody"];
const HTTP_SERVICE: &str = "spin_sdk::http_service";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Unit tests could run it.
    Unit,
    /// Only runs inside Spin, so only e2e reaches it.
    E2eOnly,
}

#[derive(Debug, Default)]
pub struct FileAnalysis {
    fns: Vec<(u32, u32, Reach)>,
    tests: Vec<(u32, u32)>,
}

impl FileAnalysis {
    pub fn is_test_line(&self, line: u32) -> bool {
        self.tests.iter().any(|&(s, e)| s <= line && line <= e)
    }

    /// None outside every function. Nested functions are not collected, so
    /// ranges do not overlap.
    pub fn reach(&self, line: u32) -> Option<Reach> {
        self.fns
            .iter()
            .find(|&&(s, e, _)| s <= line && line <= e)
            .map(|&(_, _, r)| r)
    }
}

struct FnFacts {
    file: String,
    start: u32,
    end: u32,
    name: String,
    runtime_only: bool,
    /// Names referenced as paths (calls and function values).
    path_refs: BTreeSet<String>,
    /// Names called as `self.name(…)`.
    self_calls: BTreeSet<String>,
    is_method: bool,
}

/// A function is e2e-only when it touches the Spin runtime itself or calls a
/// function that is e2e-only.
///
/// Calls are matched by name across the whole workspace, without type
/// information. A path reference (`f(…)`, `m::f(…)`, `Type::f(…)`, `.map(f)`)
/// counts when every workspace function of that name is e2e-only; `x.f(…)`
/// counts only for `self.f(…)` to a method name no free function shares.
/// Matching any `.f(…)` by name would let `.get(…)` on a map pick up a
/// workspace `get`.
pub fn analyze(
    sources: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, FileAnalysis>, String> {
    let mut facts = Vec::new();
    let mut files = BTreeMap::new();
    for (path, source) in sources {
        let file = syn::parse_file(source).map_err(|e| {
            let at = e.span().start();
            format!("{path}:{}:{}: cannot parse: {e}", at.line, at.column + 1)
        })?;
        let imports = Imports::of(&file);
        let mut analysis = FileAnalysis::default();
        collect_items(&file.items, path, &imports, &mut facts, &mut analysis);
        files.insert(path.clone(), analysis);
    }

    let mut by_name: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, f) in facts.iter().enumerate() {
        by_name.entry(f.name.as_str()).or_default().push(i);
    }
    let mut e2e: Vec<bool> = facts.iter().map(|f| f.runtime_only).collect();
    loop {
        let mut changed = false;
        for (i, f) in facts.iter().enumerate() {
            if e2e[i] {
                continue;
            }
            let via_path = f.path_refs.iter().any(|n| {
                by_name
                    .get(n.as_str())
                    .is_some_and(|c| c.iter().all(|&j| e2e[j]))
            });
            let via_self = f.self_calls.iter().any(|n| {
                by_name.get(n.as_str()).is_some_and(|c| {
                    c.iter().all(|&j| facts[j].is_method) && c.iter().all(|&j| e2e[j])
                })
            });
            if via_path || via_self {
                e2e[i] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    for (i, f) in facts.iter().enumerate() {
        let reach = if e2e[i] { Reach::E2eOnly } else { Reach::Unit };
        if let Some(a) = files.get_mut(&f.file) {
            a.fns.push((f.start, f.end, reach));
        }
    }
    Ok(files)
}

fn lines(span: proc_macro2::Span) -> (u32, u32) {
    (span.start().line as u32, span.end().line as u32)
}

fn collect_items(
    items: &[syn::Item],
    file: &str,
    imports: &Imports,
    facts: &mut Vec<FnFacts>,
    out: &mut FileAnalysis,
) {
    for item in items {
        match item {
            syn::Item::Fn(f) => {
                if is_test_code(&f.attrs) {
                    out.tests.push(lines(item.span()));
                } else {
                    facts.push(fn_facts(
                        file,
                        item.span(),
                        &f.attrs,
                        &f.sig,
                        Some(&f.block),
                        imports,
                        false,
                    ));
                }
            }
            syn::Item::Mod(m) => {
                if is_test_code(&m.attrs) {
                    out.tests.push(lines(item.span()));
                } else if let Some((_, content)) = &m.content {
                    collect_items(content, file, imports, facts, out);
                }
            }
            syn::Item::Impl(imp) => {
                if is_test_code(&imp.attrs) {
                    out.tests.push(lines(item.span()));
                    continue;
                }
                for ii in &imp.items {
                    if let syn::ImplItem::Fn(m) = ii {
                        if is_test_code(&m.attrs) {
                            out.tests.push(lines(ii.span()));
                        } else {
                            facts.push(fn_facts(
                                file,
                                ii.span(),
                                &m.attrs,
                                &m.sig,
                                Some(&m.block),
                                imports,
                                true,
                            ));
                        }
                    }
                }
            }
            syn::Item::Trait(t) => {
                if is_test_code(&t.attrs) {
                    out.tests.push(lines(item.span()));
                    continue;
                }
                for ti in &t.items {
                    if let syn::TraitItem::Fn(m) = ti
                        && let Some(block) = &m.default
                    {
                        facts.push(fn_facts(
                            file,
                            ti.span(),
                            &m.attrs,
                            &m.sig,
                            Some(block),
                            imports,
                            true,
                        ));
                    }
                }
            }
            _ => {}
        }
    }
}

fn is_test_code(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        let p = a.path();
        // #[test], and #[tokio::test]-style runners.
        if p.is_ident("test") || (p.segments.len() == 2 && p.segments[1].ident == "test") {
            return true;
        }
        if p.is_ident("cfg")
            && let syn::Meta::List(list) = &a.meta
        {
            return mentions_test(list.tokens.clone());
        }
        false
    })
}

/// `cfg(test)`, `cfg(all(test, …))`; not `cfg(not(test))`.
fn mentions_test(tokens: proc_macro2::TokenStream) -> bool {
    let mut iter = tokens.into_iter().peekable();
    while let Some(tt) = iter.next() {
        match tt {
            proc_macro2::TokenTree::Ident(id) if id == "not" => {
                iter.next();
            }
            proc_macro2::TokenTree::Ident(id) if id == "test" => return true,
            proc_macro2::TokenTree::Group(g) if mentions_test(g.stream()) => return true,
            _ => {}
        }
    }
    false
}

fn fn_facts(
    file: &str,
    span: proc_macro2::Span,
    attrs: &[syn::Attribute],
    sig: &syn::Signature,
    block: Option<&syn::Block>,
    imports: &Imports,
    is_method: bool,
) -> FnFacts {
    let (start, end) = lines(span);
    let mut v = FnVisitor {
        imports,
        runtime_only: false,
        path_refs: BTreeSet::new(),
        local_refs: BTreeSet::new(),
        bound: BTreeSet::new(),
        self_calls: BTreeSet::new(),
    };
    if attrs
        .iter()
        .any(|a| imports.resolve(a.path()).as_deref() == Some(HTTP_SERVICE))
    {
        v.runtime_only = true;
    }
    for input in &sig.inputs {
        if let syn::FnArg::Typed(t) = input {
            let mut finder = IncomingFinder {
                imports,
                found: false,
            };
            finder.visit_type(&t.ty);
            if finder.found {
                v.runtime_only = true;
            }
        }
    }
    v.visit_signature(sig);
    if let Some(b) = block {
        v.visit_block(b);
    }
    let mut path_refs = v.path_refs;
    path_refs.extend(v.local_refs.difference(&v.bound).cloned());
    FnFacts {
        file: file.to_string(),
        start,
        end,
        name: sig.ident.to_string(),
        runtime_only: v.runtime_only,
        path_refs,
        self_calls: v.self_calls,
        is_method,
    }
}

fn runtime_only(resolved: &str) -> bool {
    RUNTIME_ONLY.iter().any(|p| {
        resolved == *p
            || resolved
                .strip_prefix(p)
                .is_some_and(|r| r.starts_with("::"))
    })
}

struct FnVisitor<'a> {
    imports: &'a Imports,
    runtime_only: bool,
    path_refs: BTreeSet<String>,
    /// Single-segment references; a name the function binds itself (a
    /// parameter, a `let`, a pattern) is a local, not a call.
    local_refs: BTreeSet<String>,
    bound: BTreeSet<String>,
    self_calls: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for FnVisitor<'_> {
    fn visit_path(&mut self, p: &'ast syn::Path) {
        if let Some(r) = self.imports.resolve(p)
            && runtime_only(&r)
        {
            self.runtime_only = true;
        }
        syn::visit::visit_path(self, p);
    }

    fn visit_expr_path(&mut self, e: &'ast syn::ExprPath) {
        let refs = if e.path.segments.len() == 1 {
            &mut self.local_refs
        } else {
            &mut self.path_refs
        };
        refs.extend(e.path.segments.last().map(|l| l.ident.to_string()));
        syn::visit::visit_expr_path(self, e);
    }

    fn visit_pat_ident(&mut self, p: &'ast syn::PatIdent) {
        self.bound.insert(p.ident.to_string());
        syn::visit::visit_pat_ident(self, p);
    }

    fn visit_expr_method_call(&mut self, e: &'ast syn::ExprMethodCall) {
        if let syn::Expr::Path(r) = e.receiver.as_ref()
            && r.path.is_ident("self")
        {
            self.self_calls.insert(e.method.to_string());
        }
        syn::visit::visit_expr_method_call(self, e);
    }

    // Macro arguments are opaque tokens to syn; read them as expressions when
    // they parse as such (format!, assert!, vec!, …).
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        if let Ok(args) =
            m.parse_body_with(Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated)
        {
            for a in &args {
                self.visit_expr(a);
            }
        }
        syn::visit::visit_macro(self, m);
    }
}

struct IncomingFinder<'a> {
    imports: &'a Imports,
    found: bool,
}

impl<'ast> Visit<'ast> for IncomingFinder<'_> {
    fn visit_path(&mut self, p: &'ast syn::Path) {
        let resolved = self.imports.resolve(p);
        if resolved
            .as_deref()
            .is_some_and(|r| INCOMING_HTTP.contains(&r))
            && has_incoming_body(p)
        {
            self.found = true;
        }
        syn::visit::visit_path(self, p);
    }
}

/// The last segment names no body type (the default) or an incoming one.
fn has_incoming_body(p: &syn::Path) -> bool {
    p.segments.last().is_some_and(|last| {
        last.arguments.is_none()
            || matches!(
                &last.arguments,
                syn::PathArguments::AngleBracketed(a)
                    if a.args.len() == 1
                        && matches!(
                            a.args.first(),
                            Some(syn::GenericArgument::Type(syn::Type::Path(t)))
                                if t.path.segments.last().is_some_and(|s| {
                                    INCOMING_BODIES.contains(&s.ident.to_string().as_str())
                                })
                        )
            )
    })
}

/// `use` declarations anywhere in a file, flattened: local name → full path,
/// and glob prefixes.
#[derive(Default)]
struct Imports {
    names: BTreeMap<String, String>,
    globs: Vec<String>,
}

impl Imports {
    fn of(file: &syn::File) -> Self {
        struct Uses<'a>(&'a mut Imports);
        impl<'ast> Visit<'ast> for Uses<'_> {
            fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
                record(&u.tree, &mut Vec::new(), self.0);
            }
        }
        let mut imports = Imports::default();
        Uses(&mut imports).visit_file(file);
        imports
    }

    /// The full path `p` names when it starts at `spin_sdk`, else None. A name
    /// is taken from a glob import only when the result is one of the paths
    /// this analysis looks for; any other name (`Ok`, a local) stays unknown.
    fn resolve(&self, p: &syn::Path) -> Option<String> {
        let segs: Vec<String> = p.segments.iter().map(|s| s.ident.to_string()).collect();
        let first = segs.first()?;
        let full = if first == "spin_sdk" {
            segs.join("::")
        } else if let Some(mapped) = self.names.get(first) {
            std::iter::once(mapped.clone())
                .chain(segs[1..].iter().cloned())
                .collect::<Vec<_>>()
                .join("::")
        } else {
            return self.globs.iter().find_map(|g| {
                let candidate = format!("{g}::{}", segs.join("::"));
                let known = runtime_only(&candidate)
                    || INCOMING_HTTP.contains(&candidate.as_str())
                    || candidate == HTTP_SERVICE;
                known.then_some(candidate)
            });
        };
        full.starts_with("spin_sdk").then_some(full)
    }
}

fn record(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Imports) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            record(&p.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            let full = join_with(prefix, &n.ident.to_string());
            let local = match (n.ident == "self", prefix.last()) {
                (true, Some(module)) => module.clone(),
                _ => n.ident.to_string(),
            };
            out.names.insert(local, full);
        }
        syn::UseTree::Rename(r) => {
            let full = join_with(prefix, &r.ident.to_string());
            out.names.insert(r.rename.to_string(), full);
        }
        syn::UseTree::Glob(_) => out.globs.push(prefix.join("::")),
        syn::UseTree::Group(g) => {
            for t in &g.items {
                record(t, prefix, out);
            }
        }
    }
}

fn join_with(prefix: &[String], last: &str) -> String {
    if last == "self" {
        return prefix.join("::");
    }
    prefix
        .iter()
        .cloned()
        .chain(std::iter::once(last.to_string()))
        .collect::<Vec<_>>()
        .join("::")
}
