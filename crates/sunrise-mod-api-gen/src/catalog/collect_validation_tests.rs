use super::{Hook, RegistrationAppend};
use serde_json::json;

#[test]
fn attributed_append_only_applies_to_global_non_answered_collectors() {
    let parse = |api| {
        Hook::parse(json!({
            "name": "attributed", "doc": "Doc.", "side": "both", "target": "crate::native_attributed",
            "mod_api": api,
        }))
    };
    assert_eq!(
        parse(json!({"contract":"collect", "out":"items", "registration_append":"attributed"}))
            .unwrap()
            .registration_append,
        Some(RegistrationAppend::Attributed)
    );
    for api in [
        json!({"contract":"listen", "registration_append":"attributed"}),
        json!({"contract":"collect", "out":"items", "scope":"item", "key":"stack", "registration_append":"attributed"}),
        json!({"contract":"collect", "out":"items", "answered_by":"mod", "registration_append":"attributed"}),
        json!({"contract":"collect", "out":"items", "collect_validator":"crate::validate"}),
    ] {
        assert!(parse(api).is_err());
    }
}

#[test]
fn an_output_validator_is_a_path_on_a_global_cancel_or_plain_listen_hook() {
    let parse = |api, context: Option<&str>| {
        let mut entry = json!({
            "name": "checked", "doc": "Doc.", "side": "server", "target": "crate::native_checked",
            "mod_api": api,
        });
        if let Some(context) = context {
            entry["context"] = json!(context);
        }
        Hook::parse(entry)
    };
    for api in [
        json!({"contract":"cancel", "out":"canceled", "output_validator":"crate::check"}),
        json!({"contract":"listen", "output_validator":"crate::check"}),
        json!({"contract":"collect", "out":"value", "output_validator":"crate::check"}),
    ] {
        assert_eq!(
            parse(api, None).unwrap().output_validator.as_deref(),
            Some("crate::check")
        );
    }
    for api in [
        json!({"contract":"cancel", "out":"canceled", "scope":"item", "key":"stack", "output_validator":"crate::check"}),
        json!({"contract":"cancel", "out":"canceled", "output_validator":"not a path"}),
        json!({"contract":"listen", "staged_surface":"sink", "output_validator":"crate::check"}),
        json!({"contract":"listen", "owner":"mod_id", "output_validator":"crate::check"}),
        json!({"contract":"collect", "out":"items", "answered_by":"mod", "output_validator":"crate::check"}),
        json!({"contract":"collect", "out":"items", "registration_append":"attributed", "output_validator":"crate::check"}),
    ] {
        assert!(parse(api, None).is_err());
    }
    assert!(
        parse(
            json!({"contract":"listen", "output_validator":"crate::check"}),
            Some("loop")
        )
        .is_err()
    );
}
