//! Byte and independently typed field-schema regressions for derived and checked codecs.
use super::*;
use crate::*;
use sunrise_core::nbt::{Compound, JavaString};

fn fields<T: Wire, F: Wire + std::fmt::Debug + PartialEq>(value: T, expected: F) {
    assert_eq!(decode::<T, F>(value), expected);
}
fn id(s: &str) -> Identifier {
    Identifier::parse(s).unwrap()
}

#[test]
fn checked_records_have_declared_field_bytes() {
    fields(
        id("example:gear"),
        ("example".to_owned(), "gear".to_owned()),
    );
    fields(
        LevelKey::try_from("example:moon".to_owned()).unwrap(),
        ("example:moon".to_owned(),),
    );
    fields(JavaString(vec![65, 0xd800, 90]), (vec![65u16, 0xd800, 90],));
    fields(
        EnchantmentInfo::new(
            "example:keen".to_owned(),
            5,
            10,
            1,
            false,
            crate::Text::translate("enchantment.example.keen", Vec::new()),
        ),
        (
            "example:keen".to_owned(),
            5u32,
            10u32,
            1u32,
            false,
            crate::Text::translate("enchantment.example.keen", Vec::new()),
        ),
    );
    let mut nbt = Compound::new();
    nbt.insert("answer".into(), Tag::Int(42));
    fields(Components::from(nbt.clone()), (nbt.clone(),));
    let _keys = Keys::new();
    let key = ItemKey::from_slot(0).unwrap();
    let mut stack = ItemStack::new(key, 7).unwrap();
    stack.set_damage(11);
    fields(stack.clone(), (key, 7u32, Some(11u32), None::<Components>));
    fields(
        MenuSlotChange::new(MenuContainer::Main, 1, Some(stack.clone()), None),
        (
            MenuContainer::Main,
            1usize,
            Some(stack.clone()),
            None::<ItemStack>,
        ),
    );
    fields(
        MenuSlotWrite::new(MenuContainer::Main, 2, Some(stack.clone())),
        (MenuContainer::Main, 2usize, Some(stack)),
    );
    let block = BlockKey::from_slot(0).unwrap();
    let state: BlockState = crate::__private::block_state_from_ordinal(block, 9);
    fields(state, (block, 9u16));
    fields(BlockStateRange::new(state, 3).unwrap(), (state, 3u32));
    fields(
        FeatureFlags::new(vec!["example:a".to_owned(), "example:b".to_owned()].into()),
        (vec!["example:a".to_owned(), "example:b".to_owned()],),
    );
    fields(
        BlockPos {
            x: -3,
            y: 71,
            z: 19,
        },
        (-3i32, 71i32, 19i32),
    );
    fields(ChunkPos { x: -8, z: 11 }, (-8i32, 11i32));
    fields(
        BoundingBox {
            min: [-3, 4, 5],
            max: [6, 17, 28],
        },
        ([-3i32, 4, 5], [6i32, 17, 28]),
    );
    fields(
        serde_json::json!({"a": [1, true]}),
        ("{\"a\":[1,true]}".to_owned(),),
    );
    let map = serde_json::Map::from_iter([("b".to_owned(), serde_json::json!(2))]);
    fields(map, ("{\"b\":2}".to_owned(),));
    let text = Text::translate("k", vec![Text::literal("a")]).append(Text::literal("c"));
    fields(
        text.clone(),
        (
            text.content.clone(),
            text.style.clone(),
            text.children.clone(),
            text.metadata.clone(),
        ),
    );
}

#[test]
fn feature_flag_wire_preserves_the_effective_set() {
    let flags = FeatureFlags::new(["minecraft:vanilla".to_owned()].into());
    assert!(flags.contains("minecraft:vanilla"));
    assert!(flags.contains("vanilla"));
    assert!(flags.contains_flag(FeatureFlag::Vanilla));
    assert!(!flags.contains_flag(FeatureFlag::RedstoneExperiments));
    assert!(flags.contains_all(&[FeatureFlag::Vanilla]));
    assert!(!flags.contains_all(&[FeatureFlag::Vanilla, FeatureFlag::MinecartImprovements]));
    assert_eq!(
        FeatureFlag::from_id("minecraft:trade_rebalance"),
        Some(FeatureFlag::TradeRebalance)
    );
    assert_eq!(
        FeatureFlag::from_id("redstone_experiments"),
        Some(FeatureFlag::RedstoneExperiments)
    );
    assert_eq!(FeatureFlag::from_id("unknown:flag"), None);
    for flag in FeatureFlag::ALL {
        assert_eq!(FeatureFlag::from_id(flag.id()), Some(flag));
        let mut flag_out = Writer::new();
        flag_out.put(&flag);
        assert_eq!(
            Reader::decode::<FeatureFlag>(flag_out.bytes()).unwrap(),
            flag
        );
    }
    let spawners = [
        Spawner::Block {
            pos: BlockPos::new(1, 2, 3),
        },
        Spawner::Trial {
            pos: BlockPos::new(-4, 5, 6),
        },
        Spawner::Minecart {
            entity: EntityRef::new(WorldEpoch::new(7), 88),
        },
    ];
    for spawner in spawners {
        let mut spawner_out = Writer::new();
        spawner_out.put(&spawner);
        assert_eq!(
            Reader::decode::<Spawner>(spawner_out.bytes()).unwrap(),
            spawner
        );
    }
    assert_eq!(flags.iter().collect::<Vec<_>>(), ["minecraft:vanilla"]);
    // Independent bytes: one flag, then the 17-byte UTF-8 resource id.
    let expected = b"\x01\0\0\0\x11\0\0\0minecraft:vanilla";
    let mut out = Writer::new();
    out.put(&flags);
    assert_eq!(out.bytes(), expected);
    let decoded: FeatureFlags = Reader::decode(expected).unwrap();
    assert_eq!(decoded.iter().collect::<Vec<_>>(), ["minecraft:vanilla"]);
    let no_flags = [0, 0, 0, 0];
    let empty: FeatureFlags = Reader::decode(&no_flags).unwrap();
    assert!(!empty.contains("minecraft:vanilla"));
    assert_eq!(empty.iter().next(), None);
    out.clear();
    out.put(&empty);
    assert_eq!(out.bytes(), &no_flags);
}

#[test]
fn handle_records_have_declared_field_bytes() {
    let epoch = WorldEpoch::new(47);
    let level = LevelRef::new(epoch, DimensionKey::OVERWORLD);
    let entity = EntityRef::new(epoch, 29);
    let player = crate::__private::game::player_ref(entity, 0x1234);
    let pos = BlockPos { x: 7, y: -8, z: 9 };
    fields(level, (epoch, DimensionKey::OVERWORLD, 0u32));
    let reopened = LevelRef::with_generation(epoch, DimensionKey::NETHER, 5);
    fields(reopened, (epoch, DimensionKey::NETHER, 5u32));
    fields(ChunkRef::new(level, [3, -4]), (level, [3i32, -4]));
    fields(RaidRef::new(level, 53), (level, 53i32));
    fields(RaidRef::new(level, -1), (level, -1i32));
    fields(player, (entity, 0x1234u128));
    fields(
        GlobalPos::new(id("example:moon"), pos),
        (id("example:moon"), pos),
    );
    fields(
        BossBarRef {
            epoch,
            uuid: 0x5678,
        },
        (epoch, 0x5678u128),
    );
    fields(ObjectiveRef { epoch, id: 0x77 }, (epoch, 0x77u64));
    fields(TeamRef { epoch, id: 0x78 }, (epoch, 0x78u64));
    fields(
        MenuRef {
            player,
            window: -2,
            generation: 91,
        },
        (player, -2i32, 91u64),
    );
    fields(
        BlockEntityRef {
            level,
            position: [7, -8, 9],
            generation: 83,
        },
        (level, [7i32, -8, 9], 83u64),
    );
    fields(ModelRef::from_parts(31, 7).unwrap(), (31u64, 7u32));
    fields(
        crate::BakedModelKey::from_parts(43, 11).unwrap(),
        (43u64, 11u32),
    );
    fields(InstanceGroupRef::from_parts(37, 9).unwrap(), (37u64, 9u32));
}

#[test]
fn payload_enum_cases_have_declared_field_bytes() {
    let pos = [2i32, -4, 6];
    fields(
        crate::client::particles::PositionSource::Block([2, -4, 6]),
        (0u32, pos),
    );
    fields(
        crate::client::particles::PositionSource::Entity {
            id: 19,
            y_offset: 1.75,
        },
        (1u32, 19i32, 1.75f32),
    );
    fields(
        crate::client::particles::PositionSource::Entity {
            id: -1,
            y_offset: 1.5,
        },
        (1u32, -1i32, 1.5f32),
    );
    fields(TicketOwner::Block([1, -2, 3]), (0u32, [1i32, -2, 3]));
    fields(TicketOwner::Entity(0x1234), (1u32, 0x1234u128));
    fields(
        LevelDimension::Custom(LevelKey::try_from("example:moon".to_owned()).unwrap()),
        (3u32, LevelKey::try_from("example:moon".to_owned()).unwrap()),
    );
    fields(Tag::Int(-123), (3u32, -123i32));
    fields(Tag::LongArray(vec![7, -19]), (12u32, vec![7i64, -19]));
}

// Registration tables are scoped to this test thread, just like registrar callbacks.
struct Keys(Option<std::rc::Rc<std::cell::RefCell<sunrise_mod_runtime::registry_keys::Requests>>>);
impl Keys {
    fn new() -> Self {
        let mut requests = sunrise_mod_runtime::registry_keys::Requests::default();
        requests.items.request("example", "example:gear").unwrap();
        requests.blocks.request("example", "example:block").unwrap();
        requests.fluids.request("example", "example:fluid").unwrap();
        requests
            .block_entity_types
            .request("example", "example:machine")
            .unwrap();
        requests
            .particles
            .request("example", "example:particle")
            .unwrap();
        Self(
            crate::parts::REGISTERING_KEYS.with(|keys| {
                keys.replace(Some(std::rc::Rc::new(std::cell::RefCell::new(requests))))
            }),
        )
    }
}
impl Drop for Keys {
    fn drop(&mut self) {
        crate::parts::REGISTERING_KEYS.with(|keys| keys.replace(self.0.take()));
    }
}

#[test]
fn snapshot_records_have_declared_field_bytes() {
    let revision = crate::item::containers::ModRevision::from_snapshot(31, 47);
    fields(revision, (31u64, 47u64));
    let player_source = PlacementSource::Player {
        yaw: -45.0,
        pitch: 30.0,
        sneaking: true,
    };
    fields(player_source, (0u32, -45.0f32, 30.0f32, true));
    let directional_source = PlacementSource::Directional {
        direction: Direction::East,
    };
    fields(directional_source, (1u32, Direction::East));
    let pos = BlockPos { x: 1, y: 2, z: 3 };
    let hit_pos = BlockPos { x: 4, y: 5, z: 6 };
    let _keys = Keys::new();
    let item = ItemKey::from_slot(0).unwrap();
    let mut ctx = PlacementContext::new(pos, Direction::Up, Some(item));
    ctx.hit_pos = hit_pos;
    ctx.hit = [4.25, 5.5, 6.75];
    ctx.inside = true;
    ctx.source = player_source;
    ctx.replace_clicked = true;
    fields(
        ctx,
        (
            (pos, hit_pos, Direction::Up, [4.25f64, 5.5, 6.75], true),
            (player_source, Some(item), true),
        ),
    );
}

fn decode<T: Wire, F: Wire>(value: T) -> F {
    let mut out = Writer::new();
    out.put(&value);
    // Decoding as the independently typed tuple must consume exactly the encoded bytes.
    Reader::decode::<F>(out.bytes()).unwrap()
}

#[test]
fn attribute_records_have_declared_field_bytes() {
    let modifier = AttributeModifier::new(id("example:boost"), 1.75, AttributeOperation::AddValue);
    let entry = ItemAttributeModifier::new(
        id("minecraft:generic.attack_damage"),
        modifier.clone(),
        EquipmentSlotGroup::MainHand,
    );
    fields(
        entry.clone(),
        (
            id("minecraft:generic.attack_damage"),
            modifier,
            EquipmentSlotGroup::MainHand,
        ),
    );
    fields(
        ItemAttributeModifiers {
            entries: vec![entry.clone()],
            changed: true,
        },
        (vec![entry], true),
    );
    fields(
        ItemEnchantments {
            entries: vec![(id("minecraft:sharpness"), 3)],
            changed: true,
        },
        (vec![(id("minecraft:sharpness"), 3u32)], true),
    );
}

#[test]
fn remaining_enum_payload_fields_keep_tags_and_widths() {
    fields(
        NeighbourUpdate::NeighboursExcept(Direction::North),
        (2u32, Direction::North),
    );
    fields(Tag::Byte(-7), (1u32, -7i8));
    fields(Tag::Short(-1234), (2u32, -1234i16));
    fields(Tag::Long(-1234567), (4u32, -1234567i64));
    fields(Tag::Float(1.25f32.to_bits()), (5u32, 1.25f32));
    fields(Tag::Double((-3.75f64).to_bits()), (6u32, -3.75f64));
    fields(Tag::ByteArray(vec![255, 2, 3]), (7u32, vec![255u8, 2, 3]));
    fields(
        Tag::String(JavaString(vec![65, 0xd800])),
        (8u32, JavaString(vec![65, 0xd800])),
    );
    fields(Tag::List(vec![Tag::Int(17)]), (9u32, vec![Tag::Int(17)]));
    fields(Tag::Compound(Compound::new()), (10u32, Compound::new()));
    fields(Tag::IntArray(vec![-4, 7]), (11u32, vec![-4i32, 7]));
}

#[test]
fn newtype_widths_and_unit_enum_tags_match_their_encodings() {
    let _keys = Keys::new();
    fields(BlockKey::from_slot(0).unwrap(), (0u32,));
    fields(ItemKey::from_slot(0).unwrap(), (0u32,));
    fields(FluidKey::from_slot(0).unwrap(), (0u32,));
    fields(
        crate::UnresolvedFluid(id("absent:fluid")),
        (id("absent:fluid"),),
    );
    fields(WorldEpoch::new(47), (47u64,));
    fields(TickId::new(19), (19u64,));
    fields(
        BiomeKey::parse("minecraft:plains").unwrap(),
        (id("minecraft:plains"),),
    );
    fields(crate::__private::mod_color_resolver(13), (13u32,));
    fields(BlockUpdateFlags(17), (17u32,));
    fields(Reduction::Armor, (1u8,));
    fields(Direction::North, (2u32,));
    fields(EquipmentSlotGroup::Head, (7u32,));
    fields(EquipmentSlot::Head, (5u32,));
    fields(Tag::End, (0u32,));
}

#[test]
fn every_direction_and_equipment_case_has_declared_tag_type() {
    for (value, tag) in [
        (Direction::Down, 0u32),
        (Direction::Up, 1),
        (Direction::North, 2),
        (Direction::South, 3),
        (Direction::West, 4),
        (Direction::East, 5),
    ] {
        fields(value, (tag,));
    }
    for (value, tag) in [
        (EquipmentSlot::MainHand, 0u32),
        (EquipmentSlot::OffHand, 1),
        (EquipmentSlot::Feet, 2),
        (EquipmentSlot::Legs, 3),
        (EquipmentSlot::Chest, 4),
        (EquipmentSlot::Head, 5),
        (EquipmentSlot::Body, 6),
        (EquipmentSlot::Saddle, 7),
    ] {
        fields(value, (tag,));
    }
    for (value, tag) in [
        (EquipmentSlotGroup::Any, 0u32),
        (EquipmentSlotGroup::MainHand, 1),
        (EquipmentSlotGroup::OffHand, 2),
        (EquipmentSlotGroup::Hand, 3),
        (EquipmentSlotGroup::Feet, 4),
        (EquipmentSlotGroup::Legs, 5),
        (EquipmentSlotGroup::Chest, 6),
        (EquipmentSlotGroup::Head, 7),
        (EquipmentSlotGroup::Armor, 8),
        (EquipmentSlotGroup::Body, 9),
        (EquipmentSlotGroup::Saddle, 10),
    ] {
        fields(value, (tag,));
    }
}

#[test]
fn reduction_side_dimension_and_neighbour_cases_keep_their_tags() {
    for (value, tag) in [
        (Reduction::Invulnerability, 0u8),
        (Reduction::Armor, 1),
        (Reduction::Enchantments, 2),
        (Reduction::MobEffects, 3),
        (Reduction::Absorption, 4),
        (Reduction::InnateResistance, 5),
    ] {
        fields(value, (tag,));
    }
    fields(Side::Client, (0u32,));
    fields(Side::Both, (1u32,));
    fields(Side::Server, (2u32,));
    fields(LevelDimension::Overworld, (0u32,));
    fields(LevelDimension::Nether, (1u32,));
    fields(LevelDimension::End, (2u32,));
    fields(NeighbourUpdate::OutputSignal, (0u32,));
    fields(NeighbourUpdate::Neighbours, (1u32,));
    fields(
        NeighbourUpdate::NeighboursExcept(Direction::East),
        (2u32, Direction::East),
    );
}

#[test]
fn collision_exclusion_errors_use_the_clean_break_tags() {
    fields(CollisionExclusionsError::Stale, (0u32,));
    fields(CollisionExclusionsError::NotAPlayer, (1u32,));
    fields(CollisionExclusionsError::NotFinite, (2u32,));
    fields(CollisionExclusionsError::Inverted, (3u32,));
}

#[test]
fn every_game_mode_pose_animation_and_connection_case_has_declared_tag_type() {
    for (value, tag) in [
        (GameMode::Survival, 0u32),
        (GameMode::Creative, 1),
        (GameMode::Adventure, 2),
        (GameMode::Spectator, 3),
    ] {
        fields(value, (tag,));
    }
    for (value, tag) in [
        (ArmPose::Empty, 0u32),
        (ArmPose::Item, 1),
        (ArmPose::Block, 2),
        (ArmPose::Bow, 3),
        (ArmPose::CrossbowHold, 4),
        (ArmPose::CrossbowCharge, 5),
        (ArmPose::ThrowTrident, 6),
        (ArmPose::TootHorn, 7),
        (ArmPose::Spyglass, 8),
        (ArmPose::Spear, 9),
        (ArmPose::Brush, 10),
    ] {
        let mut output = Writer::new();
        output.put(&value);
        assert_eq!(output.bytes(), tag.to_le_bytes());
        assert_eq!(
            Reader::new(output.bytes()).take::<ArmPose>().unwrap(),
            value
        );
    }
    for (value, tag) in [
        (UseAnimation::None, 0u32),
        (UseAnimation::Eat, 1),
        (UseAnimation::Drink, 2),
        (UseAnimation::Block, 3),
        (UseAnimation::Bow, 4),
        (UseAnimation::Trident, 5),
        (UseAnimation::Crossbow, 6),
        (UseAnimation::Spyglass, 7),
        (UseAnimation::TootHorn, 8),
        (UseAnimation::Brush, 9),
        (UseAnimation::Bundle, 10),
        (UseAnimation::Spear, 11),
    ] {
        fields(value, (tag,));
    }
    fields(ConnectionKind::Fence, (0u32,));
    fields(ConnectionKind::Wall, (1u32,));
    fields(ConnectionKind::Bars, (2u32,));
}
