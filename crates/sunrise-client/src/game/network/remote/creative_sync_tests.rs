use super::*;

#[test]
fn the_creative_sync_covers_inventory_menu_slots_5_to_45_once_each() {
    // CreativeInventoryListener sends every InventoryMenu slot but the crafting ones: the
    // armour (5..=8), the inventory (9..=44) and the off hand (45).
    let stack = |item: &str| Some(sunrise_client::items::ItemStack::new(item, 1));
    let inventory: Vec<_> = (0..36)
        .map(|i| stack(&format!("minecraft:item_{i}")))
        .collect();
    let offhand = stack("minecraft:shield");
    let armor = [
        stack("minecraft:diamond_helmet"),
        stack("minecraft:diamond_chestplate"),
        stack("minecraft:diamond_leggings"),
        stack("minecraft:diamond_boots"),
    ];
    let stacks = crate::game::remote::creative_sync_stacks(&inventory, &offhand, &armor);
    assert_eq!(stacks.len(), crate::game::remote::CREATIVE_SYNCED);
    let mut slots: Vec<i16> = (0..stacks.len())
        .map(crate::game::remote::creative_sync_slot)
        .collect();
    // Each entry goes to the menu slot that holds that stack.
    for (i, &slot) in slots.iter().enumerate() {
        let game_slot = Game::inventory_screen_slot("inventory", i32::from(slot)).unwrap();
        let expected = match game_slot {
            0..36 => &inventory[game_slot],
            504 => &offhand,
            500..504 => &armor[game_slot - 500],
            _ => panic!("entry {i} went to game slot {game_slot}"),
        };
        assert_eq!(&stacks[i], expected, "entry {i}, menu slot {slot}");
    }
    slots.sort();
    assert_eq!(slots, (5..=45).collect::<Vec<i16>>());
}

#[test]
fn a_server_slot_update_leaves_an_unsent_creative_click_to_be_sent() {
    let stack = |item: &str| Some(sunrise_client::items::ItemStack::new(item, 1));
    let none = || vec![None; crate::game::remote::CREATIVE_SYNCED];
    // Synced: nothing. A click puts a helmet on (entry 37, not sent yet), then the server
    // sets hotbar slot 0 (a pickup): only that entry counts as synced.
    let mut synced = none();
    let mut before = none();
    before[37] = stack("minecraft:diamond_helmet");
    let mut now = before.clone();
    now[0] = stack("minecraft:stick");
    crate::game::remote::mark_synced(&mut synced, &before, now.clone());
    assert_eq!(synced[0], now[0]);
    assert_eq!(
        synced[37], None,
        "the helmet still differs, so the next tick sends it"
    );
}
