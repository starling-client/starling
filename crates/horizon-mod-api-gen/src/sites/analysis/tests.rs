use super::*;

#[test]
fn lexical_bindings_and_macro_tokens_cannot_be_captured_as_generated_names() {
    let sig = syn::parse_quote!(fn run(add: fn(u32) -> u32, child: &Child));
    let body = syn::parse_quote!({
        let (local, mut nested) = pair();
        let closure = |argument| match argument {
            Some(matched) => matched,
            None => 0,
        };
        let __recv = 0;
        let __recv_1 = 0;
        opaque!(__recv_2);
        use other::callee as imported;
        const INNER: u32 = 1;
        fn inner_fn(unrelated: u32) {
            let nested_body = unrelated;
        }
    });
    let mut scope = Scope::scan(&sig, Some(&body));
    assert_eq!(
        scope.parameters,
        BTreeSet::from(["add".into(), "child".into()])
    );
    for name in [
        "local", "nested", "closure", "argument", "matched", "imported", "INNER", "inner_fn",
    ] {
        assert!(scope.locals.contains(name), "{name}");
    }
    assert!(!scope.locals.contains("unrelated"));
    assert!(!scope.locals.contains("nested_body"));
    assert!(scope.opaque);
    assert_eq!(scope.fresh("__recv"), "__recv_3");
    assert_eq!(scope.fresh("__recv"), "__recv_4");
    let glob = syn::parse_quote!({
        use other::*;
    });
    assert!(Scope::scan(&sig, Some(&glob)).opaque);
    let local_crates = syn::parse_quote!({
        extern crate other as imported_crate;
        use other::{self};
    });
    let scope = Scope::scan(&sig, Some(&local_crates));
    assert!(scope.opaque);
    assert!(scope.locals.contains("imported_crate"));
}

#[test]
fn runtime_call_selection_does_not_visit_any_constant_context() {
    let body = syn::parse_quote!({
        let _: [u8; add(10)] = [];
        let _ = [add(1); add(11)];
        let _ = holder::<{ add(12) }>(add(2));
        const C: usize = add(13);
        static S: usize = add(14);
        enum E {
            Value = add(15) as isize,
        }
        let _ = const { add(16) };
        type Array = [u8; add(17)];
        let _ = |x| add(x);
    });
    let calls = calls(&body, "add")
        .into_iter()
        .map(|e| e.to_token_stream().to_string().replace(' ', ""))
        .collect::<Vec<_>>();
    assert_eq!(calls, ["add(1)", "add(2)", "add(x)"]);
}

#[test]
fn resolved_shapes_find_elided_and_nested_lifetimes_before_spelling_results() {
    use crate::crossing::{Rule, tests::internal};
    let entries = [
        internal(
            "horizon_client::game::View",
            "struct",
            "struct View<'a>(&'a u32);",
        ),
        internal(
            "horizon_client::game::Owned",
            "struct",
            "struct Owned(u32);",
        ),
        internal(
            "horizon_client::game::Alias",
            "type",
            "type Alias<'a> = View<'a>;",
        ),
    ];
    let rule = Rule::new(&entries);
    for written in [
        "View",
        "View<'_>",
        "View<'static>",
        "Option<View>",
        "Vec<View>",
        "[View; 3]",
        "(u32, View)",
        "Alias",
        "std::borrow::Cow<str>",
        "&'static u32",
        "impl Iterator<Item = u32>",
    ] {
        let ty = syn::parse_str(written).unwrap();
        assert!(
            rule.shape("horizon_client::game", None, &ty).may_borrow(),
            "{written}"
        );
    }
    for written in [
        "Owned",
        "Option<Owned>",
        "Vec<u32>",
        "String",
        "()",
        "[u32; 3]",
        "(u32, bool)",
    ] {
        let ty = syn::parse_str(written).unwrap();
        assert!(
            !rule.shape("horizon_client::game", None, &ty).may_borrow(),
            "{written}"
        );
    }
}

#[test]
fn lexical_call_identity_refuses_shadowing_in_all_binding_contexts() {
    let sig = syn::parse_quote!(fn run(child: &Child));
    let call = syn::parse_quote!(add(1));
    for body in [
        "{ let add = callback; add(1); }",
        "{ let _ = |add: fn(u32) -> u32| add(1); }",
        "{ match option { Some(add) => add(1), None => 0 } }",
        "{ use other::add; add(1); }",
        "{ opaque!(); add(1); }",
    ] {
        let body = syn::parse_str(body).unwrap();
        assert!(Scope::scan(&sig, Some(&body)).check_target(&call).is_err());
    }
    let parameter = syn::parse_quote!(fn run(add: fn(u32) -> bool));
    assert!(Scope::scan(&parameter, None).check_target(&call).is_err());
    let body = syn::parse_quote!({
        let child = another;
    });
    let child = syn::parse_quote!(child.step(1));
    assert!(Scope::scan(&sig, Some(&body)).check_target(&child).is_err());
    assert!(Scope::scan(&sig, None).check_target(&child).is_ok());
    let body = syn::parse_quote!({
        opaque!();
    });
    let fixed = syn::parse_quote!(crate::game::add(1));
    assert!(Scope::scan(&sig, Some(&body)).check_target(&fixed).is_ok());
    let method = syn::parse_quote!(self.child.step(1));
    assert!(Scope::scan(&sig, Some(&body)).check_target(&method).is_ok());
    assert!(opaque_argument(&syn::parse_quote!(unknown!(1))));
}

#[test]
fn argument_records_refuse_hidden_receiver_aliases_and_macro_expansions() {
    let receiver = syn::parse_quote!(self.child);
    for written in [
        "self.value",
        "alias.value",
        "*alias",
        "read(alias)",
        "opaque!(value)",
    ] {
        let arg = syn::parse_str(written).unwrap();
        assert!(
            arguments(&[&arg], Some(&receiver), true).is_err(),
            "{written}"
        );
    }
    for written in ["value", "(value)", "7", "-7", "!true", "Type::CONSTANT"] {
        let arg = syn::parse_str(written).unwrap();
        assert!(
            arguments(&[&arg], Some(&receiver), true).is_ok(),
            "{written}"
        );
    }
}

#[test]
fn review_raw_binding_must_shadow_plain_callee() {
    let sig = syn::parse_quote!(fn run(r#add: fn(u32) -> bool));
    let call = syn::parse_quote!(add(1));
    assert!(Scope::scan(&sig, None).check_target(&call).is_err());
}

#[test]
fn review_plain_binding_must_shadow_raw_callee() {
    let sig = syn::parse_quote!(fn run(add: fn(u32) -> bool));
    let call = syn::parse_quote!(r#add(1));
    assert!(Scope::scan(&sig, None).check_target(&call).is_err());
}

#[test]
fn review_raw_identifier_must_reserve_plain_generated_name() {
    let sig = syn::parse_quote!(fn run(r#__recv: u32));
    assert_eq!(Scope::scan(&sig, None).fresh("__recv"), "__recv_1");
}

#[test]
fn raw_receiver_and_argument_names_share_the_same_identifier_set() {
    for (receiver, argument) in [
        ("r#child", "child"),
        ("child", "r#child"),
        ("self.r#child", "self.child"),
    ] {
        let receiver = syn::parse_str(receiver).unwrap();
        let argument = syn::parse_str(argument).unwrap();
        assert!(arguments(&[&argument], Some(&receiver), true).is_err());
    }
}
