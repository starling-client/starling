//! What the parser reads of one source file for [`crate::uses`]: where structs are built and
//! matched, which paths are written, which impl each `Self` stands in, what the file defines,
//! and the byte ranges of macro bodies, whose tokens the parser does not see into.
use crate::scan::Token;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use syn::visit::{self, Visit};

/// How a struct is built or matched at one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum How {
    /// A literal or pattern that lists every field: `S { a, b }`, `S(a, b)`.
    Full,
    /// A literal that ends in `..` or a base: `S { a, ..d }`.
    Base,
    /// A name used as a value or bound in a pattern by itself (`.map(Pos)`, `Marker`): a build
    /// only for a tuple or unit struct.
    Value,
}

/// A place a struct may be built: the path written, as names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Build<'t> {
    /// The path's names; `Self` for `Self { … }`.
    pub path: Vec<&'t str>,
    /// Written with a leading `::`.
    pub rooted: bool,
    pub how: How,
    /// A brace literal (`S { … }`), for the `..Default::default()` advice.
    pub brace: bool,
    pub line: usize,
    /// The byte where the path starts.
    pub byte: usize,
}

/// A tuple struct's field read by its position where the parser can tie the read to the
/// struct's path ([`Reader::tie`]): `X(..).0`, `v.0` where `v` is bound by `let v: X`,
/// `let v = X(..)` or a parameter `v: X` (through `&` and `&mut`), `self.0` in a method (path
/// `Self`), and each position a pattern `X(a, b, ..)` lists before its `..`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TupleField<'t> {
    pub tie: Tie<'t>,
    pub position: u32,
    pub line: usize,
    /// The byte where the read is.
    pub byte: usize,
}

/// An `impl` block: the bytes of its body, and the path of the type it is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Impl<'t> {
    pub body: Range<usize>,
    /// Its names, an empty first one for a leading `::`; `None` when the type is not a plain
    /// path the scan can read (a tuple, a macro), so what `Self` means there is unknown.
    pub target: Option<Vec<&'t str>>,
}

/// One module's direct items, separate from the names visible in other modules or blocks.
#[derive(Debug, Default)]
pub(crate) struct Module<'t> {
    pub body: Range<usize>,
    pub parent: usize,
    /// Unconditional child modules: an inline scope index, or `None` for a file declaration.
    pub children: BTreeMap<&'t str, Option<usize>>,
    /// Unconditional concrete types, not aliases that can stand for API types.
    pub types: BTreeSet<&'t str>,
    /// The subset that also declare a tuple constructor in the value namespace.
    pub tuple_constructors: BTreeSet<&'t str>,
    /// The tuple variants of each unconditional enum.
    pub tuple_variants: BTreeMap<&'t str, BTreeSet<&'t str>>,
    /// A block item or type parameter may shadow the first segment of a relative path.
    pub block_items: bool,
    /// A block value item may shadow an unqualified constructor.
    pub block_values: bool,
    pub type_params: BTreeSet<&'t str>,
}

/// What the parser reads of one file.
#[derive(Debug, Default)]
pub(crate) struct Syntax<'t> {
    pub builds: Vec<Build<'t>>,
    pub tuple_fields: Vec<TupleField<'t>>,
    /// Every path written in code, attributes included, `use` trees left out: its names.
    pub paths: Vec<Vec<&'t str>>,
    pub impls: Vec<Impl<'t>>,
    /// The structs, enums, unions and type aliases the file defines at its top level, under no
    /// `#[cfg]` or `#[cfg_attr]`.
    pub defines: BTreeSet<&'t str>,
    /// Every struct, enum and union defined anywhere, and every enum variant.
    pub types: BTreeSet<&'t str>,
    /// Every trait defined anywhere.
    pub traits: BTreeSet<&'t str>,
    /// `type X = …Y<…>;` anywhere: (X, Y), and whether it is an item of its own (not an
    /// associated type of an impl or trait).
    pub type_aliases: Vec<(&'t str, &'t str, bool)>,
    /// The file holds an inline `mod name { … }`, so it is more than one scope.
    pub inline_modules: bool,
    /// Every module the file declares, inline or not (`mod api;`).
    pub modules: BTreeSet<&'t str>,
    /// Names from `extern crate dep as name` that do not refer to the API itself.
    pub extern_aliases: BTreeSet<&'t str>,
    /// The first bytes of unconditional `use` statements directly in a module.
    pub direct_uses: BTreeSet<usize>,
    /// The file's top-level items and its inline modules, and their indices in source order.
    pub scopes: Vec<Module<'t>>,
    scope_order: Vec<usize>,
    /// The bytes inside each macro call's delimiters, and each `macro_rules!` body.
    pub macro_bodies: Vec<Range<usize>>,
}

impl Syntax<'_> {
    pub fn module_at(&self, byte: usize) -> usize {
        let after = self
            .scope_order
            .partition_point(|&scope| self.scopes[scope].body.start <= byte);
        let mut scope = self.scope_order[after - 1];
        while byte >= self.scopes[scope].body.end && scope != 0 {
            scope = self.scopes[scope].parent;
        }
        scope
    }
}

/// The path a type is, through parentheses and invisible groups; `None` for any other type.
fn plain_path(ty: &syn::Type) -> Option<&syn::Path> {
    match ty {
        syn::Type::Path(t) if t.qself.is_none() => Some(&t.path),
        syn::Type::Paren(t) => plain_path(&t.elem),
        syn::Type::Group(t) => plain_path(&t.elem),
        _ => None,
    }
}

/// Whether an item carries `#[cfg(…)]` or `#[cfg_attr(…)]`.
fn conditional(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
}

/// The path a type is through `&`, `&mut`, parentheses and invisible groups, with whether it
/// is written with a leading `::`; `None` for any other type.
fn type_path<'t>(reader: &Reader<'t>, ty: &syn::Type) -> Option<Tie<'t>> {
    match ty {
        syn::Type::Reference(r) => type_path(reader, &r.elem),
        syn::Type::Paren(t) => type_path(reader, &t.elem),
        syn::Type::Group(t) => type_path(reader, &t.elem),
        _ => plain_path(ty).map(|path| reader.path_tie(path, Namespace::Type)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Namespace {
    Type,
    /// A call or tuple pattern resolves a constructor, not a type name.
    Value,
}

/// A path that may tie a positional read to a struct.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Tie<'t> {
    pub path: Vec<&'t str>,
    /// Written with a leading `::`.
    pub rooted: bool,
    pub namespace: Namespace,
}

struct Reader<'t> {
    text: &'t str,
    out: Syntax<'t>,
    module: usize,
    /// The names bound in the function being read, innermost last, each with the struct path
    /// it is tied to when the binding says it.
    bindings: Vec<(&'t str, Option<Tie<'t>>)>,
}

impl<'t> Reader<'t> {
    /// An identifier's name as written in the text, `r#` left out.
    fn name(&self, ident: &syn::Ident) -> &'t str {
        let written = self.text.get(ident.span().byte_range()).unwrap_or("");
        written.strip_prefix("r#").unwrap_or(written)
    }

    fn names(&self, path: &syn::Path) -> Vec<&'t str> {
        path.segments.iter().map(|s| self.name(&s.ident)).collect()
    }

    fn path_tie(&self, path: &syn::Path, namespace: Namespace) -> Tie<'t> {
        Tie {
            path: self.names(path),
            rooted: path.leading_colon.is_some(),
            namespace,
        }
    }

    fn module_items(&mut self, items: &[syn::Item]) {
        for item in items {
            let concrete = match item {
                syn::Item::Struct(i) if !conditional(&i.attrs) => {
                    if matches!(i.fields, syn::Fields::Unnamed(_)) {
                        let name = self.name(&i.ident);
                        self.out.scopes[self.module].tuple_constructors.insert(name);
                    }
                    Some(&i.ident)
                }
                syn::Item::Enum(i) if !conditional(&i.attrs) => {
                    let name = self.name(&i.ident);
                    let variants = i
                        .variants
                        .iter()
                        .filter(|v| {
                            !conditional(&v.attrs) && matches!(v.fields, syn::Fields::Unnamed(_))
                        })
                        .map(|v| self.name(&v.ident))
                        .collect();
                    self.out.scopes[self.module]
                        .tuple_variants
                        .insert(name, variants);
                    Some(&i.ident)
                }
                syn::Item::Union(i) if !conditional(&i.attrs) => Some(&i.ident),
                _ => None,
            };
            if let Some(ident) = concrete {
                let name = self.name(ident);
                self.out.scopes[self.module].types.insert(name);
                if self.module == 0 {
                    self.out.defines.insert(name);
                }
            }
            // For the existing unqualified-name check, a readable alias is also a definition.
            if self.module == 0
                && let syn::Item::Type(i) = item
                && !conditional(&i.attrs)
                && plain_path(&i.ty).is_some()
            {
                self.out.defines.insert(self.name(&i.ident));
            }
            match item {
                syn::Item::Mod(m) => self.module_item(m, true),
                syn::Item::Use(u) if !conditional(&u.attrs) => {
                    self.out
                        .direct_uses
                        .insert(u.use_token.span.byte_range().start);
                    self.visit_item_use(u);
                }
                item => self.visit_item(item),
            }
        }
    }

    fn module_item(&mut self, m: &syn::ItemMod, direct: bool) {
        let name = self.name(&m.ident);
        self.out.modules.insert(name);
        let parent = self.module;
        if direct && !conditional(&m.attrs) && !m.attrs.iter().any(|a| a.path().is_ident("path")) {
            let inline = m.content.as_ref().map(|_| self.out.scopes.len());
            self.out.scopes[parent].children.insert(name, inline);
        }
        if let Some((brace, items)) = &m.content {
            self.out.inline_modules = true;
            self.module = self.out.scopes.len();
            self.out.scopes.push(Module {
                body: brace.span.join().byte_range(),
                parent,
                ..Module::default()
            });
            for attr in &m.attrs {
                self.visit_attribute(attr);
            }
            self.module_items(items);
            self.module = parent;
        } else {
            visit::visit_item_mod(self, m);
        }
    }

    fn alias(&mut self, alias: &syn::Ident, ty: &syn::Type, item: bool) {
        // An alias of a type the scan cannot read is not recorded, so a literal of its name is
        // of a name the scan cannot place.
        let alias = self.name(alias);
        if let Some(last) = plain_path(ty).and_then(|path| path.segments.last()) {
            let target = self.name(&last.ident);
            self.out.type_aliases.push((alias, target, item));
        }
    }

    /// Binds every name `pat` binds, tied to no struct.
    fn bind_untied(&mut self, pat: &syn::Pat) {
        struct Names<'r, 't>(&'r mut Reader<'t>);
        impl<'a> Visit<'a> for Names<'_, '_> {
            fn visit_pat_ident(&mut self, p: &'a syn::PatIdent) {
                let name = self.0.name(&p.ident);
                self.0.bindings.push((name, None));
                visit::visit_pat_ident(self, p);
            }
        }
        Names(self).visit_pat(pat);
    }

    /// Binds a parameter or a `let` pattern: a name with a type is tied to the type's path.
    fn bind(&mut self, pat: &syn::Pat, tie: Option<Tie<'t>>) {
        match (pat, tie) {
            (syn::Pat::Type(typed), _) => {
                let tie = type_path(self, &typed.ty);
                self.bind(&typed.pat, tie);
            }
            (syn::Pat::Ident(p), tie) if p.subpat.is_none() => {
                let name = self.name(&p.ident);
                self.bindings.push((name, tie));
            }
            (pat, _) => self.bind_untied(pat),
        }
    }

    /// The path a value is tied to: a possible constructor call, a name bound with its type,
    /// or `self` in a method. Anything else (a tuple, a field, a method's result, a closure's
    /// untyped argument) is tied to none.
    fn tie(&self, expr: &syn::Expr) -> Option<Tie<'t>> {
        match expr {
            syn::Expr::Paren(e) => self.tie(&e.expr),
            syn::Expr::Group(e) => self.tie(&e.expr),
            syn::Expr::Call(call) => match &*call.func {
                syn::Expr::Path(p) if p.qself.is_none() => {
                    Some(self.path_tie(&p.path, Namespace::Value))
                }
                _ => None,
            },
            syn::Expr::Path(p) if p.qself.is_none() => {
                let ident = p.path.get_ident()?;
                let name = self.name(ident);
                self.bindings
                    .iter()
                    .rev()
                    .find(|(bound, _)| *bound == name)
                    .and_then(|(_, tie)| tie.clone())
            }
            _ => None,
        }
    }

    fn tuple_field(&mut self, tie: Tie<'t>, position: u32, at: proc_macro2::Span) {
        self.out.tuple_fields.push(TupleField {
            tie,
            position,
            line: at.start().line,
            byte: at.byte_range().start,
        });
    }

    /// Reads a function with only its own parameters bound.
    fn function(&mut self, sig: &syn::Signature, read: impl FnOnce(&mut Self)) {
        let outer = std::mem::take(&mut self.bindings);
        for input in &sig.inputs {
            match input {
                syn::FnArg::Receiver(r) => {
                    let tie = match r.colon_token {
                        None => Some(Tie {
                            path: vec!["Self"],
                            rooted: false,
                            namespace: Namespace::Type,
                        }),
                        Some(_) => type_path(self, &r.ty),
                    };
                    self.bindings.push(("self", tie));
                }
                syn::FnArg::Typed(t) => {
                    let tie = type_path(self, &t.ty);
                    self.bind(&t.pat, tie);
                }
            }
        }
        read(self);
        self.bindings = outer;
    }

    fn build(&mut self, path: &syn::Path, how: How, brace: bool) {
        let span = match (&path.leading_colon, path.segments.first()) {
            (Some(colons), _) => colons.spans[0],
            (None, Some(first)) => first.ident.span(),
            (None, None) => return,
        };
        self.out.builds.push(Build {
            path: self.names(path),
            rooted: path.leading_colon.is_some(),
            how,
            brace,
            line: span.start().line,
            byte: span.byte_range().start,
        });
    }
}

impl<'a, 't> Visit<'a> for Reader<'t> {
    fn visit_expr_struct(&mut self, e: &'a syn::ExprStruct) {
        let how = if e.dot2_token.is_some() || e.rest.is_some() {
            How::Base
        } else {
            How::Full
        };
        self.build(&e.path, how, true);
        visit::visit_expr_struct(self, e);
    }

    fn visit_expr_call(&mut self, e: &'a syn::ExprCall) {
        if let syn::Expr::Path(p) = &*e.func {
            self.build(&p.path, How::Full, false);
        }
        visit::visit_expr_call(self, e);
    }

    fn visit_expr_path(&mut self, e: &'a syn::ExprPath) {
        // A value (and in a pattern, `Pat::Path`): a tuple struct's constructor, a unit struct.
        self.build(&e.path, How::Value, false);
        visit::visit_expr_path(self, e);
    }

    fn visit_pat_struct(&mut self, p: &'a syn::PatStruct) {
        // With `..` a pattern reads the fields it names; without, it lists them all.
        if p.rest.is_none() {
            self.build(&p.path, How::Full, false);
        }
        visit::visit_pat_struct(self, p);
    }

    fn visit_pat_tuple_struct(&mut self, p: &'a syn::PatTupleStruct) {
        if !p.elems.iter().any(|e| matches!(e, syn::Pat::Rest(_))) {
            self.build(&p.path, How::Full, false);
        }
        // The positions before a `..` are known; those after it count from an end the scan
        // does not know.
        if let Some(first) = p.path.segments.first() {
            let tie = self.path_tie(&p.path, Namespace::Value);
            let at = first.ident.span();
            let listed = p
                .elems
                .iter()
                .take_while(|e| !matches!(e, syn::Pat::Rest(_)));
            for (position, _) in (0..).zip(listed) {
                self.tuple_field(tie.clone(), position, at);
            }
        }
        visit::visit_pat_tuple_struct(self, p);
    }

    fn visit_expr_field(&mut self, e: &'a syn::ExprField) {
        if let syn::Member::Unnamed(index) = &e.member
            && let Some(tie) = self.tie(&e.base)
        {
            self.tuple_field(tie, index.index, index.span);
        }
        visit::visit_expr_field(self, e);
    }

    fn visit_item_fn(&mut self, i: &'a syn::ItemFn) {
        self.function(&i.sig, |this| visit::visit_item_fn(this, i));
    }

    fn visit_impl_item_fn(&mut self, i: &'a syn::ImplItemFn) {
        self.function(&i.sig, |this| visit::visit_impl_item_fn(this, i));
    }

    fn visit_trait_item_fn(&mut self, i: &'a syn::TraitItemFn) {
        self.function(&i.sig, |this| visit::visit_trait_item_fn(this, i));
    }

    fn visit_type_param(&mut self, p: &'a syn::TypeParam) {
        let name = self.name(&p.ident);
        self.out.scopes[self.module].type_params.insert(name);
        visit::visit_type_param(self, p);
    }

    fn visit_block(&mut self, b: &'a syn::Block) {
        let outer = self.bindings.len();
        let module = &mut self.out.scopes[self.module];
        for stmt in &b.stmts {
            if let syn::Stmt::Item(item) = stmt {
                match item {
                    syn::Item::Mod(_)
                    | syn::Item::ExternCrate(_)
                    | syn::Item::Struct(_)
                    | syn::Item::Enum(_)
                    | syn::Item::Union(_)
                    | syn::Item::Trait(_)
                    | syn::Item::TraitAlias(_)
                    | syn::Item::Type(_) => module.block_items = true,
                    syn::Item::Const(_)
                    | syn::Item::Fn(_)
                    | syn::Item::ForeignMod(_)
                    | syn::Item::Static(_) => {
                        module.block_values = true;
                    }
                    _ => {}
                }
            }
        }
        visit::visit_block(self, b);
        self.bindings.truncate(outer);
    }

    fn visit_local(&mut self, l: &'a syn::Local) {
        // The value is read before the pattern binds: `let v = v.0;`.
        for attr in &l.attrs {
            self.visit_attribute(attr);
        }
        if let Some(init) = &l.init {
            self.visit_local_init(init);
        }
        self.visit_pat(&l.pat);
        let tie = l.init.as_ref().and_then(|init| self.tie(&init.expr));
        self.bind(&l.pat, tie);
    }

    fn visit_expr_closure(&mut self, e: &'a syn::ExprClosure) {
        let outer = self.bindings.len();
        for input in &e.inputs {
            self.bind(input, None);
        }
        visit::visit_expr_closure(self, e);
        self.bindings.truncate(outer);
    }

    fn visit_arm(&mut self, a: &'a syn::Arm) {
        let outer = self.bindings.len();
        self.bind_untied(&a.pat);
        visit::visit_arm(self, a);
        self.bindings.truncate(outer);
    }

    fn visit_expr_for_loop(&mut self, e: &'a syn::ExprForLoop) {
        let outer = self.bindings.len();
        for attr in &e.attrs {
            self.visit_attribute(attr);
        }
        if let Some(label) = &e.label {
            self.visit_label(label);
        }
        self.visit_pat(&e.pat);
        self.visit_expr(&e.expr);
        self.bind_untied(&e.pat);
        self.visit_block(&e.body);
        self.bindings.truncate(outer);
    }

    fn visit_expr_let(&mut self, e: &'a syn::ExprLet) {
        visit::visit_expr_let(self, e);
        self.bind_untied(&e.pat);
    }

    fn visit_expr_if(&mut self, e: &'a syn::ExprIf) {
        // What `if let` binds holds in its first branch only.
        let outer = self.bindings.len();
        for attr in &e.attrs {
            self.visit_attribute(attr);
        }
        self.visit_expr(&e.cond);
        self.visit_block(&e.then_branch);
        self.bindings.truncate(outer);
        if let Some((_, otherwise)) = &e.else_branch {
            self.visit_expr(otherwise);
        }
    }

    fn visit_expr_while(&mut self, e: &'a syn::ExprWhile) {
        let outer = self.bindings.len();
        visit::visit_expr_while(self, e);
        self.bindings.truncate(outer);
    }

    fn visit_pat_ident(&mut self, p: &'a syn::PatIdent) {
        // `Marker` alone in a pattern parses as a binding; it may be a unit struct.
        if p.subpat.is_none() && p.by_ref.is_none() && p.mutability.is_none() {
            self.build(&syn::Path::from(p.ident.clone()), How::Value, false);
        }
        visit::visit_pat_ident(self, p);
    }

    fn visit_path(&mut self, p: &'a syn::Path) {
        let names = self.names(p);
        self.out.paths.push(names);
        visit::visit_path(self, p);
    }

    fn visit_item_impl(&mut self, i: &'a syn::ItemImpl) {
        let target = plain_path(&i.self_ty).map(|path| {
            let mut target = self.names(path);
            if path.leading_colon.is_some() {
                target.insert(0, "");
            }
            target
        });
        self.out.impls.push(Impl {
            body: i.brace_token.span.join().byte_range(),
            target,
        });
        visit::visit_item_impl(self, i);
    }

    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        self.module_item(m, false);
    }

    fn visit_item_extern_crate(&mut self, item: &'a syn::ItemExternCrate) {
        if self.name(&item.ident) != "mod_api"
            && let Some((_, name)) = &item.rename
        {
            let name = self.name(name);
            self.out.extern_aliases.insert(name);
        }
        visit::visit_item_extern_crate(self, item);
    }

    fn visit_item_struct(&mut self, i: &'a syn::ItemStruct) {
        let name = self.name(&i.ident);
        self.out.types.insert(name);
        visit::visit_item_struct(self, i);
    }

    fn visit_item_enum(&mut self, i: &'a syn::ItemEnum) {
        let name = self.name(&i.ident);
        self.out.types.insert(name);
        for variant in &i.variants {
            let name = self.name(&variant.ident);
            self.out.types.insert(name);
        }
        visit::visit_item_enum(self, i);
    }

    fn visit_item_union(&mut self, i: &'a syn::ItemUnion) {
        let name = self.name(&i.ident);
        self.out.types.insert(name);
        visit::visit_item_union(self, i);
    }

    fn visit_item_trait(&mut self, i: &'a syn::ItemTrait) {
        let name = self.name(&i.ident);
        self.out.traits.insert(name);
        visit::visit_item_trait(self, i);
    }

    fn visit_item_type(&mut self, i: &'a syn::ItemType) {
        self.alias(&i.ident, &i.ty, true);
        visit::visit_item_type(self, i);
    }

    fn visit_impl_item_type(&mut self, i: &'a syn::ImplItemType) {
        self.alias(&i.ident, &i.ty, false);
        visit::visit_impl_item_type(self, i);
    }

    fn visit_macro(&mut self, m: &'a syn::Macro) {
        let span = match &m.delimiter {
            syn::MacroDelimiter::Paren(d) => d.span.join(),
            syn::MacroDelimiter::Brace(d) => d.span.join(),
            syn::MacroDelimiter::Bracket(d) => d.span.join(),
        };
        self.out.macro_bodies.push(span.byte_range());
        visit::visit_macro(self, m);
    }
}

/// The deepest the parser is let recurse, counted by [`too_deep`].
pub(crate) const MAX_NESTING: usize = 256;

/// Keywords after which a token starts an operand rather than continuing one.
pub(crate) const KEYWORDS: &[&str] = &[
    "as", "async", "box", "break", "const", "continue", "dyn", "else", "enum", "extern", "fn",
    "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "static", "struct", "trait", "type", "unsafe", "use", "where", "while", "yield",
];

/// Keywords the parser reads what follows by recursion: `return return x`, `fn() -> fn()`,
/// `if … else if …`.
const RECURSIVE: &[&str] = &[
    "async", "box", "break", "dyn", "fn", "if", "impl", "let", "move", "ref", "return", "yield",
];

/// What the nesting count is inside: a bracket, a generic `<`, or a closure's parameters.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Frame {
    Bracket,
    Angle,
    /// A `<` straight after a literal, `)`, `]` or `?`, which compares or shifts, unless a type
    /// follows it: counted like a generic, but ended by a `,` like an expression.
    Compare,
    Params,
}

/// The first line where the parser would recurse deeper than [`MAX_NESTING`].
///
/// The count is taken along the path of nesting. Each open bracket, generic `<` and closure
/// counts until it closes, and so does each assignment, `@` and keyword the parser reads what
/// follows by recursion. A run of prefix operators counts until its operand ends, through
/// method calls and fields (`-x.f(…)`), and no further than the bracket after it. A count is
/// dropped only where the parser has certainly returned: at the bracket that encloses it
/// closing, at a `;`, at a match arm's `=>`, at a `}` that ends a statement, and at a `,`
/// directly inside a bracket (between elements, arms or fields). A `,` inside a generic or a
/// closure's parameters drops nothing, since generic arguments and parameters are listed with
/// commas. A `<` straight after a literal, `)`, `]` or `?` counts like any other, but a `,`
/// ends it, as it ends the comparison or shift it most likely is (`[1 << 0, 1 << 1]`).
///
/// This is a cheap filter, not the guarantee: the scan's parse runs in a child process first
/// ([`crate::child`]), so a parse that still runs out of stack ends the child, not the caller.
pub(crate) fn too_deep(tokens: &[(Token<'_>, usize)]) -> Option<usize> {
    let token = |at: usize| tokens.get(at).map(|(t, _)| *t);
    let punct = |at: usize, byte: u8| token(at) == Some(Token::Punct(byte));
    // Whether the token before `at` ends an operand, so an operator at `at` is binary.
    let operand_end = |at: usize| match at.checked_sub(1).and_then(token) {
        Some(Token::Literal(_) | Token::Punct(b')' | b']' | b'}' | b'?')) => true,
        Some(Token::Ident(name)) => !KEYWORDS.contains(&name),
        _ => false,
    };
    let mut frames: Vec<(usize, Frame)> = Vec::new();
    // The count that stays until a drop, and the prefix operators of the current operand.
    let (mut depth, mut run) = (0usize, 0usize);
    let mut skip = false;
    // Drops the generics and closure parameters above the innermost bracket, and the count of
    // the statement inside it.
    let end_statement = |frames: &mut Vec<(usize, Frame)>| {
        while frames
            .last()
            .is_some_and(|(_, frame)| *frame != Frame::Bracket)
        {
            frames.pop();
        }
        frames.last().map_or(0, |(before, _)| before + 1)
    };
    for (at, (current, line)) in tokens.iter().enumerate() {
        if skip {
            skip = false;
            continue;
        }
        match current {
            // A prefix operator's count ends where the bracket after it closes.
            Token::Punct(b'(' | b'[' | b'{') => {
                frames.push((depth, Frame::Bracket));
                depth += run + 1;
                run = 0;
            }
            Token::Punct(b')' | b']' | b'}') => {
                while let Some((before, frame)) = frames.pop() {
                    depth = before;
                    if frame == Frame::Bracket {
                        break;
                    }
                }
                run = 0;
                // A block that ends its statement: the next one starts afresh.
                if punct(at, b'}')
                    && match token(at + 1) {
                        None | Some(Token::Punct(b'}' | b';' | b',' | b'#' | b')' | b']')) => true,
                        Some(Token::Ident(next)) => !matches!(next, "else" | "as" | "in"),
                        _ => false,
                    }
                {
                    depth = end_statement(&mut frames);
                }
            }
            // A match arm's pattern and guard end at `=>`, generics and comparisons with them.
            Token::Punct(b';') | Token::Punct(b'=') if punct(at, b';') || punct(at + 1, b'>') => {
                depth = end_statement(&mut frames);
                run = 0;
            }
            Token::Punct(b',') => {
                while let Some((before, Frame::Compare)) = frames.last().copied() {
                    frames.pop();
                    depth = before;
                }
                if frames
                    .last()
                    .is_none_or(|(_, frame)| *frame == Frame::Bracket)
                {
                    depth = end_statement(&mut frames);
                }
                run = 0;
            }
            Token::Punct(b'<') if punct(at + 1, b'=') => run = 0,
            Token::Punct(b'<') => {
                // The second `<` of a shift (`1 << 2`) compares too.
                let compare = match at.checked_sub(1).and_then(token) {
                    Some(Token::Literal(_) | Token::Punct(b')' | b']' | b'?')) => true,
                    Some(Token::Punct(b'<')) => {
                        matches!(frames.last(), Some((_, Frame::Compare)))
                    }
                    _ => false,
                };
                let frame = if compare {
                    Frame::Compare
                } else {
                    Frame::Angle
                };
                frames.push((depth + run, frame));
                depth += run + 1;
                run = 0;
            }
            Token::Punct(b'>') => {
                // Not `->`, `=>` or `>=`: the close of the innermost generic, if one is open.
                if !(at >= 1 && (punct(at - 1, b'-') || punct(at - 1, b'=')))
                    && !punct(at + 1, b'=')
                    && let Some((before, Frame::Angle | Frame::Compare)) = frames.last().copied()
                {
                    frames.pop();
                    depth = before;
                }
                run = 0;
            }
            Token::Punct(b'|') => {
                if frames
                    .last()
                    .is_some_and(|(_, frame)| *frame == Frame::Params)
                {
                    // The parameters end; the closure's body follows.
                    if let Some((before, _)) = frames.pop() {
                        depth = before + 1;
                    }
                } else if operand_end(at) {
                    // A binary `|` or `||`.
                    skip = punct(at + 1, b'|');
                } else if punct(at + 1, b'|') {
                    // A closure with no parameters.
                    depth += run + 1;
                    skip = true;
                } else {
                    frames.push((depth + run, Frame::Params));
                    depth += run + 1;
                }
                run = 0;
            }
            Token::Punct(b'=')
                if !punct(at + 1, b'=')
                    && !punct(at + 1, b'>')
                    && !(at >= 1
                        && matches!(
                            token(at - 1),
                            Some(Token::Punct(b'=' | b'!' | b'<' | b'>'))
                        )) =>
            {
                depth += 1;
                run = 0;
            }
            Token::Punct(b'@') => {
                depth += 1;
                run = 0;
            }
            Token::Punct(b'-' | b'!' | b'*' | b'&' | b'.') if !operand_end(at) => run += 1,
            // A path's `::`, and a method call's or field's `.`, continue the operand.
            Token::Punct(b':' | b'.') => {}
            Token::Punct(_) => run = 0,
            Token::Ident(name) if RECURSIVE.contains(name) => depth += 1,
            _ => {}
        }
        if depth + run > MAX_NESTING {
            return Some(*line);
        }
    }
    None
}

/// The stack the parser runs on. Measured on 20 shapes nested as deep as [`too_deep`] accepts
/// (brackets, generics with and without commas, closures with one and two parameters,
/// references, `fn` types, assignments, `return`, `@`, `else if`, qualified paths after a
/// comparison, prefix operators on method calls, mixes of them), the deepest
/// needs 1.6 MiB in a release build and 1.5 MiB in a test build; this leaves a tenfold margin.
pub(crate) const PARSER_STACK: usize = 16 << 20;

/// Why a file was not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unread {
    /// It does not parse, at this line.
    Parse(usize),
    /// The parser's own stack could not be reserved.
    NoStack,
}

/// Parses `text` and reads it, on a thread with a stack of its own.
pub(crate) fn read(text: &str) -> Result<Syntax<'_>, Unread> {
    // The parser reads nesting by recursion. [`too_deep`] bounds it, and a stack of its own
    // leaves room for any file within that bound, wherever the scan is called from. Without one
    // the file is not parsed at all.
    std::thread::scope(|scope| {
        match std::thread::Builder::new()
            .stack_size(PARSER_STACK)
            .spawn_scoped(scope, || parse(text))
        {
            Ok(thread) => thread.join().unwrap_or(Err(Unread::NoStack)),
            Err(_) => Err(Unread::NoStack),
        }
    })
}

pub(crate) fn parse(text: &str) -> Result<Syntax<'_>, Unread> {
    // The parser skips a byte-order mark and a shebang line itself, which would shift every
    // byte it reports; blank them in a copy of the same length instead.
    let mut copy = text.to_owned();
    if copy.starts_with('\u{feff}') {
        copy.replace_range(..3, "   ");
    }
    if copy.starts_with("#!") && !copy.starts_with("#![") {
        let end = copy.find('\n').unwrap_or(copy.len());
        copy.replace_range(..end, &" ".repeat(end));
    }
    let parsed = syn::parse_str::<syn::File>(&copy);
    let result = match parsed {
        Ok(file) => {
            let mut reader = Reader {
                text,
                out: Syntax::default(),
                module: 0,
                bindings: Vec::new(),
            };
            reader.out.scopes.push(Module {
                body: 0..text.len(),
                ..Module::default()
            });
            for attr in &file.attrs {
                reader.visit_attribute(attr);
            }
            reader.module_items(&file.items);
            reader.out.scope_order = (0..reader.out.scopes.len()).collect();
            reader
                .out
                .scope_order
                .sort_by_key(|&scope| reader.out.scopes[scope].body.start);
            Ok(reader.out)
        }
        Err(error) => Err(Unread::Parse(error.span().start().line.max(1))),
    };
    // The parser keeps every text it read for its spans until told to forget them.
    proc_macro2::extra::invalidate_current_thread_spans();
    result
}
