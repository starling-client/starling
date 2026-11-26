//! Provider roles and built-in capability metadata derived from the public keys.
use crate::index::{Found, Index};
use quote::ToTokens;
use std::path::Path;
use syn::Item;

/// A capability key's shape: what its providers are registered by, its handler trait as
/// written (`ResourceHandler<ItemResource>`), and an entity key's lookup context.
#[derive(PartialEq)]
pub(crate) struct KeyShape {
    pub(crate) by: &'static str,
    pub(crate) handler: String,
    pub(crate) context: String,
}

/// Every exported capability declaration, including keys sharing a handler shape.
fn key_declarations(index: &mut Index<'_>) -> Result<Vec<(String, KeyShape)>, String> {
    let api = index.module("mod_api", &[])?;
    let names: Vec<String> = index.modules[api]
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Use(item) if matches!(item.vis, syn::Visibility::Public(_)) => Some(&item.tree),
            _ => None,
        })
        .flat_map(use_names)
        .collect();
    let mut declarations = Vec::new();
    for name in names {
        let Ok(Found::Item(id)) = index.resolve(api, &name) else {
            continue;
        };
        let Item::Const(item) = index.item(id) else {
            continue;
        };
        let syn::Type::Path(path) = &*item.ty else {
            continue;
        };
        let Some(last) = path.path.segments.last() else {
            continue;
        };
        let by = match last.ident.to_string().as_str() {
            "CapabilityKey" => "Block",
            "ItemCapabilityKey" => "Item",
            "EntityCapabilityKey" => "Entity",
            _ => continue,
        };
        let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
            continue;
        };
        let mut args = args.args.iter();
        let Some(syn::GenericArgument::Type(syn::Type::TraitObject(object))) = args.next() else {
            continue;
        };
        let context = match args.next() {
            Some(syn::GenericArgument::Type(ty)) => written(ty),
            _ => "()".to_string(),
        };
        let shape = KeyShape {
            by,
            handler: trait_name(object)?,
            context,
        };
        declarations.push((name, shape));
    }
    Ok(declarations)
}

/// Distinct provider shapes, in the order their keys are exported.
pub(crate) fn key_shapes(index: &mut Index<'_>) -> Result<Vec<KeyShape>, String> {
    let mut shapes = Vec::new();
    for (_, shape) in key_declarations(index)? {
        if !shapes.contains(&shape) {
            shapes.push(shape);
        }
    }
    Ok(shapes)
}

impl KeyShape {
    /// Its provider trait, as an owner (`BlockCapabilityProvider<dyn ResourceHandler<ItemResource>>`).
    pub(crate) fn provider(&self) -> String {
        match self.by {
            "Entity" => format!(
                "EntityCapabilityProvider<dyn {}, {}>",
                self.handler, self.context
            ),
            by => format!("{by}CapabilityProvider<dyn {}>", self.handler),
        }
    }
}

/// Every name a `use` tree brings in.
pub(crate) fn use_names(tree: &syn::UseTree) -> Vec<String> {
    match tree {
        syn::UseTree::Path(path) => use_names(&path.tree),
        syn::UseTree::Name(name) => vec![name.ident.to_string()],
        syn::UseTree::Rename(rename) => vec![rename.rename.to_string()],
        syn::UseTree::Group(group) => group.items.iter().flat_map(use_names).collect(),
        syn::UseTree::Glob(_) => Vec::new(),
    }
}

/// A trait object's trait as the generator names it: its last path segment, with its type
/// arguments as written (`ResourceHandler<ItemResource>`).
fn trait_name(object: &syn::TypeTraitObject) -> Result<String, String> {
    object
        .bounds
        .iter()
        .find_map(|bound| match bound {
            syn::TypeParamBound::Trait(bound) => bound.path.segments.last().map(|s| {
                let args = match &s.arguments {
                    syn::PathArguments::AngleBracketed(args) => format!(
                        "<{}>",
                        args.args.iter().map(written).collect::<Vec<_>>().join(", ")
                    ),
                    _ => String::new(),
                };
                format!("{}{args}", s.ident)
            }),
            _ => None,
        })
        .ok_or_else(|| format!("dyn {}", object.to_token_stream()))
}

/// Tokens as written, without the spaces `quote` puts around punctuation
/// (`Option<Direction>`, `dyn ResourceHandler<ItemResource>`).
fn written(tokens: &impl ToTokens) -> String {
    tokens
        .to_token_stream()
        .to_string()
        .replace(" < ", "<")
        .replace("< ", "<")
        .replace(" <", "<")
        .replace(" >", ">")
        .replace(" ,", ",")
        .replace("( )", "()")
}

pub(crate) const OUTPUT: &str = "builtin_capability_keys.rs";

/// The built-in capability metadata, and the source files read to derive it.
pub(crate) fn generate(
    root: &Path,
    included: &[(&str, &str)],
) -> Result<(String, Vec<std::path::PathBuf>), String> {
    let index = &mut crate::index::game(root, included);
    let declarations = key_declarations(index)?;
    let entries = declarations
        .iter()
        .map(|(name, shape)| {
            format!(
                "(ProvidedBy::{}, crate::{name}.name, crate::{name}.binding()),",
                shape.by
            )
        })
        .collect::<String>();
    let keys = format!(
        "// @generated by sunrise-mod-api-gen from mod_api's capability keys; do not edit.\n\
         pub(crate) fn builtin_capabilities() -> impl Iterator<Item = (ProvidedBy, &'static str, (TypeId, &'static str))> {{\n\
         [{entries}].into_iter()\n}}\n"
    );
    Ok((keys, index.read.take()))
}
