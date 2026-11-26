//! `#[derive(Event)]`: a mod's own event, from `#[event(id = "…", side = …, cancellable)]`.
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields};

pub(crate) fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(name, "an event is a struct"));
    };
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "an event has no generic parameters: one Rust type per event id",
        ));
    }
    if !input
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("non_exhaustive"))
    {
        return Err(syn::Error::new_spanned(
            name,
            "an event is #[non_exhaustive], so its mod can add fields in a minor release",
        ));
    }
    let mut id = None;
    let mut side = None;
    let mut cancellable = false;
    for attr in input
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("event"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("id") && id.is_none() {
                let text: syn::LitStr = meta.value()?.parse()?;
                let value = text.value();
                let valid = value.split_once(':').is_some_and(|(namespace, path)| {
                    !namespace.is_empty()
                        && !path.is_empty()
                        && namespace != "minecraft"
                        && namespace != ".."
                        && namespace
                            .bytes()
                            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.'))
                        && path.bytes().all(|b| {
                            matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/')
                        })
                });
                if !valid {
                    return Err(syn::Error::new_spanned(
                        text,
                        "an event id is `your_mod:name`, not in `minecraft:`",
                    ));
                }
                id = Some(text);
                Ok(())
            } else if meta.path.is_ident("side") && side.is_none() {
                let ident: syn::Ident = meta.value()?.parse()?;
                side = Some(match ident.to_string().as_str() {
                    "server" => quote!(::mod_api::ServerSide),
                    "client" => quote!(::mod_api::ClientSide),
                    _ => return Err(syn::Error::new_spanned(ident, "expected `server` or `client`")),
                });
                Ok(())
            } else if meta.path.is_ident("cancellable") && !cancellable {
                cancellable = true;
                Ok(())
            } else {
                Err(meta.error(
                    "expected one each of `id = \"mod:name\"`, `side = server | client`, `cancellable`",
                ))
            }
        })?;
    }
    let Some(id) = id else {
        return Err(syn::Error::new_spanned(
            name,
            "write #[event(id = \"your_mod:name\")]",
        ));
    };
    let side = side.unwrap_or_else(|| quote!(::mod_api::BothSides));
    let cancel = if cancellable {
        let has_flag = matches!(&data.fields, Fields::Named(fields) if fields.named.iter().any(|field| {
            field.ident.as_ref().is_some_and(|ident| ident == "cancelled")
                && matches!(&field.ty, syn::Type::Path(path) if path.path.is_ident("bool"))
        }));
        if !has_flag {
            return Err(syn::Error::new_spanned(
                name,
                "a cancellable event has a `cancelled: bool` field",
            ));
        }
        quote! {
            type Outcome = ::mod_api::Flow;
            fn is_cancelled(&self) -> bool { self.cancelled }
            fn set_cancelled(&mut self, cancelled: bool) { self.cancelled = cancelled; }
        }
    } else {
        quote!(
            type Outcome = ();
        )
    };
    Ok(quote! {
        impl ::mod_api::Event for #name {
            const ID: &'static str = #id;
            type Side = #side;
            #cancel
        }
    })
}
