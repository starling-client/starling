//! Conservative lexical facts shared by site resolution and expression rewriting.
use crate::points::unraw;
use proc_macro2::{Ident, Span, TokenStream, TokenTree};
use quote::ToTokens;
use std::collections::BTreeSet;
use syn::{Expr, Item, spanned::Spanned, visit::Visit};

#[derive(Default)]
struct Bindings {
    names: BTreeSet<String>,
    identifiers: BTreeSet<String>,
    opaque: bool,
}
fn tokens(stream: TokenStream, identifiers: &mut BTreeSet<String>) {
    for token in stream {
        match token {
            TokenTree::Ident(id) => {
                identifiers.insert(unraw(&id));
            }
            TokenTree::Group(group) => tokens(group.stream(), identifiers),
            _ => {}
        }
    }
}
impl<'a> Visit<'a> for Bindings {
    fn visit_ident(&mut self, id: &'a Ident) {
        self.identifiers.insert(unraw(id));
    }
    fn visit_pat_ident(&mut self, pattern: &'a syn::PatIdent) {
        self.names.insert(unraw(&pattern.ident));
        syn::visit::visit_pat_ident(self, pattern);
    }
    fn visit_macro(&mut self, mac: &'a syn::Macro) {
        self.opaque = true;
        tokens(mac.to_token_stream(), &mut self.identifiers);
    }
    fn visit_use_name(&mut self, name: &'a syn::UseName) {
        self.opaque |= name.ident == "self";
        self.names.insert(unraw(&name.ident));
    }
    fn visit_use_rename(&mut self, name: &'a syn::UseRename) {
        self.names.insert(unraw(&name.rename));
    }
    fn visit_use_glob(&mut self, _: &'a syn::UseGlob) {
        self.opaque = true;
    }
    fn visit_item(&mut self, item: &'a Item) {
        tokens(item.to_token_stream(), &mut self.identifiers);
        let id = match item {
            Item::ExternCrate(i) => i.rename.as_ref().map_or(&i.ident, |(_, name)| name),
            Item::Fn(i) => &i.sig.ident,
            Item::Const(i) => &i.ident,
            Item::Static(i) => &i.ident,
            Item::Mod(i) => &i.ident,
            Item::Struct(i) => &i.ident,
            Item::Enum(i) => &i.ident,
            Item::Union(i) => &i.ident,
            Item::Type(i) => &i.ident,
            Item::Trait(i) => &i.ident,
            Item::Use(i) => {
                self.visit_use_tree(&i.tree);
                return;
            }
            Item::Macro(_) => {
                self.opaque = true;
                return;
            }
            _ => return,
        };
        self.names.insert(unraw(id));
    }
}

/// Whole-function over-approximation: a binding anywhere in the body may shadow a catalog path.
/// Unsupported macro expansions and local glob imports cannot safely supply lexical facts.
pub(crate) struct Scope {
    pub parameters: BTreeSet<String>,
    pub locals: BTreeSet<String>,
    pub opaque: bool,
    identifiers: BTreeSet<String>,
}
impl Scope {
    pub fn scan(signature: &syn::Signature, body: Option<&syn::Block>) -> Self {
        let mut scan = Bindings::default();
        scan.visit_signature(signature);
        let parameters = std::mem::take(&mut scan.names);
        if let Some(body) = body {
            scan.visit_block(body);
        }
        Self {
            parameters,
            locals: scan.names,
            opaque: scan.opaque,
            identifiers: scan.identifiers,
        }
    }
    /// Refuse call identities that cannot be proven from module-level catalog facts.
    pub fn check_target(&self, expr: &Expr) -> Result<(), super::Cause> {
        let lexical = match expr {
            Expr::MethodCall(call) => root(&call.receiver).map(|name| (name, false)),
            Expr::Call(call) => match &*call.func {
                Expr::Path(path) if path.path.leading_colon.is_none() => path
                    .path
                    .segments
                    .first()
                    .filter(|s| {
                        !matches!(
                            unraw(&s.ident).as_str(),
                            "crate" | "self" | "super" | "Self"
                        )
                    })
                    .map(|s| (&s.ident, true)),
                _ => None,
            },
            _ => None,
        };
        if let Some((name, parameter)) = lexical {
            let name = unraw(name);
            if self.locals.contains(&name)
                || parameter && self.parameters.contains(&name)
                || self.opaque && name != "self"
            {
                return Err(super::Cause::Missing(
                    "lexically shadowed or macro-dependent call target",
                ));
            }
        }
        Ok(())
    }

    pub fn fresh(&mut self, base: &str) -> Ident {
        for index in 0.. {
            let name = if index == 0 {
                base.into()
            } else {
                format!("{base}_{index}")
            };
            if self.identifiers.insert(name.clone()) {
                return Ident::new(&name, Span::call_site());
            }
        }
        unreachable!()
    }
}

/// The lexical root of a supported receiver expression.
pub(crate) fn root(expr: &Expr) -> Option<&Ident> {
    match expr {
        Expr::Path(path) if path.path.leading_colon.is_none() && path.path.segments.len() == 1 => {
            Some(&path.path.segments[0].ident)
        }
        Expr::Field(field) => root(&field.base),
        _ => None,
    }
}

/// A macro or unknown local import may expand into bindings that cannot be hygienically lowered.
pub(crate) fn opaque_argument(expr: &Expr) -> bool {
    let mut scan = Bindings::default();
    scan.visit_expr(expr);
    scan.opaque
}

/// Check expression lowering after the callee's record fields have been proven scalar/unit.
/// A mutable receiver is borrowed before record construction, so argument expressions needing
/// alias analysis are refused rather than losing Rust's two-phase borrowing semantics.
pub(crate) fn arguments(
    args: &[&Expr],
    receiver: Option<&Expr>,
    mutable: bool,
) -> Result<(), super::Cause> {
    fn simple_value(expr: &Expr) -> bool {
        match expr {
            Expr::Lit(_) | Expr::Path(_) => true,
            Expr::Paren(e) => simple_value(&e.expr),
            Expr::Group(e) => simple_value(&e.expr),
            Expr::Unary(e) => {
                matches!(e.op, syn::UnOp::Neg(_) | syn::UnOp::Not(_))
                    && matches!(&*e.expr, Expr::Lit(_))
            }
            _ => false,
        }
    }
    let root = receiver.and_then(root).map(unraw);
    for arg in args {
        if opaque_argument(arg) {
            return Err(super::Cause::Record(
                "macro-dependent argument cannot be hygienically lowered",
            ));
        }
        let mut identifiers = BTreeSet::new();
        tokens(arg.to_token_stream(), &mut identifiers);
        if identifiers.contains("self")
            || root.as_ref().is_some_and(|name| identifiers.contains(name))
        {
            return Err(super::Cause::Record(
                "argument borrows or reads the receiver: two-phase borrowing is not record-safe",
            ));
        }
        if mutable && !simple_value(arg) {
            return Err(super::Cause::Record(
                "mutable-receiver argument needs a literal or independent scalar binding: two-phase borrowing is not record-safe",
            ));
        }
    }
    Ok(())
}

/// Matching calls in source order, excluding every AST context whose expressions are constant.
pub(crate) fn calls<'a>(body: &'a syn::Block, selector: &str) -> Vec<&'a Expr> {
    struct Calls<'a, 's> {
        selector: &'s str,
        calls: Vec<&'a Expr>,
    }
    impl<'a> Visit<'a> for Calls<'a, '_> {
        fn visit_expr(&mut self, expr: &'a Expr) {
            let spelling = match expr {
                Expr::MethodCall(call) => {
                    format!("{}.{}", call.receiver.to_token_stream(), call.method).replace(' ', "")
                }
                Expr::Call(call) => call.func.to_token_stream().to_string().replace(' ', ""),
                _ => String::new(),
            };
            if !spelling.is_empty()
                && (self.selector.is_empty()
                    || spelling == self.selector
                    || self.selector.starts_with('.') && spelling.ends_with(self.selector))
            {
                self.calls.push(expr);
            }
            syn::visit::visit_expr(self, expr);
        }
        fn visit_item(&mut self, _: &'a Item) {}
        fn visit_type(&mut self, _: &'a syn::Type) {}
        fn visit_generic_argument(&mut self, _: &'a syn::GenericArgument) {}
        fn visit_expr_const(&mut self, _: &'a syn::ExprConst) {}
        fn visit_expr_repeat(&mut self, repeat: &'a syn::ExprRepeat) {
            self.visit_expr(&repeat.expr);
        }
    }
    let mut calls = Calls {
        selector,
        calls: Vec::new(),
    };
    calls.visit_block(body);
    calls.calls.sort_by_key(|call| match call {
        Expr::MethodCall(call) => call.method.span().start(),
        _ => call.span().start(),
    });
    calls.calls
}

#[cfg(test)]
mod tests;
