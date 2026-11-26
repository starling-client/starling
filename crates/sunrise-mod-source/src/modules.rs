//! A crate's source tree as the tools walk it: each file with its module path and the `cfg`s it
//! is compiled under, following `mod` declarations, `#[path]`s, inline modules and literal
//! item-position `include!`s.

use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::{
    fs,
    path::{Path, PathBuf},
};
use syn::{
    Item, Meta, Token,
    parse::{Parse, ParseStream, Parser},
    punctuated::Punctuated,
};

/// A source file of a crate, with its module path and the `cfg`s it is compiled under.
#[derive(Clone)]
pub struct ModuleFile {
    pub module: String,
    pub path: PathBuf,
    /// The conditions of the `mod` and `include!` declarations that lead to it, outermost first.
    pub cfg: Vec<String>,
    /// Whether this file supplies items to the containing module via `include!`.
    pub included: bool,
}

/// One contribution to a module's items, from a file or an inline declaration in that file.
pub struct ModuleVariant {
    pub file: ModuleFile,
    /// The item indices from the file's root down to the inline module (empty for a file module).
    pub inline: Vec<usize>,
}

/// A normalized Rust cfg predicate. Both attribute extraction and stored module predicates use
/// this parser, and emitted cfg attributes quote the same tree.
#[derive(Clone)]
pub struct Cfg {
    predicate: Predicate,
    // Guarded rows in the internal catalog are keyed by their written predicates. Preserve
    // their spelling even when the emitter no longer needs the insignificant punctuation.
    written: String,
}

#[derive(Clone)]
enum Predicate {
    Bool(bool),
    All(Vec<Cfg>),
    Any(Vec<Cfg>),
    Not(Box<Cfg>),
    Atom(Meta),
}

impl Parse for Cfg {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(syn::LitBool) {
            let value = input.parse::<syn::LitBool>()?.value;
            return Ok(Self {
                predicate: Predicate::Bool(value),
                written: value.to_string(),
            });
        }
        let meta: Meta = input.parse()?;
        let written = meta.to_token_stream().to_string();
        let predicate = match &meta {
            Meta::List(list) if list.path.is_ident("all") || list.path.is_ident("any") => {
                let args = list.parse_args_with(Punctuated::<Self, Token![,]>::parse_terminated)?;
                if list.path.is_ident("all") {
                    Predicate::All(args.into_iter().collect())
                } else {
                    Predicate::Any(args.into_iter().collect())
                }
            }
            Meta::List(list) if list.path.is_ident("not") => {
                Predicate::Not(Box::new(list.parse_args_with(cfg_arg)?))
            }
            _ => Predicate::Atom(meta),
        };
        Ok(Self { predicate, written })
    }
}

impl ToTokens for Cfg {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(match &self.predicate {
            Predicate::Bool(value) => quote!(#value),
            Predicate::All(args) => quote!(all(#(#args),*)),
            Predicate::Any(args) => quote!(any(#(#args),*)),
            Predicate::Not(arg) => quote!(not(#arg)),
            Predicate::Atom(meta) => meta.to_token_stream(),
        });
    }
}

impl Cfg {
    fn all_guards(guards: &[Self]) -> Self {
        let text = guards
            .iter()
            .map(|guard| guard.written.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Self {
            written: format!("all({text})"),
            predicate: Predicate::All(guards.to_vec()),
        }
    }

    /// Evaluate the predicate with the given cfg facts. An unknown atom matters only if neither
    /// `all` nor `any` can decide without it.
    pub fn evaluate(&self, mut atom: impl FnMut(&str) -> Option<bool>) -> Option<bool> {
        fn eval(cfg: &Cfg, atom: &mut impl FnMut(&str) -> Option<bool>) -> Option<bool> {
            match &cfg.predicate {
                Predicate::Bool(value) => Some(*value),
                Predicate::Not(arg) => eval(arg, atom).map(|value| !value),
                Predicate::Atom(meta) => atom(&atom_key(meta)),
                Predicate::All(args) | Predicate::Any(args) => {
                    let all = matches!(&cfg.predicate, Predicate::All(_));
                    let mut unknown = false;
                    for arg in args {
                        match eval(arg, atom) {
                            Some(value) if value != all => return Some(value),
                            Some(_) => {}
                            None => unknown = true,
                        }
                    }
                    (!unknown).then_some(all)
                }
            }
        }
        eval(self, &mut atom)
    }

    /// Whether the predicate is false without `test`, regardless of other build options.
    pub fn disabled_without_tests(&self) -> bool {
        self.evaluate(|atom| (atom == "test").then_some(false)) == Some(false)
    }
}

/// Parse one predicate from a cfg attribute or a `not`, with Rust's optional trailing comma.
fn cfg_arg(input: ParseStream<'_>) -> syn::Result<Cfg> {
    let cfg = input.parse()?;
    input.parse::<Option<Token![,]>>()?;
    Ok(cfg)
}

/// The predicates imposed by `attrs`, including cfgs from arbitrarily nested `cfg_attr`s.
/// A conditional attribute applies only if all the guards enclosing it hold.
pub fn cfg_predicates(attrs: &[syn::Attribute]) -> Vec<Cfg> {
    let mut conditions = Vec::new();
    for attr in attrs {
        effective_attrs(&attr.meta, &mut Vec::new(), &mut |attr, guards| {
            let Some(predicate) = attr_cfg(attr) else {
                return;
            };
            conditions.push(if guards.is_empty() {
                predicate
            } else {
                let when = Cfg::all_guards(guards);
                let skip = Cfg {
                    written: format!("not({})", when.written),
                    predicate: Predicate::Not(Box::new(when)),
                };
                Cfg {
                    written: format!("any({}, {})", skip.written, predicate.written),
                    predicate: Predicate::Any(vec![skip, predicate]),
                }
            });
        });
    }
    conditions
}

fn attr_cfg(attr: &Meta) -> Option<Cfg> {
    if let Meta::List(list) = attr
        && list.path.is_ident("cfg")
    {
        list.parse_args_with(cfg_arg).ok()
    } else if attr.path().is_ident("test") || attr.path().is_ident("bench") {
        Some(Cfg {
            predicate: Predicate::Atom(syn::parse_quote!(test)),
            written: "test".into(),
        })
    } else {
        None
    }
}

/// The attributes of an item in one resolved build, or `None` when that build does not contain it.
/// Conditional attributes are expanded in their original order; resolved `cfg` gates are gone.
/// Used when moving an item into a crate that does not share the original crate's cfgs.
pub fn configured_attrs(
    attrs: &[syn::Attribute],
    atom: impl Fn(&str) -> bool,
) -> Option<Vec<syn::Attribute>> {
    let mut active = Vec::new();
    for attr in attrs {
        let mut exists = true;
        effective_attrs(&attr.meta, &mut Vec::new(), &mut |meta, guards| {
            if guards
                .iter()
                .all(|guard| guard.evaluate(|name| Some(atom(name))) == Some(true))
            {
                if let Some(predicate) = attr_cfg(meta) {
                    exists &= predicate.evaluate(|name| Some(atom(name))) == Some(true);
                } else if guards.is_empty() {
                    active.push(attr.clone());
                } else {
                    active.push(syn::Attribute {
                        meta: meta.clone(),
                        ..attr.clone()
                    });
                }
            }
        });
        if !exists {
            return None;
        }
    }
    Some(active)
}

fn effective_attrs(meta: &Meta, guards: &mut Vec<Cfg>, f: &mut impl FnMut(&Meta, &[Cfg])) {
    if let Meta::List(list) = meta
        && list.path.is_ident("cfg_attr")
        && let Ok((guard, attrs)) = list.parse_args_with(|input: ParseStream<'_>| {
            let guard = input.parse::<Cfg>()?;
            let attrs = if input.is_empty() {
                Punctuated::new()
            } else {
                input.parse::<Token![,]>()?;
                Punctuated::<Meta, Token![,]>::parse_terminated(input)?
            };
            Ok((guard, attrs))
        })
    {
        guards.push(guard);
        for attr in &attrs {
            effective_attrs(attr, guards, f);
        }
        guards.pop();
    } else {
        f(meta, guards);
    }
}

/// The `cfg` conditions in `attrs`, including conditional `cfg` attributes, except test-only gates.
pub fn conditions(attrs: &[syn::Attribute]) -> Vec<String> {
    cfg_predicates(attrs)
        .into_iter()
        .filter(|predicate| !predicate.disabled_without_tests())
        .map(|predicate| predicate.written)
        .collect()
}

/// The normalized conditions in `attrs` as text, including test-only gates.
pub fn cfg_conditions(attrs: &[syn::Attribute]) -> Vec<String> {
    cfg_predicates(attrs)
        .into_iter()
        .map(|predicate| predicate.written)
        .collect()
}

/// Read a single variable from rustc's printed cfg set in the form predicates look it up.
pub fn cfg_atom(text: &str) -> syn::Result<String> {
    let meta = syn::parse_str::<Meta>(text)?;
    if matches!(meta, Meta::List(_)) {
        return Err(syn::Error::new_spanned(meta, "expected a cfg atom"));
    }
    Ok(atom_key(&meta))
}

fn atom_key(meta: &Meta) -> String {
    if let Meta::NameValue(entry) = meta
        && let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) = &entry.value
    {
        let path = &entry.path;
        let value = syn::LitStr::new(&value.value(), value.span());
        quote!(#path = #value).to_string()
    } else {
        meta.to_token_stream().to_string()
    }
}

/// Evaluate a written cfg predicate with the given cfg facts.
pub fn evaluate_cfg(predicate: &str, atom: impl FnMut(&str) -> Option<bool>) -> Option<bool> {
    cfg_arg.parse_str(predicate).ok()?.evaluate(atom)
}

/// Whether a node cannot be compiled with `test` off, including gates applied by `cfg_attr`.
pub fn test_only(attrs: &[syn::Attribute]) -> bool {
    cfg_predicates(attrs)
        .iter()
        .any(Cfg::disabled_without_tests)
}

/// An item-position standard `include!` of one literal path, with the attributes on the
/// invocation. The caller resolves the path relative to the physical file containing the item.
/// A bare `include!` is the standard macro unless `shadowed` ([`include_shadowed`]); one written
/// `std::include!` or `core::include!` always is. Any other invocation is an opaque macro.
pub fn literal_include(item: &Item, shadowed: bool) -> Option<(syn::LitStr, &[syn::Attribute])> {
    let Item::Macro(item) = item else { return None };
    let path = &item.mac.path;
    let standard = if path.leading_colon.is_none() && path.is_ident("include") {
        !shadowed
    } else {
        let segments: Vec<_> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        matches!(segments.as_slice(), [root, name] if (root == "std" || root == "core") && name == "include")
    };
    if !standard {
        return None;
    }
    let path = item
        .mac
        .parse_body_with(|input: ParseStream<'_>| {
            let path = input.parse::<syn::LitStr>()?;
            input.parse::<Option<Token![,]>>()?;
            Ok(path)
        })
        .ok()?;
    Some((path, &item.attrs))
}

/// Whether `items` define a `macro_rules! include` or import a name `include`. The tools then
/// read a bare `include!` there, and in the modules inside, as that macro.
pub fn include_shadowed(items: &[Item]) -> bool {
    fn binds(tree: &syn::UseTree) -> bool {
        match tree {
            syn::UseTree::Path(path) => binds(&path.tree),
            syn::UseTree::Name(name) => name.ident == "include",
            syn::UseTree::Rename(rename) => rename.rename == "include",
            syn::UseTree::Group(group) => group.items.iter().any(binds),
            syn::UseTree::Glob(_) => false,
        }
    }
    items.iter().any(|item| match item {
        Item::Macro(defined) => {
            defined.mac.path.is_ident("macro_rules")
                && defined
                    .ident
                    .as_ref()
                    .is_some_and(|ident| ident == "include")
        }
        Item::Use(used) => binds(&used.tree),
        _ => false,
    })
}

/// Whether some build can hold every predicate in `conditions` at once: an assignment of the
/// atoms they test (a name or `key = "value"`) under which none of them is false. Atoms vary
/// independently; `test` is false unless `include_tests`.
pub fn may_compile(conditions: &[String], include_tests: bool) -> bool {
    let mut atoms: Vec<String> = Vec::new();
    for condition in conditions {
        evaluate_cfg(condition, |atom| {
            if !(atoms.iter().any(|known| known == atom) || (!include_tests && atom == "test")) {
                atoms.push(atom.to_owned());
            }
            None
        });
    }
    fn search(
        conditions: &[String],
        atoms: &[String],
        values: &mut Vec<bool>,
        include_tests: bool,
    ) -> bool {
        let refuted = conditions.iter().any(|condition| {
            evaluate_cfg(condition, |atom| {
                if !include_tests && atom == "test" {
                    return Some(false);
                }
                let at = atoms.iter().position(|known| known == atom)?;
                values.get(at).copied()
            }) == Some(false)
        });
        if refuted {
            return false;
        }
        if values.len() == atoms.len() {
            return true;
        }
        [true, false].into_iter().any(|value| {
            values.push(value);
            let found = search(conditions, atoms, values, include_tests);
            values.pop();
            found
        })
    }
    search(conditions, &atoms, &mut Vec::new(), include_tests)
}

/// The files of `module` read from `path`, with conventional children in `dir`. With `every_build`,
/// a module behind a `cfg` is left out with its descendants. Unless `include_tests`, test-only
/// modules are left out, including files gated by inner attributes.
pub fn walk_module_files(
    module: String,
    path: PathBuf,
    dir: PathBuf,
    every_build: bool,
    include_tests: bool,
    parse: impl Fn(&Path, &str) -> Result<syn::File, String>,
) -> Result<Vec<ModuleFile>, String> {
    Ok(
        walk_module_variants(module, path, dir, every_build, include_tests, parse)?
            .into_iter()
            .filter(|variant| variant.inline.is_empty())
            .map(|variant| variant.file)
            .collect(),
    )
}

/// Like [`walk_module_files`], but also reports inline modules with their own conditions.
pub fn walk_module_variants(
    module: String,
    path: PathBuf,
    dir: PathBuf,
    every_build: bool,
    include_tests: bool,
    parse: impl Fn(&Path, &str) -> Result<syn::File, String>,
) -> Result<Vec<ModuleVariant>, String> {
    let mut walker = Walker {
        every_build,
        include_tests,
        pending: vec![Pending {
            file: ModuleFile {
                module,
                path,
                cfg: Vec::new(),
                included: false,
            },
            dir,
            ancestors: Vec::new(),
            shadowed: false,
        }],
        variants: Vec::new(),
    };
    while let Some(Pending {
        mut file,
        dir,
        mut ancestors,
        shadowed,
    }) = walker.pending.pop()
    {
        // A target with only conditional paths need not have a conventional file. A literal
        // include has no such fallback: a fragment some build compiles must be readable, and
        // one no build can compile is never opened.
        if file.included && !may_compile(&file.cfg, include_tests) {
            continue;
        }
        if !file.included && !file.path.is_file() {
            continue;
        }
        let physical = file
            .path
            .canonicalize()
            .map_err(|e| format!("{}: {e}", file.path.display()))?;
        if ancestors.contains(&physical) {
            return Err(format!("{}: source inclusion cycle", file.path.display()));
        }
        ancestors.push(physical);
        let source =
            fs::read_to_string(&file.path).map_err(|e| format!("{}: {e}", file.path.display()))?;
        let parsed = parse(&file.path, &source)?;
        if !walker.keep(&parsed.attrs) {
            continue;
        }
        file.cfg.extend(walker.conditions(&parsed.attrs));
        let here = file.path.parent().expect("a source file has a folder");
        walker.children(
            &file,
            &parsed.items,
            here,
            &dir,
            &mut Vec::new(),
            &ancestors,
            shadowed,
        );
        walker.variants.push(ModuleVariant {
            file,
            inline: Vec::new(),
        });
    }
    walker
        .variants
        .sort_by(|a, b| a.file.path.cmp(&b.file.path));
    Ok(walker.variants)
}

struct Walker {
    every_build: bool,
    include_tests: bool,
    pending: Vec<Pending>,
    variants: Vec<ModuleVariant>,
}

/// A file to read, the directory Rust uses for its conventional children, its physical
/// ancestors and whether a bare `include!` reaching it names a shadowing macro.
struct Pending {
    file: ModuleFile,
    dir: PathBuf,
    ancestors: Vec<PathBuf>,
    shadowed: bool,
}

impl Walker {
    fn keep(&self, attrs: &[syn::Attribute]) -> bool {
        (self.include_tests || !test_only(attrs)) && !(self.every_build && gated(attrs))
    }

    fn conditions(&self, attrs: &[syn::Attribute]) -> Vec<String> {
        if self.include_tests {
            cfg_conditions(attrs)
        } else {
            conditions(attrs)
        }
    }

    /// Look through inline modules and literal `include!`s as well as regular file declarations.
    fn children(
        &mut self,
        file: &ModuleFile,
        items: &[Item],
        here: &Path,
        dir: &Path,
        inline: &mut Vec<usize>,
        ancestors: &[PathBuf],
        shadowed: bool,
    ) {
        let shadowed = shadowed || include_shadowed(items);
        for (index, item) in items.iter().enumerate() {
            if let Some((relative, attrs)) = literal_include(item, shadowed)
                && self.keep(attrs)
            {
                let mut included_file = file.clone();
                included_file.path = file
                    .path
                    .parent()
                    .expect("a source file has a folder")
                    .join(relative.value());
                included_file.cfg.extend(self.conditions(attrs));
                included_file.included = true;
                self.pending.push(Pending {
                    file: included_file,
                    dir: dir.to_owned(),
                    ancestors: ancestors.to_vec(),
                    shadowed,
                });
            }
            let Item::Mod(declared) = item else { continue };
            if !self.keep(&declared.attrs) {
                continue;
            }
            let name = declared.ident.to_string();
            let module = format!("{}::{name}", file.module);
            let mut cfg = file.cfg.clone();
            cfg.extend(self.conditions(&declared.attrs));
            let mut paths = Vec::new();
            for attr in &declared.attrs {
                effective_attrs(&attr.meta, &mut Vec::new(), &mut |attr, guards| {
                    if let Meta::NameValue(value) = attr
                        && value.path.is_ident("path")
                        && let syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(path),
                            ..
                        }) = &value.value
                    {
                        let guard = match guards {
                            [guard] => guard.written.clone(),
                            _ => Cfg::all_guards(guards).written,
                        };
                        paths.push((Some(path.value()), guard));
                    }
                });
            }
            if paths.is_empty() {
                paths.push((None, String::new()));
            } else {
                let alternatives: Vec<_> = paths.iter().map(|(_, guard)| guard.as_str()).collect();
                paths.push((None, format!("not(any({}))", alternatives.join(", "))));
            }
            for (explicit, guard) in paths {
                let mut cfg = cfg.clone();
                match evaluate_cfg(&guard, |atom| {
                    (!self.include_tests && atom == "test").then_some(false)
                }) {
                    Some(false) => continue,
                    Some(true) => {}
                    None if !guard.is_empty() => cfg.push(guard.to_owned()),
                    None => {}
                }
                if let Some((_, inner)) = &declared.content {
                    let inner_dir = dir.join(explicit.as_deref().unwrap_or(&name));
                    let child = ModuleFile {
                        module: module.clone(),
                        path: file.path.clone(),
                        cfg,
                        included: file.included,
                    };
                    inline.push(index);
                    self.children(
                        &child, inner, &inner_dir, &inner_dir, inline, ancestors, shadowed,
                    );
                    self.variants.push(ModuleVariant {
                        file: child,
                        inline: inline.clone(),
                    });
                    inline.pop();
                } else {
                    let (path, child_dir) = if let Some(relative) = explicit {
                        let child = here.join(relative);
                        let dir = child
                            .parent()
                            .expect("a module file has a folder")
                            .to_owned();
                        (child, dir)
                    } else if dir.join(format!("{name}.rs")).is_file() {
                        (dir.join(format!("{name}.rs")), dir.join(&name))
                    } else {
                        (dir.join(&name).join("mod.rs"), dir.join(&name))
                    };
                    self.pending.push(Pending {
                        file: ModuleFile {
                            module: module.clone(),
                            path,
                            cfg,
                            included: false,
                        },
                        dir: child_dir,
                        ancestors: ancestors.to_vec(),
                        shadowed,
                    });
                }
            }
        }
    }
}

/// Whether `attrs` hold a `cfg` (or `cfg_attr`): the item is compiled only in some builds.
pub fn gated(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(root: &Path, include_tests: bool) -> Result<Vec<String>, String> {
        let files = walk_module_files(
            "lower".into(),
            root.join("lib.rs"),
            root.to_owned(),
            false,
            include_tests,
            |path, text| syn::parse_file(text).map_err(|e| format!("{}: {e}", path.display())),
        )?;
        Ok(files
            .into_iter()
            .map(|file| file.path.strip_prefix(root).unwrap().display().to_string())
            .collect())
    }

    /// A fragment no build can compile under its gates is never opened, tests or not; only the
    /// standard `include!` is followed, written bare or as `std::include!`.
    #[test]
    fn the_walk_opens_only_compilable_standard_includes() {
        let root = tempfile::tempdir().unwrap();
        let put = |name: &str, text: &str| fs::write(root.path().join(name), text).unwrap();
        put(
            "lib.rs",
            r#"
#[cfg(unix)]
include!("unix.rs");
#[cfg(false)]
include!("never.rs");
std::include!("qualified.rs");
mod shadow {
    macro_rules! include { ($($t:tt)*) => {}; }
    include!("no such file");
}
"#,
        );
        put("unix.rs", "#[cfg(not(unix))]\ninclude!(\"gone.rs\");\n");
        put("qualified.rs", "fn qualified() {}\n");
        for include_tests in [false, true] {
            let mut files = walk(root.path(), include_tests).unwrap();
            files.sort();
            assert_eq!(files, ["lib.rs", "qualified.rs", "unix.rs"]);
        }
    }

    #[test]
    fn contradictory_gates_cannot_compile() {
        let gates = |gates: &[&str]| gates.iter().map(|g| (*g).to_owned()).collect::<Vec<_>>();
        assert!(may_compile(&gates(&["unix", "feature = \"a\""]), false));
        assert!(!may_compile(&gates(&["unix", "not(unix)"]), false));
        assert!(!may_compile(&gates(&["false"]), true));
        assert!(!may_compile(&gates(&["test"]), false));
        assert!(may_compile(&gates(&["test"]), true));
        assert!(!may_compile(
            &gates(&["any(a, b)", "not(a)", "not(b)"]),
            false
        ));
    }
}
