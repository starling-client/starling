// Damage presence, normalization and the public NBT escape share the real API implementation.
struct Definitions;
impl crate::prelude::ItemsHost for Definitions {
    fn item_key(&self, id: &crate::Identifier) -> Option<ItemKey> {
        match id.to_string().as_str() {
            "minecraft:iron_sword" => Some(ItemKey(0)),
            "minecraft:stone" => Some(ItemKey(1)),
            _ => None,
        }
    }
    fn item_default_component(&self, item: ItemKey, id: &str) -> Option<Tag> {
        match (item.0, id) {
            (0, "minecraft:damage") => Some(Tag::Int(0)),
            (0, "minecraft:max_damage") => Some(Tag::Int(250)),
            _ => None,
        }
    }
}
impl crate::host::keys::KeysBackend for Definitions {
    fn item_key_id(&self, item: ItemKey) -> Option<&str> {
        ["minecraft:iron_sword", "minecraft:stone"]
            .get(item.0 as usize)
            .copied()
    }
}
impl crate::Host for Definitions {
    fn backend(&self) -> Option<crate::BackendRef<'_>> {
        Some(crate::BackendRef::of(self))
    }
}
crate::impl_host_areas!(Definitions, except(items));
impl crate::prelude::ItemsBackend for Definitions {
    /// `ItemStack.isDamageableItem` over this fixture's prototypes: maximum damage and damage
    /// after the stack's patch, and no unbreakable component.
    fn item_resource_damageable(&self, stack: &ItemStack) -> bool {
        let resource = crate::ItemResource::of(stack);
        let has = |id: &str| {
            resource
                .value(self, &crate::ComponentId::new(id).unwrap())
                .is_some()
        };
        has("minecraft:max_damage") && has("minecraft:damage") && !has("minecraft:unbreakable")
    }
}
crate::impl_backend_areas!(Definitions, except(keys, items));
impl crate::Backend for Definitions {}

#[test]
fn prototype_damage_explicit_zero_and_removal_round_trip_distinctly() {
    let plain = ItemStack::of(ItemKey(0));
    let id = crate::ComponentId::new("minecraft:damage").unwrap();
    assert!(plain.components().is_none());
    assert_eq!(plain.damage_component(), None);
    assert_eq!(
        crate::ItemResource::of(&plain).value(&Definitions, &id),
        Some(Tag::Int(0))
    );
    let mut removed = plain.clone();
    removed.clear_damage();
    assert_eq!(
        crate::ItemResource::of(&removed).value(&Definitions, &id),
        None
    );
    let mut explicit = removed.clone();
    explicit.set_damage(0);
    assert_eq!(explicit.damage_component(), Some(0));
    assert!(explicit.components().is_none());
    assert_eq!(
        crate::ItemResource::of(&explicit).value(&Definitions, &id),
        Some(Tag::Int(0))
    );
    for stack in [plain, removed, explicit] {
        let nbt = stack.to_nbt(&Definitions).unwrap();
        assert_eq!(ItemStack::from_nbt(&Definitions, &nbt), Some(stack));
    }
}

#[test]
fn a_maximum_override_does_not_invent_a_damage_component() {
    let mut stack = ItemStack::of(ItemKey(1));
    stack
        .set_components([(JavaString::from("minecraft:max_damage"), Tag::Int(100))].into())
        .unwrap();
    assert!(!crate::ItemResource::of(&stack).damageable(&Definitions));
    stack.set_damage(0);
    assert!(crate::ItemResource::of(&stack).damageable(&Definitions));
    assert!(
        !stack
            .components()
            .unwrap()
            .as_nbt()
            .contains_key(&JavaString::from("minecraft:damage"))
    );
    stack.clear_damage();
    assert!(!crate::ItemResource::of(&stack).damageable(&Definitions));
}

#[test]
fn raw_damage_uses_the_vanilla_numeric_codec_on_owned_and_staged_writes() {
    let patch: Compound = [("minecraft:damage".into(), Tag::Float(2.75_f32.to_bits()))].into();
    let nbt = [
        ("id".into(), Tag::String("minecraft:iron_sword".into())),
        ("components".into(), Tag::Compound(patch.clone())),
    ]
    .into();
    let mut stack = ItemStack::from_nbt(&Definitions, &nbt).unwrap();
    assert_eq!(stack.damage_component(), Some(2));

    stack.set_damage(5);
    stack.set_components(patch.clone()).unwrap();
    assert_eq!(stack.damage_component(), Some(2));
    assert!(stack.components().is_none());

    stack.set_damage(5);
    stack.apply_components(&patch).unwrap();
    assert_eq!(stack.damage_component(), Some(2));

    stack.set_damage(5);
    let view = ItemStackView::from_stack(&stack);
    let edits = edit_item_components(view, &|_| None, |editor| {
        editor.apply_components(&patch).unwrap();
        assert_eq!(editor.damage_component(), Some(2));
    });
    edits.apply_to(&mut stack);
    assert_eq!(stack.damage_component(), Some(2));
}

#[test]
fn malformed_damage_refuses_while_removal_wins() {
    struct RemoveDamage;
    impl Component for RemoveDamage {
        type Value = ();
        fn id(&self) -> &str {
            "!minecraft:damage"
        }
        fn decode(&self, _: &Tag) -> Option<()> {
            None
        }
        fn encode(&self, _: ()) -> Tag {
            Tag::Compound(Compound::new())
        }
    }

    let mut stack = ItemStack::of(ItemKey(0));
    for value in [
        Tag::Int(-1),
        Tag::String("bad".into()),
        Tag::Long(i64::MAX),
        Tag::Double((-1.25_f64).to_bits()),
    ] {
        let before = stack.clone();
        assert_eq!(
            stack.set_components([(JavaString::from("minecraft:damage"), value)].into()),
            Err(StackError::InvalidDamage)
        );
        assert_eq!(stack, before);
    }
    let before = stack.clone();
    assert_eq!(
        stack.set_components(
            [
                (JavaString::from("minecraft:damage"), Tag::Int(-1)),
                (
                    JavaString::from("!minecraft:damage"),
                    Tag::Compound(Compound::new()),
                ),
            ]
            .into()
        ),
        Err(StackError::InvalidDamage)
    );
    assert_eq!(stack, before);
    assert_eq!(
        stack.set_components([(JavaString::from("!minecraft:damage"), Tag::Int(0))].into()),
        Err(StackError::InvalidDamage)
    );
    assert_eq!(stack, before);

    // Explicit `!minecraft:damage` clears prior damage and wins over `minecraft:damage`.
    stack.set_damage(5);
    stack
        .set_components(
            [
                (JavaString::from("minecraft:damage"), Tag::Int(9)),
                (
                    JavaString::from("!minecraft:damage"),
                    Tag::Compound(Compound::new()),
                ),
            ]
            .into(),
        )
        .unwrap();
    assert_eq!(stack.damage_component(), None);
    let patch = stack.components().unwrap().as_nbt();
    assert!(!patch.contains_key(&JavaString::from("minecraft:damage")));
    assert!(patch.contains_key(&JavaString::from("!minecraft:damage")));

    // Direct setter for `!minecraft:damage` clears existing damage on both ItemStack and editor.
    stack.set_damage(4);
    stack.set(&RemoveDamage, ()).unwrap();
    assert_eq!(stack.damage_component(), None);
    let patch = stack.components().unwrap().as_nbt();
    assert!(!patch.contains_key(&JavaString::from("minecraft:damage")));
    assert!(patch.contains_key(&JavaString::from("!minecraft:damage")));

    stack.set_damage(6);
    let view = ItemStackView::from_stack(&stack);
    let read_override = |_: &str| -> Option<Tag> { None };
    let edits = edit_item_components(view, &read_override, |editor| {
        assert_eq!(editor.damage_component(), Some(6));
        editor.set(&RemoveDamage, ()).unwrap();
        assert_eq!(editor.damage_component(), None);
    });
    edits.apply_to(&mut stack);
    assert_eq!(stack.damage_component(), None);
    assert!(
        stack
            .components()
            .unwrap()
            .as_nbt()
            .contains_key(&JavaString::from("!minecraft:damage"))
    );
}
