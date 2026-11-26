//! Custom bundle-like component contents surviving save NBT, network wire codecs,
//! and nested custom pouches.
extern crate sunrise_client_lib as sunrise_client;
use sunrise_client::{item_sync, items::ItemStack, lan::guest::stack_to_network};
use sunrise_core::{nbt::Tag, vanilla_bytes::Writer};
use sunrise_items::{
    bundle::{BundleSpec, CustomBundleContents, destroyed_component},
    item_nbt::{item_from_nbt, item_to_nbt},
    items::Items,
};
use sunrise_protocol::{
    buf::{Codec, Reader},
    item::{DelimitedSlot, Hashed, Slot, Stack, Template, custom},
    registry::{Builtin, Dynamic, Ids, SyncedNames},
};

const FIRST_CUSTOM_ID: i32 = 122;

#[test]
fn custom_bundle_contents_survive_save_wire_and_nested_pouches() {
    custom::install(FIRST_CUSTOM_ID, vec!["runes:pouch_contents".into()]).unwrap();
    let mut component_names = vec![String::new(); FIRST_CUSTOM_ID as usize];
    component_names[sunrise_protocol::item::component::BUNDLE_CONTENTS as usize] =
        "minecraft:bundle_contents".into();
    component_names[sunrise_protocol::item::component::MAX_STACK_SIZE as usize] =
        "minecraft:max_stack_size".into();
    let builtin = Builtin {
        registries: [
            (
                "minecraft:item".into(),
                Ids::from_names(vec![
                    "minecraft:air".into(),
                    "minecraft:stone".into(),
                    "minecraft:paper".into(),
                    "minecraft:ender_pearl".into(),
                ]),
            ),
            (
                "minecraft:data_component_type".into(),
                Ids::from_names(component_names),
            ),
        ]
        .into(),
    };
    let dynamic = Dynamic {
        custom_components: true,
        ..Default::default()
    };
    let names = SyncedNames {
        builtin: &builtin,
        dynamic: &dynamic,
    };
    let catalog_root =
        std::env::temp_dir().join(format!("sunrise-custom-bundle-wire-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&catalog_root);
    std::fs::create_dir_all(catalog_root.join("data/minecraft/loot_table")).unwrap();
    std::fs::write(
        catalog_root.join("items.json"),
        r#"{
            "minecraft:stone": {"minecraft:max_stack_size": 64},
            "minecraft:paper": {"minecraft:max_stack_size": 64},
            "minecraft:ender_pearl": {"minecraft:max_stack_size": 16}
        }"#,
    )
    .unwrap();
    let defaults = sunrise_world::item_defaults::read(&catalog_root).unwrap();
    let items = Items::catalog(
        &catalog_root,
        &sunrise_world::loot::PackOverlay::NONE,
        &defaults,
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(&catalog_root);
    let spec = BundleSpec::custom("runes:pouch_contents", 4);

    let stack_with_max = |id: &str, count: u32, max: i32| {
        let mut s = ItemStack::new(id, count);
        s.data_mut()
            .set_other("minecraft:max_stack_size", Some(Tag::Int(max)));
        s
    };

    // Inner custom pouch holding 8 pearls (weight = 8/16 + 1/16 = 9/16).
    let mut inner = ItemStack::new("minecraft:paper", 1);
    inner
        .data_mut()
        .set_other("runes:pouch_contents", Some(Tag::List(Vec::new())));
    let mut inner_contents = CustomBundleContents::of(&items, &inner, spec).unwrap();
    let mut pearls = stack_with_max("minecraft:ender_pearl", 8, 16);
    assert_eq!(inner_contents.try_insert(&items, &mut pearls, true), 8);
    inner_contents.store(&items, &mut inner);

    // Outer custom pouch holding the inner pouch + 64 stones (total weight = 1 + 9/16 = 25/16).
    let mut outer = ItemStack::new("minecraft:paper", 1);
    outer
        .data_mut()
        .set_other("runes:pouch_contents", Some(Tag::List(Vec::new())));
    let mut outer_contents = CustomBundleContents::of(&items, &outer, spec).unwrap();
    let mut inner_offer = inner.clone();
    assert_eq!(outer_contents.try_insert(&items, &mut inner_offer, true), 1);
    let mut stones = stack_with_max("minecraft:stone", 64, 64);
    assert_eq!(outer_contents.try_insert(&items, &mut stones, true), 64);
    outer_contents.store(&items, &mut outer);

    // Save NBT round-trip.
    let saved = item_from_nbt(&item_to_nbt(&outer, &items)).unwrap();
    assert!(outer.same_item_same_components(&saved));

    // Network Slot / DelimitedSlot / Template / Hashed round-trip.
    let item_id = |name: &str| match name {
        "minecraft:stone" => 1,
        "minecraft:paper" => 2,
        "minecraft:ender_pearl" => 3,
        _ => 0,
    };
    let encode = |stack: &ItemStack| {
        stack_to_network(item_id, |_, name| custom::id(name), &Some(stack.clone())).unwrap()
    };
    let net = encode(&saved);
    assert!(!custom::permitted(&net, false).unwrap());
    assert!(custom::permitted(&net, true).unwrap());
    let hash = Hashed::create(&net, &names).unwrap();

    let mut writer = Writer::new();
    Slot::write(&Some(net.clone()), &mut writer);
    let received = Slot::read(&mut Reader::new(&writer.data)).unwrap().unwrap();
    let client = item_sync::from_network(&received, &builtin, &dynamic);
    assert!(saved.same_item_same_components(&client));
    assert_eq!(hash, Hashed::create(&encode(&client), &names).unwrap());

    let mut writer = Writer::new();
    DelimitedSlot::write(&Some(encode(&client)), &mut writer);
    let returned = DelimitedSlot::read(&mut Reader::new(&writer.data))
        .unwrap()
        .unwrap();
    let round_tripped = item_sync::from_network(&returned, &builtin, &dynamic);
    assert!(saved.same_item_same_components(&round_tripped));

    let mut writer = Writer::new();
    Template::write(&net, &mut writer);
    let template: Stack = Template::read(&mut Reader::new(&writer.data)).unwrap();
    assert!(
        saved.same_item_same_components(&item_sync::from_network(&template, &builtin, &dynamic))
    );

    // Decoded nested contents preserve exact remaining weight and destruction spills.
    let decoded = CustomBundleContents::of(&items, &round_tripped, spec).unwrap();
    assert_eq!(decoded.items().len(), 2);
    // Remaining capacity = 4 - 25/16 = 39/16 = 156 sixty-four-stack stones.
    assert_eq!(
        decoded.room_for(&items, &stack_with_max("minecraft:stone", 1, 64)),
        156
    );
    let mut to_destroy = round_tripped.clone();
    let spilled = destroyed_component(&items, &mut to_destroy, "runes:pouch_contents");
    assert_eq!(spilled.len(), 2);
    assert_eq!(&*spilled[0].item, "minecraft:stone");
    assert_eq!(spilled[0].count, 64);
    assert_eq!(&*spilled[1].item, "minecraft:paper");
    assert!(
        CustomBundleContents::of(&items, &to_destroy, spec)
            .unwrap()
            .items()
            .is_empty()
    );
}
