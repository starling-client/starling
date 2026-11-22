//! Native mods declare their internals at the manifest root.
use sunrise_mod_package::{Manifest, manifest::Internal};

const HEADER: &str =
    "id='owner'\nname='Owner'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n";

fn native() -> String {
    format!(
        "[native]\ngame='26.3'\nsunrise='{}'\nmixins='{}'\n",
        "a".repeat(40),
        "b".repeat(40)
    )
}

#[test]
fn root_records_read_in_both_toml_forms() {
    let expected = vec![
        Internal {
            path: "Round::listening".into(),
            kind: Some("field".into()),
            hash: Some("field-hash".into()),
        },
        Internal {
            path: "Round::listening".into(),
            kind: Some("method".into()),
            hash: Some("method-hash".into()),
        },
        Internal {
            path: "Game".into(),
            kind: None,
            hash: Some("game-hash".into()),
        },
    ];
    for declarations in [
        "internals=[{path='Round::listening',kind='field',hash='field-hash'},{path='Round::listening',kind='method',hash='method-hash'},{path='Game',hash='game-hash'}]\n",
        "[[internals]]\npath='Round::listening'\nkind='field'\nhash='field-hash'\n[[internals]]\npath='Round::listening'\nkind='method'\nhash='method-hash'\n[[internals]]\npath='Game'\nhash='game-hash'\n",
    ] {
        let manifest = Manifest::parse(&format!("{HEADER}{declarations}{}", native())).unwrap();
        assert_eq!(manifest.internals, expected);
    }
}

#[test]
fn native_table_declarations_are_unknown_fields_not_an_alternate_reader() {
    for declarations in [
        "internals=[{path='Game',hash='h'}]\n",
        "[[native.internals]]\npath='Game'\nhash='h'\n",
    ] {
        let old = format!("{HEADER}{}{declarations}", native());
        let error = Manifest::parse(&old).unwrap_err();
        assert!(error.contains("unknown field `internals`"), "{error}");
    }
}

#[test]
fn invalid_root_records_are_refused() {
    for declarations in [
        "[{path='',hash='h'}]",
        "[{path='Game',hash=''}]",
        "[{path='Game',kind='',hash='h'}]",
        "[{path='Game',hash='a'},{path='Game',hash='b'}]",
        "[{path='Game::field',kind='field',hash='a'},{path='Game::field',kind='field',hash='b'}]",
    ] {
        let error = Manifest::parse(&format!("{HEADER}internals={declarations}\n{}", native()))
            .unwrap_err();
        assert!(
            error
                .contains("empty internal path, kind or hash, or duplicate internal path and kind"),
            "{error}"
        );
    }
    let error =
        Manifest::parse(&format!("{HEADER}internals=[{{path='Game',hash='h'}}]\n")).unwrap_err();
    assert!(error.contains("internals need a [native] mod"), "{error}");
}

#[test]
fn native_modules_stay_native() {
    let native = native();
    let manifest = Manifest::parse(&format!("{HEADER}{native}[[native.module]]\nid='support'\nfile='src/game.rs'\nname='support'\nsource='src/support.rs'\n")).unwrap();
    assert_eq!(manifest.native.unwrap().module[0].id, "support");
}
