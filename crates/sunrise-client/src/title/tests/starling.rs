//! Starling's part of tests.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

#[test]
pub(super) fn the_account_screen_offers_signing_in_and_goes_back() {
    let mut f = Fixture::new();
    // Only where the game signs in itself (Android): the title keeps vanilla's buttons here.
    assert!(f.title.buttons(480.0, 262.0).iter().all(|b| b.id != 80));
    f.title.mode = Mode::Account(account::Screen::default());
    let labels: Vec<String> = f
        .title
        .buttons(480.0, 262.0)
        .into_iter()
        .map(|b| b.label)
        .collect();
    assert!(labels.contains(&lang::get("starling.account.add")));
    assert_eq!(f.click(&lang::get("starling.account.add")), Action::None);
    let labels: Vec<String> = f
        .title
        .buttons(480.0, 262.0)
        .into_iter()
        .map(|b| b.label)
        .collect();
    assert!(labels.contains(&"Microsoft".to_owned()));
    assert!(labels.contains(&"Ely.by".to_owned()));
    assert!(labels.contains(&lang::get("starling.account.offline")));
    f.title.escape();
    assert_eq!(
        f.click_with_sound(&lang::get("starling.account.done")),
        (Action::None, true)
    );
    assert!(matches!(f.title.mode, Mode::Main));
}

#[test]
pub(super) fn world_options_survive_the_restart_as_json() {
    let mut options = WorldOptions {
        hardcore: true,
        world_type: crate::save::WorldPreset::Flat,
        flat_settings: Some(FlatSettings::default()),
        data_packs: vec!["vanilla".into(), "file/zipped.zip".into()],
        mods: Some(["create".to_owned()].into()),
        ..WorldOptions::default()
    };
    assert!(options.rules.set_string("keep_inventory", "true"));
    assert!(options.rules.set_string("random_tick_speed", "7"));
    let json = serde_json::to_value(&options).unwrap();
    let back: WorldOptions = serde_json::from_value(json).unwrap();
    assert_eq!(back, options);
    // An older file without the newer fields reads as their defaults.
    let old: WorldOptions = serde_json::from_str(r#"{"hardcore":true}"#).unwrap();
    assert_eq!(old.mods, None);
}

#[test]
pub(super) fn account_secret_fields_accept_credentials_without_chat_filtering_or_debug_disclosure()
{
    use winit::keyboard::KeyCode;
    let mut f = Fixture::new();
    f.title.mode = Mode::Account(account::Screen::default());
    f.click(&lang::get("starling.account.add"));
    f.click("Ely.by");
    f.title.type_text("player@example.test");
    f.title.tab_dir(false, (480.0, 262.0));
    assert!(f.title.password_focused());
    f.title.type_text("Secret§123");
    f.title.field_key(KeyCode::ArrowLeft, false, false);
    f.title.type_text("!");
    assert_eq!(f.title.focused_field().unwrap().1, "Secret§12!3");
    f.title.set_account_field("Secret§ABC", 7, 10);
    assert_eq!(f.title.account_field_selection(), Some((7, 10)));
    f.title.type_text("XYZ");
    assert_eq!(f.title.focused_field().unwrap().1, "Secret§XYZ");
    if let Mode::Account(screen) = &f.title.mode {
        assert!(!format!("{screen:?}").contains("Secret"));
    }
    assert!(!format!("{:?}", f.title.caret.borrow()).contains("Secret"));
    f.title.escape();
    if let Mode::Account(screen) = &f.title.mode {
        assert!(!format!("{screen:?}").contains("Secret"));
    }
    assert!(!format!("{:?}", f.title.caret.borrow()).contains("Secret"));
}
