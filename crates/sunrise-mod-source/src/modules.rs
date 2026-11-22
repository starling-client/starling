//! A crate's source tree as the tools walk it: each file with its module path and the `cfg`s it
//! is compiled under, following `mod` declarations, `#[path]`s and inline modules.

use quote::ToTokens;
use std::{
    fs,
    path::{Path, PathBuf},
};
use syn::{Item, Meta, punctuated::Punctuated};

/// A source file of a crate, with its module path and the `cfg`s it is compiled under.
#[derive(Clone)]
pub struct ModuleFile {
    pub module: String,
    pub path: PathBuf,
    /// The conditions of the `mod` declarations that lead to it, outermost first.
    pub cfg: Vec<String>,
}

/// One contribution to a module's items, from a file or an inline declaration in that file.
pub struct ModuleVariant {
    pub file: ModuleFile,
    /// The item indices from the file's root down to the inline module (empty for a file module).
    pub inline: Vec<usize>,
}

/// The `cfg` conditions in `attrs`, including conditional `cfg` attributes, except test-only gates.
pub fn conditions(attrs: &[syn::Attribute]) -> Vec<String> {
    cfg_conditions(attrs)
        .into_iter()
        .filter(|condition| !disabled_without_tests(condition))
        .collect()
}

/// The `cfg` conditions as predicates; a conditional attribute takes effect only under its guard.
pub fn cfg_conditions(attrs: &[syn::Attribute]) -> Vec<String> {
    let mut conditions = Vec::new();
    for attr in attrs {
        effective_attrs(&attr.meta, &mut Vec::new(), &mut |attr, guards| {
            let predicate = if let Meta::List(list) = attr
                && list.path.is_ident("cfg")
                && let Ok(predicate) = list.parse_args::<Meta>()
            {
                predicate.to_token_stream().to_string()
            } else if attr.path().is_ident("test") || attr.path().is_ident("bench") {
                "test".to_owned()
            } else {
                return;
            };
            conditions.push(if guards.is_empty() {
                predicate
            } else {
                format!("any(not(all({})), {predicate})", guards.join(", "))
            });
        });
    }
    conditions
}

fn effective_attrs(meta: &Meta, guards: &mut Vec<String>, f: &mut impl FnMut(&Meta, &[String])) {
    if let Meta::List(list) = meta
        && list.path.is_ident("cfg_attr")
        && let Ok(args) = list.parse_args_with(Punctuated::<Meta, syn::Token![,]>::parse_terminated)
    {
        let mut args = args.iter();
        if let Some(guard) = args.next() {
            guards.push(guard.to_token_stream().to_string());
            for attr in args {
                effective_attrs(attr, guards, f);
            }
            guards.pop();
        }
    } else {
        f(meta, guards);
    }
}

/// Evaluate a written cfg predicate when its atoms are known. An unknown atom matters only if
/// neither `all` nor `any` can decide the result without it.
pub fn evaluate_cfg(predicate: &str, mut atom: impl FnMut(&str) -> Option<bool>) -> Option<bool> {
    fn eval(meta: &Meta, atom: &mut impl FnMut(&str) -> Option<bool>) -> Option<bool> {
        if let Meta::List(list) = meta {
            let args = list
                .parse_args_with(Punctuated::<Meta, syn::Token![,]>::parse_terminated)
                .ok()?;
            if list.path.is_ident("not") && args.len() == 1 {
                return eval(args.first()?, atom).map(|value| !value);
            }
            let all = list.path.is_ident("all");
            if all || list.path.is_ident("any") {
                let mut unknown = false;
                for arg in &args {
                    match eval(arg, atom) {
                        Some(value) if value != all => return Some(value),
                        Some(_) => {}
                        None => unknown = true,
                    }
                }
                return (!unknown).then_some(all);
            }
        }
        if meta.path().is_ident("true") {
            Some(true)
        } else if meta.path().is_ident("false") {
            Some(false)
        } else {
            atom(&meta.to_token_stream().to_string())
        }
    }
    eval(&syn::parse_str::<Meta>(predicate).ok()?, &mut atom)
}

fn disabled_without_tests(predicate: &str) -> bool {
    evaluate_cfg(predicate, |atom| (atom == "test").then_some(false)) == Some(false)
}

/// Whether a node cannot be compiled with `test` off, including gates applied by `cfg_attr`.
pub fn test_only(attrs: &[syn::Attribute]) -> bool {
    cfg_conditions(attrs)
        .iter()
        .any(|cfg| disabled_without_tests(cfg))
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
        pending: vec![(
            ModuleFile {
                module,
                path,
                cfg: Vec::new(),
            },
            dir,
        )],
        variants: Vec::new(),
    };
    while let Some((mut file, dir)) = walker.pending.pop() {
        // A target with only conditional paths need not have a conventional file.
        if !file.path.is_file() {
            continue;
        }
        let source =
            fs::read_to_string(&file.path).map_err(|e| format!("{}: {e}", file.path.display()))?;
        let parsed = parse(&file.path, &source)?;
        if !walker.keep(&parsed.attrs) {
            continue;
        }
        file.cfg.extend(walker.conditions(&parsed.attrs));
        let here = file.path.parent().expect("a source file has a folder");
        walker.children(&file, &parsed.items, here, &dir, &mut Vec::new());
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
    /// Each file's declaration and the directory Rust uses for conventional children.
    pending: Vec<(ModuleFile, PathBuf)>,
    variants: Vec<ModuleVariant>,
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
    ) {
        for (index, item) in items.iter().enumerate() {
            if let Item::Macro(included) = item
                && included.mac.path.is_ident("include")
                && self.keep(&included.attrs)
                && let Ok(relative) = syn::parse2::<syn::LitStr>(included.mac.tokens.clone())
            {
                let mut included_file = file.clone();
                included_file.path = file
                    .path
                    .parent()
                    .expect("a source file has a folder")
                    .join(relative.value());
                included_file.cfg.extend(self.conditions(&included.attrs));
                self.pending.push((included_file, dir.to_owned()));
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
                            [] => "all()".to_owned(),
                            [guard] => guard.clone(),
                            _ => format!("all({})", guards.join(", ")),
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
                    };
                    inline.push(index);
                    self.children(&child, inner, &inner_dir, &inner_dir, inline);
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
                    self.pending.push((
                        ModuleFile {
                            module: module.clone(),
                            path,
                            cfg,
                        },
                        child_dir,
                    ));
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
