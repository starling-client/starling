#[test]
fn client_menu_generation_changes_on_reused_window_ids() {
    let mut menu = crate::game::remote::ClientMenu::default();
    let inventory = menu.generation;
    menu.set(Some((7, "minecraft:generic_9x3".to_owned())));
    let first = menu.generation;
    assert_ne!(inventory, first);
    menu.set(None);
    let closed = menu.generation;
    assert_ne!(first, closed);
    assert!(menu.as_ref().is_none());
    menu.set(Some((7, "minecraft:generic_9x3".to_owned())));
    assert_ne!(closed, menu.generation);
    assert_ne!(first, menu.generation);
    assert_eq!(menu.as_ref().map(|entry| entry.0), Some(7));
    let before_replacement = menu.generation;
    menu.set(Some((7, "minecraft:generic_9x3".to_owned())));
    assert_ne!(before_replacement, menu.generation);
}

#[test]
fn client_menu_slot_mapping_covers_inventory_and_containers() {
    for (name, count, first, last) in [
        ("inventory", 46, 200, 504),
        ("minecraft:crafting", 46, 200, 8),
        ("minecraft:generic_9x3", 63, 300, 8),
        ("minecraft:hopper", 41, 300, 8),
        ("minecraft:furnace", 39, 300, 8),
        ("minecraft:brewing_stand", 41, 300, 8),
    ] {
        assert_eq!(super::Game::inventory_screen_slot(name, 0), Some(first));
        assert_eq!(
            super::Game::inventory_screen_slot(name, count - 1),
            Some(last)
        );
        assert_eq!(super::Game::inventory_screen_slot(name, count), None);
    }
    assert_eq!(
        super::Game::inventory_screen_slot("minecraft:unknown", 0),
        None
    );
}
