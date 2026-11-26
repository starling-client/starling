//! The owned-value Wire derive. Call-table borrowing is deliberately not handled here.
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, Type, parse_quote, visit::Visit};

pub(crate) fn expand(mut input: DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let mut via: Option<Type> = None;
    let mut contract: Option<syn::Path> = None;
    for attr in &input.attrs {
        if attr.path().is_ident("wire") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("crate") {
                    if contract.is_some() {
                        return Err(meta.error("duplicate Wire contract path"));
                    }
                    contract = Some(meta.value()?.parse()?);
                    return Ok(());
                }
                if !meta.path.is_ident("via") || via.is_some() {
                    return Err(meta.error("expected one `via = Type`"));
                }
                via = Some(meta.value()?.parse()?);
                Ok(())
            })?;
        }
    }
    let w = contract.unwrap_or_else(|| parse_quote!(::mod_api::wire));
    if via.is_none() {
        for param in input.generics.type_params_mut() {
            param.bounds.push(parse_quote!(#w::Wire));
        }
    }
    if let Some(via) = &via {
        let where_clause = input.generics.make_where_clause();
        where_clause.predicates.push(parse_quote!(#via: #w::Wire));
        where_clause
            .predicates
            .push(parse_quote!(#via: for<'__wire> ::core::convert::From<&'__wire Self>));
        where_clause
            .predicates
            .push(parse_quote!(Self: ::core::convert::TryFrom<#via>));
    }
    let (imp, ty, clause) = input.generics.split_for_impl();
    if let Some(via) = via {
        return Ok(quote! {
            #[automatically_derived]
            impl #imp #w::Wire for #name #ty #clause {
                const MIN: ::core::primitive::usize = <#via as #w::Wire>::MIN;
                fn describe(schema: &mut #w::WireSchema) -> ::core::primitive::u64
                where
                    Self: 'static,
                {
                    schema.ordinary_only();
                    schema.named::<Self>(|schema| <#via as #w::Wire>::describe(schema))
                }
                fn put(&self, out: &mut #w::Writer) {
                    if out.ordinary_only() {
                        out.put(&<#via as ::core::convert::From<&Self>>::from(self));
                    }
                }
                fn take(input: &mut #w::Reader<'_>) -> ::core::result::Result<Self, #w::WireError> {
                    input.ordinary_only()?;
                    <Self as ::core::convert::TryFrom<#via>>::try_from(input.take::<#via>()?)
                        .map_err(|_| #w::WireError::Invalid(::core::stringify!(#name)))
                }
            }
        });
    }
    let mut min = quote!(0usize);
    let mut puts = Vec::new();
    let mut takes = Vec::new();
    let nested;
    let describe;
    match &input.data {
        Data::Struct(data) => {
            nested = !matches!(data.fields, Fields::Unit);
            let shape = describe_fields(&data.fields, &w);
            describe = quote! {
                let hash = #w::WireSchema::leaf("struct");
                #shape
                hash
            };
            let (pattern, build, fields) = fields(&data.fields);
            for (field, bind) in data.fields.iter().zip(&fields) {
                check_owned(&field.ty)?;
                let ft = &field.ty;
                min = quote!(#min + <#ft as #w::Wire>::MIN);
                puts.push(quote!(out.put(#bind);));
            }
            takes.push(quote!(#build));
            puts = vec![quote!(let Self #pattern = self; #(#puts)*)];
        }
        Data::Enum(data) => {
            min = quote!(4usize);
            nested = data
                .variants
                .iter()
                .any(|v| !matches!(v.fields, Fields::Unit));
            let count = data.variants.len() as u64;
            let shapes = data.variants.iter().map(|v| describe_fields(&v.fields, &w));
            describe = quote! {
                let hash = #w::WireSchema::mix(#w::WireSchema::leaf("enum:u32-le"), #count);
                #(#shapes)*
                hash
            };
            for (index, variant) in data.variants.iter().enumerate() {
                let index = index as u32;
                let case = &variant.ident;
                let (pat, _, binds) = fields(&variant.fields);
                let pat = quote!(Self::#case #pat);
                // `fields` returns a constructor beginning with Self; enum cases supply that prefix.
                let build = construct(&variant.fields, quote!(Self::#case));
                let mut ps = Vec::new();
                for (field, bind) in variant.fields.iter().zip(binds) {
                    check_owned(&field.ty)?;
                    ps.push(quote!(out.put(#bind);));
                }
                puts.push(quote!(#pat => { out.put(&#index); #(#ps)* }));
                takes.push(quote!(#index => #build,));
            }
            puts = vec![quote!(match self { #(#puts,)* _ => ::core::unreachable!() })];
            takes = vec![quote! {
                match input.take::<::core::primitive::u32>()? {
                    #(#takes)*
                    tag => return ::core::result::Result::Err(#w::WireError::Tag(tag)),
                }
            }];
        }
        Data::Union(_) => return Err(syn::Error::new_spanned(name, "Wire cannot encode a union")),
    }
    let take = if nested {
        quote!(input.record(|input| ::core::result::Result::Ok(#(#takes)*)))
    } else {
        quote!(::core::result::Result::Ok(#(#takes)*))
    };
    let put = if nested {
        quote!(out.record(|out| { #(#puts)* }))
    } else {
        quote!(#(#puts)*)
    };
    Ok(quote! {
        #[allow(unreachable_patterns, unused_variables)]
        #[automatically_derived]
        impl #imp #w::Wire for #name #ty #clause {
            const MIN: ::core::primitive::usize = #min;
            fn describe(schema: &mut #w::WireSchema) -> ::core::primitive::u64
            where
                Self: 'static,
            {
                schema.named::<Self>(|schema| { #describe })
            }
            fn put(&self, out: &mut #w::Writer) { #put }
            fn take(input: &mut #w::Reader<'_>) -> ::core::result::Result<Self, #w::WireError> { #take }
        }
    })
}

fn describe_fields(fields: &Fields, wire: &syn::Path) -> TokenStream {
    let count = fields.len() as u64;
    let types = fields.iter().map(|field| &field.ty);
    quote! {
        let hash = #wire::WireSchema::mix(hash, #count);
        #(let hash = #wire::WireSchema::mix(hash, <#types as #wire::Wire>::describe(schema));)*
    }
}

fn fields(fields: &Fields) -> (TokenStream, TokenStream, Vec<syn::Ident>) {
    let binds: Vec<_> = fields
        .iter()
        .enumerate()
        .map(|(i, _)| format_ident!("__wire_f_{i}"))
        .collect();
    let pattern = match fields {
        Fields::Named(fields) => {
            let names = fields.named.iter().map(|f| &f.ident);
            quote!({ #(#names: #binds),* })
        }
        Fields::Unnamed(_) => quote!((#(#binds),*)),
        Fields::Unit => quote!(),
    };
    (pattern, construct(fields, quote!(Self)), binds)
}

fn construct(fields: &Fields, prefix: TokenStream) -> TokenStream {
    match fields {
        Fields::Named(fields) => {
            let names = fields.named.iter().map(|f| &f.ident);
            quote!(#prefix { #(#names: input.take()?),* })
        }
        Fields::Unnamed(fields) => {
            let takes = fields.unnamed.iter().map(|_| quote!(input.take()?));
            quote!(#prefix (#(#takes),*))
        }
        Fields::Unit => prefix,
    }
}

fn check_owned(ty: &Type) -> syn::Result<()> {
    struct Borrowed(Option<syn::Error>);
    impl<'ast> Visit<'ast> for Borrowed {
        fn visit_type_path(&mut self, p: &'ast syn::TypePath) {
            if p.path.segments.last().is_some_and(|s| s.ident == "Cow") {
                if let syn::PathArguments::AngleBracketed(args) =
                    &p.path.segments.last().unwrap().arguments
                {
                    for arg in &args.args {
                        if let syn::GenericArgument::Type(t) = arg {
                            self.visit_type(t);
                        }
                    }
                }
                return;
            }
            syn::visit::visit_type_path(self, p);
        }
        fn visit_lifetime(&mut self, lifetime: &'ast syn::Lifetime) {
            if lifetime.ident != "static" {
                self.0 = Some(syn::Error::new_spanned(
                    lifetime,
                    "Wire is for owned values; a borrowed record crosses as a call argument",
                ));
            }
        }
        fn visit_type_reference(&mut self, r: &'ast syn::TypeReference) {
            self.0 = Some(syn::Error::new_spanned(
                r,
                "Wire is for owned values; a borrowed record crosses as a call argument",
            ));
        }
    }
    let mut check = Borrowed(None);
    check.visit_type(ty);
    check.0.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_expand_to_rust_items() {
        for source in [
            "struct Named { a: u32, b: String }",
            "struct Tuple(u32, String);",
            "struct Unit;",
            "enum Plain { A = 7, B = 9 }",
            "enum Empty {}",
            "struct Shadow { out: u32, input: u8 }",
            "enum Payload { A(u32), B { text: String }, C }",
            "struct Generic<T, const N: usize> { value: [T; N] }",
            "struct Recursive { children: Vec<Recursive> }",
            "#[wire(via = String)] struct Checked(String);",
            "struct OwnedCow { text: std::borrow::Cow<'static, str> }",
        ] {
            let expanded = expand(syn::parse_str(source).unwrap()).unwrap();
            syn::parse2::<syn::ItemImpl>(expanded).unwrap_or_else(|e| panic!("{source}: {e}"));
        }
    }

    #[test]
    fn contract_path_reaches_bounds_fields_and_via() {
        for source in [
            "#[wire(crate = ::mod_vocab::wire)] struct Generic<T> { value: T }",
            "#[wire(crate = ::mod_vocab::wire, via = String)] struct Checked(String);",
            "#[wire(crate = ::mod_vocab::wire)] enum Payload { A(String), B }",
        ] {
            let output = expand(syn::parse_str(source).unwrap()).unwrap();
            syn::parse2::<syn::ItemImpl>(output.clone()).unwrap();
            let output = output.to_string();
            assert!(output.contains("mod_vocab"));
            assert!(!output.contains("mod_api"));
        }
        assert!(
            expand(syn::parse_quote!(
                #[wire(crate = a, crate = b)]
                struct Duplicate;
            ))
            .is_err()
        );
    }

    #[test]
    fn borrowed_fields_are_rejected_even_inside_containers() {
        for source in [
            "struct Borrow<'a> { value: &'a str }",
            "struct Borrow<'a> { value: Vec<&'a u8> }",
            "struct Borrow { value: &'static [u8] }",
            "struct Borrow { value: &'static str }",
            "struct Borrow { value: Option<&'static str> }",
            "struct Borrow { value: &'static mut str }",
            "struct Borrow<'a> { value: Option<Other<'a>> }",
        ] {
            let err = expand(syn::parse_str(source).unwrap()).unwrap_err();
            assert!(err.to_string().contains("Wire is for owned values"));
        }
    }

    #[test]
    fn union_and_unknown_options_are_rejected() {
        for source in [
            "union Union { a: u32, b: u8 }",
            "#[wire(unknown)] struct Unknown;",
            "#[wire(via = String, via = u8)] struct Duplicate;",
            "#[wire(internal_field_layouts)] struct Removed;",
        ] {
            assert!(expand(syn::parse_str(source).unwrap()).is_err());
        }
    }
}
