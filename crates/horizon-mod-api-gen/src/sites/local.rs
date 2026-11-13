//! Conservative source-only local validation. Unknown dataflow is refused.
use super::{Cause, Error, Local};
use quote::ToTokens;
use syn::{spanned::Spanned, visit::Visit};

pub(super) fn validate(
    signature: &syn::Signature,
    body: &syn::Block,
    call: &syn::Expr,
    local: &Local,
) -> Result<syn::Ident, Error> {
    let refuse =
        |why: String| Error::from(Cause::Source(format!("local `{}`: {why}", local.binding)));
    let stmt = body
        .stmts
        .iter()
        .position(|s| {
            s.span().start() <= call.span().start() && s.span().end() >= call.span().end()
        })
        .ok_or_else(|| refuse("position is not in the enclosing block".into()))?;
    if !matches!(&body.stmts[stmt], syn::Stmt::Expr(e, _) if e.span().start() == call.span().start() && e.span().end() == call.span().end())
    {
        return Err(refuse(
            "nested call position has uncertain scope or evaluation order".into(),
        ));
    }
    let mut bindings = Vec::new();
    for (index, statement) in body.stmts[..stmt].iter().enumerate() {
        if let syn::Stmt::Local(decl) = statement {
            let (pat, ty) = match &decl.pat {
                syn::Pat::Type(p) => (&*p.pat, Some(p.ty.to_token_stream().to_string())),
                p => (p, None),
            };
            if let syn::Pat::Ident(pat) = pat {
                if pat.ident == local.binding {
                    let ty = ty.or_else(|| {
                        decl.init.as_ref().and_then(|i| match &*i.expr {
                            syn::Expr::Lit(lit) => match &lit.lit {
                                syn::Lit::Int(n) if !n.suffix().is_empty() => {
                                    Some(n.suffix().into())
                                }
                                syn::Lit::Bool(_) => Some("bool".into()),
                                _ => None,
                            },
                            _ => None,
                        })
                    });
                    bindings.push((index, pat, ty, decl.init.is_some()));
                }
            } else if pat
                .to_token_stream()
                .to_string()
                .split_whitespace()
                .any(|n| n == local.binding)
            {
                return Err(refuse("ambiguous destructured binding".into()));
            }
        }
    }
    let parameter_shadow = signature.inputs.iter().any(|arg| match arg {
        syn::FnArg::Typed(arg) => arg
            .pat
            .to_token_stream()
            .to_string()
            .split_whitespace()
            .any(|n| n == local.binding),
        _ => false,
    });
    if parameter_shadow {
        return Err(refuse("shadowed or unsupported parameter binding".into()));
    }
    if bindings.len() > 1 {
        return Err(refuse("shadowed binding".into()));
    }
    let (index, pat, ty, initialized) = bindings
        .pop()
        .ok_or_else(|| refuse("binding not found".into()))?;
    if pat.mutability.is_none() || pat.by_ref.is_some() {
        return Err(refuse("binding must be declared mut".into()));
    }
    if !initialized {
        return Err(refuse(
            "binding is not initialized at its declaration".into(),
        ));
    }
    let ty = ty.ok_or_else(|| refuse(format!("cannot prove declared type `{}`", local.ty)))?;
    let expected = syn::parse_str::<syn::Type>(&local.ty)
        .map_err(|_| refuse(format!("invalid type `{}`", local.ty)))?
        .to_token_stream()
        .to_string();
    if ty != expected {
        return Err(refuse(format!(
            "type mismatch: binding `{ty}`, manifest `{expected}`"
        )));
    }
    let copy = matches!(
        ty.as_str(),
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "bool"
            | "char"
            | "f32"
            | "f64"
    );
    struct Uses<'a> {
        name: &'a str,
        used: bool,
        borrowed: bool,
        opaque: bool,
    }
    impl Uses<'_> {
        fn pattern_borrow(&mut self, pattern: &syn::Pat, value: &syn::Expr) {
            #[derive(Default)]
            struct ByRef(bool);
            impl<'ast> Visit<'ast> for ByRef {
                fn visit_pat_ident(&mut self, pat: &'ast syn::PatIdent) {
                    self.0 |= pat.by_ref.is_some();
                    syn::visit::visit_pat_ident(self, pat);
                }
            }
            let mut pattern_scan = ByRef::default();
            pattern_scan.visit_pat(pattern);
            if pattern_scan.0 {
                let mut value_scan = Uses {
                    name: self.name,
                    used: false,
                    borrowed: false,
                    opaque: false,
                };
                value_scan.visit_expr(value);
                self.borrowed |= value_scan.used;
            }
        }
    }
    impl<'ast> Visit<'ast> for Uses<'_> {
        fn visit_local(&mut self, local: &'ast syn::Local) {
            if let Some(init) = &local.init {
                self.pattern_borrow(&local.pat, &init.expr);
            }
            syn::visit::visit_local(self, local);
        }
        fn visit_expr_let(&mut self, expr: &'ast syn::ExprLet) {
            self.pattern_borrow(&expr.pat, &expr.expr);
            syn::visit::visit_expr_let(self, expr);
        }
        fn visit_expr_match(&mut self, expr: &'ast syn::ExprMatch) {
            for arm in &expr.arms {
                self.pattern_borrow(&arm.pat, &expr.expr);
            }
            syn::visit::visit_expr_match(self, expr);
        }

        fn visit_expr_path(&mut self, p: &'ast syn::ExprPath) {
            self.used |= p.path.is_ident(self.name);
        }
        fn visit_expr_reference(&mut self, r: &'ast syn::ExprReference) {
            let mut use_ = Uses {
                name: self.name,
                used: false,
                borrowed: false,
                opaque: false,
            };
            use_.visit_expr(&r.expr);
            self.borrowed |= use_.used;
            syn::visit::visit_expr_reference(self, r);
        }
        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            let mut receiver = Uses {
                name: self.name,
                used: false,
                borrowed: false,
                opaque: false,
            };
            receiver.visit_expr(&call.receiver);
            self.borrowed |= receiver.used;
            syn::visit::visit_expr_method_call(self, call);
        }
        fn visit_expr_raw_addr(&mut self, raw: &'ast syn::ExprRawAddr) {
            let mut referent = Uses {
                name: self.name,
                used: false,
                borrowed: false,
                opaque: false,
            };
            referent.visit_expr(&raw.expr);
            self.borrowed |= referent.used;
            syn::visit::visit_expr_raw_addr(self, raw);
        }
        fn visit_macro(&mut self, _: &'ast syn::Macro) {
            self.opaque = true;
        }
        fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {
            self.opaque = true;
        }
    }
    let mut uses = Uses {
        name: &local.binding,
        used: false,
        borrowed: false,
        opaque: false,
    };
    let mut prefix = Uses {
        name: &local.binding,
        used: false,
        borrowed: false,
        opaque: false,
    };
    for s in &body.stmts[..index] {
        prefix.visit_stmt(s);
    }
    if prefix.opaque {
        return Err(refuse(
            "macro or closure makes binding scope uncertain".into(),
        ));
    }
    for s in &body.stmts[index + 1..stmt + usize::from(local.after)] {
        uses.visit_stmt(s);
    }
    if uses.borrowed {
        return Err(refuse("binding is borrowed across the position".into()));
    }
    if uses.opaque || uses.used && !copy {
        return Err(refuse(
            "binding may have moved or been borrowed; dataflow is uncertain".into(),
        ));
    }
    Ok(pat.ident.clone())
}
