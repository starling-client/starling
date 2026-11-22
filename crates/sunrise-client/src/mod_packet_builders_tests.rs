use super::*;
use sunrise_core::nbt::{Compound, Tag};
use sunrise_items::items::ItemStack as EngineStack;
use sunrise_protocol::{buf::Reader, item::Component, packets::play::Serverbound};

fn builtin() -> Builtin {
    Builtin::parse(r#"{"minecraft:item":{"entries":{"minecraft:stick":{"protocol_id":4}}},"minecraft:data_component_type":{"entries":{"minecraft:damage":{"protocol_id":3},"minecraft:lore":{"protocol_id":11}}}}"#).unwrap()
}
fn fixture() -> ContainerClickRequest {
    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
    mod_api::__private::resolve_items(["minecraft:stick"]).unwrap();
    let mut stack =
        mod_api::ItemStack::new(mod_api::__private::item_key("minecraft:stick").unwrap(), 3)
            .unwrap();
    stack
        .set_components(Compound::from([
            ("minecraft:damage".into(), Tag::Int(7)),
            ("!minecraft:lore".into(), Tag::Compound(Compound::new())),
        ]))
        .unwrap();
    ContainerClickRequest {
        container_id: 2,
        state_id: 5,
        slot: 36,
        changed_slots: vec![(36, Some(stack.clone())), (37, None)],
        carried: Some(stack),
        ..Default::default()
    }
}
fn engine(value: &mod_api::ItemStack) -> Option<EngineStack> {
    let patch = value
        .components()
        .map(|p| Tag::Compound(p.as_nbt().clone()));
    let (_, extra) = sunrise_items::item_data::decode_components(patch.as_ref());
    Some(EngineStack {
        item: "minecraft:stick".into(),
        count: value.count(),
        damage: value.damage(),
        extra,
    })
}

#[test]
fn plain_component_stacks_match_game_container_codec() {
    mod_platform::testing::with_mods(vec![].into(), || {
        let request = fixture();
        let got =
            encode_container_click(&request, &builtin(), &Dynamic::default(), engine).unwrap();
        let network = Stack {
            added: vec![(3, Component::Int(7))],
            removed: vec![11],
            ..Stack::new(4, 3)
        };
        let hashed = Hashed::create(&network, &sunrise_protocol::item::NoNames).unwrap();
        let expected = Serverbound::ContainerClick(ContainerClick {
            container: 2,
            state: 5,
            slot: 36,
            button: 0,
            input: 0,
            changed: vec![(36, Some(hashed.clone())), (37, None)],
            carried: Some(hashed),
        })
        .encode();
        let mut reader = Reader::new(&expected);
        reader.varint().unwrap();
        assert_eq!(got.id.to_string(), "minecraft:container_click");
        assert_eq!(got.body, reader.rest());
    });
}

#[test]
fn container_count_slots_and_unencodable_components_are_named() {
    mod_platform::testing::with_mods(vec![].into(), || {
        let mut request = fixture();
        request.changed_slots = vec![(36, None); 129];
        assert_eq!(
            encode_container_click(&request, &builtin(), &Dynamic::default(), |_| panic!(
                "limits must precede conversion"
            )),
            Err(PacketBuildError::TooManyChangedSlots {
                packet: "container_click".into(),
                field: "changed_slots".into()
            })
        );
        request.changed_slots = vec![(32768, None)];
        assert_eq!(
            encode_container_click(&request, &builtin(), &Dynamic::default(), engine),
            Err(range("changed_slots[0].slot"))
        );
        request.changed_slots.clear();
        request
            .carried
            .as_mut()
            .unwrap()
            .set_components(Compound::from([("test:unknown".into(), Tag::Int(1))]))
            .unwrap();
        assert_eq!(
            encode_container_click(&request, &builtin(), &Dynamic::default(), engine),
            Err(component_error("carried.components.test:unknown"))
        );
        let mut zero =
            mod_api::ItemStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
        zero.set_damage(0);
        request.carried = Some(zero);
        assert_eq!(
            encode_container_click(&request, &builtin(), &Dynamic::default(), engine),
            Err(component_error("carried.components.minecraft:damage"))
        );
        let unsupported = Stack {
            added: vec![(999, Component::Raw(vec![]))],
            ..Stack::new(4, 3)
        };
        assert!(checked_hash(&unsupported, &sunrise_protocol::item::NoNames, "carried").is_err());
    });
}
