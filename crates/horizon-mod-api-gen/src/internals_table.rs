//! How the site resolver spells game types and finds a function's receiver and signature
//! from the `internals.json` catalog.
use crate::crossing::{Collection, Rule, Scalar, Shape, Wrapper, bare};
use crate::internals::Internal;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// The game's crate.
const GAME: &str = "horizon_client";

/// A type's Rust spelling, from the game crate's root.
pub(crate) fn spell(shape: &Shape) -> Option<TokenStream> {
    Some(match shape {
        Shape::Scalar(scalar) => match scalar {
            Scalar::Bool => quote!(bool),
            Scalar::Char => quote!(char),
            Scalar::Signed(name)
            | Scalar::Unsigned(name)
            | Scalar::Float(name)
            | Scalar::Wide(name) => {
                let name = format_ident!("{name}");
                quote!(#name)
            }
            Scalar::Str => quote!(str),
            Scalar::String => quote!(::std::string::String),
        },
        Shape::Unit => quote!(()),
        Shape::Ref { mutable, to } => {
            let to = spell(to)?;
            if *mutable {
                quote!(&'static mut #to)
            } else {
                quote!(&'static #to)
            }
        }
        Shape::Tuple(parts) => {
            let parts = parts.iter().map(spell).collect::<Option<Vec<_>>>()?;
            quote!((#(#parts,)*))
        }
        Shape::Array(elem, n) => {
            let elem = spell(elem)?;
            quote!([#elem; #n])
        }
        Shape::Slice(elem) => {
            let elem = spell(elem)?;
            quote!([#elem])
        }
        Shape::Option(inner) => {
            let inner = spell(inner)?;
            quote!(::std::option::Option<#inner>)
        }
        Shape::Result(ok, err) => {
            let (ok, err) = (spell(ok)?, spell(err)?);
            quote!(::std::result::Result<#ok, #err>)
        }
        Shape::DynError => quote!(dyn ::std::error::Error),
        Shape::Wrapped(wrapper, inner) => {
            let inner = spell(inner)?;
            match wrapper {
                Wrapper::Box => quote!(::std::boxed::Box<#inner>),
                Wrapper::Arc => quote!(::std::sync::Arc<#inner>),
                Wrapper::Rc => quote!(::std::rc::Rc<#inner>),
                Wrapper::Cow => quote!(::std::borrow::Cow<'static, #inner>),
            }
        }
        Shape::Collection(collection, args) => {
            let args = args.iter().map(spell).collect::<Option<Vec<_>>>()?;
            match collection {
                Collection::Vec => quote!(::std::vec::Vec<#(#args),*>),
                Collection::VecDeque => quote!(::std::collections::VecDeque<#(#args),*>),
                Collection::HashSet => quote!(::std::collections::HashSet<#(#args),*>),
                Collection::BTreeSet => quote!(::std::collections::BTreeSet<#(#args),*>),
                Collection::HashMap => quote!(::std::collections::HashMap<#(#args),*>),
                Collection::BTreeMap => quote!(::std::collections::BTreeMap<#(#args),*>),
            }
        }
        Shape::Named(path, lifetimes) => {
            let path = path_tokens(path);
            if *lifetimes == 0 {
                path
            } else {
                let lifetimes = (0..*lifetimes).map(|_| quote!('static));
                quote!(#path<#(#lifetimes),*>)
            }
        }
        Shape::Opaque(_) => return None,
    })
}

/// A listed path, from the game crate's root.
fn path_tokens(path: &str) -> TokenStream {
    let mut segments = path.split("::");
    let first = segments.next().unwrap_or_default();
    let head = if first == GAME {
        quote!(crate)
    } else {
        let first = format_ident!("{first}");
        quote!(::#first)
    };
    let rest = segments.map(|s| format_ident!("{s}"));
    quote!(#head #(::#rest)*)
}

/// Resolve the receiver exactly where its method signature is written.
pub(crate) fn function_scope(rule: &Rule, internal: &Internal) -> Option<(String, Option<String>)> {
    let path = bare(&internal.path);
    let (owner, _) = path.rsplit_once("::")?;
    let (module, this) = match internal.kind {
        "method" => {
            let (module, ty) = owner.rsplit_once("::")?;
            let written: syn::Path = syn::parse_str(ty).ok()?;
            let ty = match rule.shape(
                module,
                None,
                &syn::Type::Path(syn::TypePath {
                    qself: None,
                    path: written,
                }),
            ) {
                Shape::Named(path, _) => path,
                _ => return None,
            };
            (module.to_owned(), Some(ty))
        }
        "fn" => (owner.to_owned(), None),
        _ => return None,
    };
    Some((module, this))
}

/// The manifest records impl bounds as well as the method signature. A bounded or type-generic
/// impl is not callable on every receiver, even when the method itself has no generics.
pub(crate) fn signature(written: &str) -> Option<syn::Signature> {
    use syn::parse::Parser;
    let parser = |input: syn::parse::ParseStream<'_>| {
        if input.peek(syn::Token![impl]) {
            input.parse::<syn::Token![impl]>()?;
            let generics: syn::Generics = input.parse()?;
            let _: syn::Type = input.parse()?;
            let bounds: Option<syn::WhereClause> = input.parse()?;
            if generics.params.len() != generics.lifetimes().count()
                || generics.lifetimes().any(|l| !l.bounds.is_empty())
                || bounds.is_some()
            {
                return Err(input.error("a generic or bounded impl"));
            }
            let content;
            syn::braced!(content in input);
            content.parse::<syn::Signature>()
        } else {
            input.parse::<syn::Signature>()
        }
    };
    parser.parse_str(written).ok()
}
