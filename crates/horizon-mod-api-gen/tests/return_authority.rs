//! Provisional until #2057: the serialized flag is RETURN authority, not HEAD authority.
use quote::quote;

#[test]
fn catalog_serializes_return_slot_authority_for_each_native_return_class() {
    for (signature, expected) in [
        (syn::parse_quote!(fn f()), false),
        (syn::parse_quote!(fn f() -> ()), true),
        (syn::parse_quote!(fn f() -> u32), true),
        (syn::parse_quote!(fn f() -> &u32), true),
        (syn::parse_quote!(fn f() -> &mut u32), true),
        (syn::parse_quote!(fn f() -> Result<(), E>), true),
    ] {
        let shape = point_shape::shape(quote!(raw), &signature).unwrap();
        let contract = point_shape::raw_shape(&shape, &signature, None);
        let json = serde_json::to_value(contract).unwrap();
        assert_eq!(json["return_record_mutable"], expected);
    }
}
