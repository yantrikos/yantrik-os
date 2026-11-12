//! What the reach scans read with `syn` instead of by hand: which parts of a file are test items (dropped),
//! and which spellings start a process in a way `Command::new(` text matching cannot see.
//!
//! A file `syn` cannot parse is an error that names the file. It is never skipped.
#![allow(dead_code)]

use proc_macro2::LineColumn;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Attribute, Meta};

/// Whether `#[cfg(…)]` holds only under `cargo test`: `test`, `all(test, …)`, `any(test, test)`.
fn only_in_test(meta: &Meta) -> bool {
    match meta {
        Meta::Path(p) => p.is_ident("test"),
        Meta::List(list) => {
            let args = list
                .parse_args_with(
                    syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
                )
                .map(|p| p.into_iter().collect::<Vec<_>>())
                .unwrap_or_default();
            if list.path.is_ident("all") {
                args.iter().any(only_in_test)
            } else if list.path.is_ident("any") {
                !args.is_empty() && args.iter().all(only_in_test)
            } else {
                false
            }
        }
        Meta::NameValue(_) => false,
    }
}

fn is_test_attr(attr: &Attribute) -> bool {
    attr.path().is_ident("cfg")
        && attr
            .parse_args::<Meta>()
            .is_ok_and(|meta| only_in_test(&meta))
}

/// The byte range of every node in a file that carries a test-only `#[cfg]`.
#[derive(Default)]
struct TestRanges(Vec<(LineColumn, LineColumn)>);

impl TestRanges {
    fn note(&mut self, attrs: &[Attribute], span: proc_macro2::Span) -> bool {
        if attrs.iter().any(is_test_attr) {
            self.0.push((span.start(), span.end()));
            true
        } else {
            false
        }
    }
}

impl<'ast> Visit<'ast> for TestRanges {
    fn visit_item(&mut self, i: &'ast syn::Item) {
        if !self.note(item_attrs(i), i.span()) {
            visit::visit_item(self, i);
        }
    }
    fn visit_impl_item(&mut self, i: &'ast syn::ImplItem) {
        let attrs: &[Attribute] = match i {
            syn::ImplItem::Const(x) => &x.attrs,
            syn::ImplItem::Fn(x) => &x.attrs,
            syn::ImplItem::Type(x) => &x.attrs,
            syn::ImplItem::Macro(x) => &x.attrs,
            _ => &[],
        };
        if !self.note(attrs, i.span()) {
            visit::visit_impl_item(self, i);
        }
    }
    fn visit_trait_item(&mut self, i: &'ast syn::TraitItem) {
        let attrs: &[Attribute] = match i {
            syn::TraitItem::Const(x) => &x.attrs,
            syn::TraitItem::Fn(x) => &x.attrs,
            syn::TraitItem::Type(x) => &x.attrs,
            syn::TraitItem::Macro(x) => &x.attrs,
            _ => &[],
        };
        if !self.note(attrs, i.span()) {
            visit::visit_trait_item(self, i);
        }
    }
    fn visit_local(&mut self, i: &'ast syn::Local) {
        if !self.note(&i.attrs, i.span()) {
            visit::visit_local(self, i);
        }
    }
    fn visit_arm(&mut self, i: &'ast syn::Arm) {
        if !self.note(&i.attrs, i.span()) {
            visit::visit_arm(self, i);
        }
    }
    fn visit_field(&mut self, i: &'ast syn::Field) {
        if !self.note(&i.attrs, i.span()) {
            visit::visit_field(self, i);
        }
    }
    fn visit_variant(&mut self, i: &'ast syn::Variant) {
        if !self.note(&i.attrs, i.span()) {
            visit::visit_variant(self, i);
        }
    }
}

fn item_attrs(i: &syn::Item) -> &[Attribute] {
    use syn::Item::*;
    match i {
        Const(x) => &x.attrs,
        Enum(x) => &x.attrs,
        ExternCrate(x) => &x.attrs,
        Fn(x) => &x.attrs,
        ForeignMod(x) => &x.attrs,
        Impl(x) => &x.attrs,
        Macro(x) => &x.attrs,
        Mod(x) => &x.attrs,
        Static(x) => &x.attrs,
        Struct(x) => &x.attrs,
        Trait(x) => &x.attrs,
        TraitAlias(x) => &x.attrs,
        Type(x) => &x.attrs,
        Union(x) => &x.attrs,
        Use(x) => &x.attrs,
        _ => &[],
    }
}

fn offset(code: &str, at: LineColumn) -> usize {
    let mut start = 0;
    for _ in 1..at.line {
        start += code[start..]
            .find('\n')
            .map_or(code[start..].len(), |i| i + 1);
    }
    let line = &code[start..];
    start
        + line
            .char_indices()
            .nth(at.column)
            .map_or(line.len(), |(i, _)| i)
}

/// `code` without its test-only items, wherever they sit. What follows an item is kept.
pub fn without_test_items(code: &str) -> Result<String, String> {
    let file = syn::parse_file(code).map_err(|e| e.to_string())?;
    let mut tests = TestRanges::default();
    tests.visit_file(&file);
    let mut ranges: Vec<(usize, usize)> = tests
        .0
        .into_iter()
        .map(|(a, b)| (offset(code, a), offset(code, b)))
        .collect();
    ranges.sort();
    let (mut out, mut at) = (String::new(), 0);
    for (start, end) in ranges {
        if start >= at {
            out.push_str(&code[at..start]);
            out.push('\n');
        }
        at = at.max(end);
    }
    out.push_str(&code[at..]);
    Ok(out)
}

#[derive(Default)]
struct Spawns(Vec<String>);

fn path_text(p: &syn::Path) -> String {
    p.segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

/// Whether a path names a way to start or replace a process other than `Command::new(…)`.
fn exec_path(segs: &[String]) -> bool {
    let last = segs.last().map_or("", String::as_str);
    let from = |m: &str| segs.iter().any(|s| s == m);
    (from("libc") || from("nix") || from("unistd"))
        && (last.starts_with("exec")
            || last.starts_with("posix_spawn")
            || matches!(last, "system" | "fork" | "vfork"))
        || last.starts_with("posix_spawn")
}

impl Spawns {
    fn tree(&mut self, tree: &syn::UseTree, prefix: &mut Vec<String>) {
        match tree {
            syn::UseTree::Path(p) => {
                prefix.push(p.ident.to_string());
                self.tree(&p.tree, prefix);
                prefix.pop();
            }
            syn::UseTree::Name(n) => self.leaf(prefix, &n.ident.to_string(), false),
            syn::UseTree::Rename(r) => self.leaf(prefix, &r.ident.to_string(), true),
            syn::UseTree::Glob(_) => {
                if prefix.last().is_some_and(|m| m == "process") {
                    self.0
                        .push(format!("a glob import of `{}::*`", prefix.join("::")));
                }
            }
            syn::UseTree::Group(g) => g.items.iter().for_each(|t| self.tree(t, prefix)),
        }
    }
    fn leaf(&mut self, prefix: &[String], name: &str, renamed: bool) {
        let mut full = prefix.to_vec();
        full.push(name.to_string());
        if renamed && name == "Command" {
            self.0
                .push(format!("`{}` imported under another name", full.join("::")));
        }
        if exec_path(&full) {
            self.0.push(format!("`{}` imported", full.join("::")));
        }
    }
}

fn names_command(ty: &syn::Type) -> bool {
    struct Find(bool);
    impl<'a> Visit<'a> for Find {
        fn visit_path(&mut self, p: &'a syn::Path) {
            if p.segments.last().is_some_and(|s| s.ident == "Command") {
                self.0 = true;
            }
            visit::visit_path(self, p);
        }
    }
    let mut find = Find(false);
    find.visit_type(ty);
    find.0
}

impl<'ast> Visit<'ast> for Spawns {
    fn visit_item_use(&mut self, i: &'ast syn::ItemUse) {
        self.tree(&i.tree, &mut Vec::new());
    }
    fn visit_item_type(&mut self, i: &'ast syn::ItemType) {
        if names_command(&i.ty) {
            self.0
                .push(format!("`type {}` is an alias of a `Command`", i.ident));
        }
        visit::visit_item_type(self, i);
    }
    fn visit_expr_call(&mut self, i: &'ast syn::ExprCall) {
        // `Command::new(x)` as a call is read by the text scans; named any other way it is flagged below.
        if let syn::Expr::Path(p) = &*i.func {
            let segs: Vec<String> = p
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            if exec_path(&segs) {
                self.0.push(format!("`{}(…)` called", segs.join("::")));
            }
        } else {
            self.visit_expr(&i.func);
        }
        i.args.iter().for_each(|a| self.visit_expr(a));
    }
    fn visit_expr_path(&mut self, i: &'ast syn::ExprPath) {
        let text = path_text(&i.path);
        if text.ends_with("Command::new") {
            self.0.push(format!("`{text}` used as a value, not called"));
        }
    }
}

/// Every way a file starts a process that the text scan for `Command::new(` cannot see: `Command` under
/// another name (`use … as`, a brace import, a `type` alias, a glob of `process`), `Command::new` taken
/// as a function value, `libc::exec*`, `libc::system`, `posix_spawn*`, `nix::unistd::exec*`.
pub fn hidden_spawns(code: &str) -> Result<Vec<String>, String> {
    let file = syn::parse_file(code).map_err(|e| e.to_string())?;
    let mut spawns = Spawns::default();
    spawns.visit_file(&file);
    Ok(spawns.0)
}
