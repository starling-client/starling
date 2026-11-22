//! A mod crate's view of the mod API: `Hooks` is implemented with the names `mod_api` exports.
//! The test is that this file builds with the exported event types.
use mod_api::{
    self, ClientHost, Hooks, InputKeyEvent, KeyAction, ModEntry, ScreenExclusionAreasEvent,
};

#[derive(Default)]
struct Keys;

#[mod_api::sunrise_mod]
impl Hooks for Keys {
    fn input_key(&self, host: &mut dyn ClientHost, event: &mut InputKeyEvent<'_>) {
        let _: (&str, KeyAction) = (event.key, event.action);
        if let Some(menu) = host
            .local_player()
            .and_then(|player| player.container_menu(host))
        {
            let _ = menu.slot_count(host);
            let _ = menu.slot(host, 0);
            let _ = menu.carried(host);
        }
    }
}

#[test]
fn a_mod_implements_the_key_hooks() {
    let _: ModEntry = ModEntry::new("keys", hooks());
}

mod sidebar {
    use super::*;

    #[derive(Default)]
    struct Sidebar;

    #[mod_api::sunrise_mod]
    impl Hooks for Sidebar {
        fn screen_exclusion_areas(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenExclusionAreasEvent<'_>,
        ) {
            if event.screen.id == "inventory" {
                event.areas.push([0.0, 0.0, 40.0, event.screen.rect[3]]);
            }
        }
    }

    #[test]
    fn a_mod_adds_screen_exclusion_areas() {
        let _: ModEntry = ModEntry::new("sidebar", hooks());
    }
}

#[test]
fn client_packet_hands_a_mod_the_type_and_custom_payload() {
    use mod_api::{
        ClientPacket, ClientPacketEvent, ClientPayload, Identifier, dispatch_client_packet,
    };
    use mod_platform::testing::FakeHost;
    use std::sync::Mutex;
    static HEARD: Mutex<Vec<(String, Vec<u8>)>> = Mutex::new(Vec::new());
    // A plugin-channel listener using only the mod API's plain data.
    #[derive(Default)]
    struct Channels;
    #[mod_api::sunrise_mod]
    impl Hooks for Channels {
        fn client_packet(&self, _host: &mut dyn ClientHost, event: &mut ClientPacketEvent<'_>) {
            if let Some(p) = &event.packet.custom_payload {
                HEARD
                    .lock()
                    .unwrap()
                    .push((p.channel.clone(), p.data.clone()));
            }
        }
    }
    mod_platform::testing::with_mods(vec![ModEntry::new("channels", hooks())].into(), || {
        dispatch_client_packet(
            &mut FakeHost::new(),
            &ClientPacket::new(Identifier::parse("minecraft:ping").unwrap(), None),
        );
        let payload = ClientPayload::new("voicechat:secret".into(), vec![1, 2, 3]);
        dispatch_client_packet(
            &mut FakeHost::new(),
            &ClientPacket::new(
                Identifier::parse("minecraft:custom_payload").unwrap(),
                Some(payload),
            ),
        );
    });
    assert_eq!(
        *HEARD.lock().unwrap(),
        [("voicechat:secret".to_string(), vec![1, 2, 3])]
    );
}

// Fluid acceptance is scoped to a FLUIDS provider, not a global Hooks method.
// Its default/refused/accepted writes are covered with a real Game backend in
// game/blocks/machines/native_fluids.rs::tests::a_scoped_tank_can_refuse_a_fluid.

mod model_layers {
    use mod_api::{
        CubeListBuilder, Hooks, LayerDefinition, MeshDefinition, ModEntry, PartPose,
        RegisterLayerDefinitionsEvent, RegistrationContext,
    };

    struct Golem(&'static str);
    #[mod_api::__subscriptions]
    impl Hooks for Golem {
        // ANCHOR: register_layer_definitions
        fn register_layer_definitions(
            &self,
            _context: &RegistrationContext,
            event: &mut RegisterLayerDefinitionsEvent<'_>,
        ) {
            event.register(self.0, || {
                let mut mesh = MeshDefinition::new();
                let root = mesh.root();
                let head = CubeListBuilder::create()
                    .tex_offs(0, 0)
                    .add_box([-4.0, -8.0, -4.0], [8.0, 8.0, 8.0]);
                mesh.add_or_replace_child(root, "head", head, PartPose::offset(0.0, 6.0, 0.0));
                LayerDefinition::create(mesh, 64, 32)
            });
        }
        // ANCHOR_END: register_layer_definitions
    }

    #[test]
    fn register_layer_definitions_collect_each_mods_layers_in_order() {
        // The harness supplies the engine-owned context; the listener above uses only public API.
        use mod_api::__private::game::RegistrationContextNew;
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new("first", Box::new(Golem("first:golem#main"))),
                ModEntry::new("second", Box::new(Golem("second:golem#main"))),
            ]
            .into(),
            || {
                let mut layers = Vec::new();
                mod_api::dispatch_register_layer_definitions(
                    &RegistrationContext::new(),
                    &mut layers,
                );
                let owners_and_ids: Vec<_> = layers
                    .iter()
                    .map(|layer| (layer.mod_id, layer.answer.id.as_str()))
                    .collect();
                assert_eq!(
                    owners_and_ids,
                    [
                        ("first", "first:golem#main"),
                        ("second", "second:golem#main")
                    ]
                );
                let model = (layers[0].answer.supplier)().bake_root();
                let head = model.get_child("head").expect("the head part");
                assert_eq!((head.y, head.polygons.len()), (6.0, 6));
            },
        );
    }
}

mod skull_models {
    use mod_api::{CreateSkullModelsEvent, Hooks, ModEntry, RegistrationContext, SkullModel};

    struct Heads(&'static str);
    #[mod_api::__subscriptions]
    impl Hooks for Heads {
        // ANCHOR: create_skull_models
        fn create_skull_models(
            &self,
            _context: &RegistrationContext,
            event: &mut CreateSkullModelsEvent<'_>,
        ) {
            event.register(SkullModel {
                kind: format!("{}:goblin", self.0),
                layer: format!("{}:goblin_head#main", self.0),
                texture: format!("{}:entity/goblin", self.0),
            });
        }
        // ANCHOR_END: create_skull_models
    }

    #[test]
    fn create_skull_models_collects_each_mods_types_in_order() {
        use mod_api::__private::game::RegistrationContextNew;
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new("first", Box::new(Heads("first"))),
                ModEntry::new("second", Box::new(Heads("second"))),
            ]
            .into(),
            || {
                let mut models = Vec::new();
                mod_api::dispatch_create_skull_models(&RegistrationContext::new(), &mut models);
                let kinds: Vec<_> = models.iter().map(|m| m.kind.as_str()).collect();
                assert_eq!(kinds, ["first:goblin", "second:goblin"]);
                assert_eq!(models[1].layer, "second:goblin_head#main");
                assert_eq!(models[1].texture, "second:entity/goblin");
            },
        );
    }
}

#[test]
fn block_entity_additions_collect_in_order_and_quiet_dispatch_preserves_input() {
    use mod_api::__private::game::RegistrationContextNew;
    use mod_api::__private::parts_installation::{block_entity_type_key, block_key};
    use mod_api::{
        BlockEntityTypeAddBlocksEvent, BlockEntityTypeKey, BlockKey, RegistrationContext,
        dispatch_block_entity_type_add_blocks,
    };

    struct Listener {
        block: BlockKey,
        direct: BlockEntityTypeKey,
    }

    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn block_entity_type_add_blocks(
            &self,
            _: &RegistrationContext,
            event: &mut BlockEntityTypeAddBlocksEvent<'_>,
        ) {
            event.modify_matching(|kind| kind != self.direct, &[self.block]);
            event.modify(self.direct, &[self.block]);
        }
    }

    let [a, b, c] = [0, 1, 2].map(block_entity_type_key);
    let [first_block, second_block] = [0, 1].map(block_key);
    let types = [b, a, c];
    let seed = (c, second_block);
    let mut additions = vec![seed];

    mod_platform::testing::with_mods(Vec::<ModEntry>::new().into(), || {
        dispatch_block_entity_type_add_blocks(&RegistrationContext::new(), &types, &mut additions);
    });
    assert_eq!(additions, [seed]);

    mod_platform::testing::with_mods(
        vec![
            ModEntry::new(
                "first",
                Box::new(Listener {
                    block: first_block,
                    direct: c,
                }),
            ),
            ModEntry::new(
                "second",
                Box::new(Listener {
                    block: second_block,
                    direct: b,
                }),
            ),
        ]
        .into(),
        || {
            dispatch_block_entity_type_add_blocks(
                &RegistrationContext::new(),
                &types,
                &mut additions,
            );
        },
    );
    assert_eq!(
        additions,
        [
            seed,
            (b, first_block),
            (a, first_block),
            (c, first_block),
            (a, second_block),
            (c, second_block),
            (b, second_block),
        ]
    );
}

/// Each mod changes the prior edits, vanilla's answer stays readable, and
/// cancel stops later mods.
#[test]
fn an_anvil_update_is_changed_in_turn_and_cancel_stops_later_mods() {
    use mod_api::item::menu::{AnvilUpdateEdits, AnvilUpdateInputs};
    use mod_api::{AnvilUpdateEvent, Flow, Host, HostSide, ItemStack, ServerHost, StackResult};
    fn key(id: &str) -> mod_api::ItemKey {
        mod_api::__private::item_key(id).unwrap()
    }
    struct TestHost;
    impl Host for TestHost {}
    mod_api::impl_host_areas!(TestHost);
    mod_api::impl_server_host_areas!(TestHost);
    impl ServerHost for TestHost {}
    #[derive(Default)]
    struct Recipe;
    #[mod_api::__subscriptions]
    impl Hooks for Recipe {
        fn anvil_update(&self, _: HostSide<'_>, event: &mut AnvilUpdateEvent<'_>) -> Flow {
            let inputs = event.inputs;
            if inputs.name == Some("void") {
                return Flow::Cancel;
            }
            if inputs.left.item() == key("minecraft:iron_ingot")
                && inputs
                    .right
                    .is_some_and(|r| r.item() == key("minecraft:emerald"))
            {
                event.edits.output =
                    StackResult::Set(Some(ItemStack::of(key("minecraft:diamond"))));
                event.edits.xp_cost = 5;
                event.edits.material_cost = 1;
            }
            Flow::Continue
        }
    }
    #[derive(Default)]
    struct Discount;
    #[mod_api::__subscriptions]
    impl Hooks for Discount {
        fn anvil_update(&self, _: HostSide<'_>, event: &mut AnvilUpdateEvent<'_>) -> Flow {
            assert_eq!(event.inputs.vanilla_xp_cost, 3);
            assert_eq!(event.inputs.vanilla_material_cost, 0);
            assert_eq!(
                event.inputs.vanilla_output.unwrap().item(),
                key("minecraft:iron_sword")
            );
            event.edits.xp_cost -= 1;
            Flow::Continue
        }
    }
    let mods: mod_api::Mods = vec![
        ModEntry::new("recipe", Box::new(Recipe)),
        ModEntry::new("discount", Box::new(Discount)),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        mod_platform::__private::install_parts(
            &[],
            &mod_platform::__private::part_tests::rule_schema(),
            |_| false,
        )
        .unwrap();
        mod_api::__private::resolve_items([
            "minecraft:iron_sword",
            "minecraft:iron_ingot",
            "minecraft:emerald",
            "minecraft:diamond",
        ])
        .unwrap();
        let vanilla_output = ItemStack::of(key("minecraft:iron_sword"));
        let update = |left: &str, right: Option<&str>, name: Option<&str>| {
            let left = ItemStack::of(key(left));
            let right = right.map(|r| ItemStack::new(key(r), 4).unwrap());
            let inputs = AnvilUpdateInputs {
                left: &left,
                right: right.as_ref(),
                name,
                vanilla_output: Some(&vanilla_output),
                vanilla_xp_cost: 3,
                vanilla_material_cost: 0,
            };
            let mut edits = AnvilUpdateEdits {
                output: StackResult::Keep,
                xp_cost: inputs.vanilla_xp_cost,
                material_cost: inputs.vanilla_material_cost,
            };
            let mut cancel = false;
            mod_api::dispatch_anvil_update(
                HostSide::Server(&mut TestHost),
                7,
                1,
                [0, 64, 0],
                &inputs,
                &mut edits,
                &mut cancel,
            );
            (edits.output, edits.xp_cost, edits.material_cost, cancel)
        };
        assert_eq!(
            update("minecraft:iron_ingot", Some("minecraft:emerald"), None),
            (
                StackResult::Set(Some(ItemStack::of(key("minecraft:diamond")))),
                4,
                1,
                false
            )
        );
        assert_eq!(
            update("minecraft:iron_sword", None, Some("")),
            (StackResult::Keep, 2, 0, false)
        );
        assert_eq!(
            update("minecraft:iron_sword", None, Some("void")),
            (StackResult::Keep, 3, 0, true)
        );
    });
}

#[test]
fn wind_burst_filter_preserves_order_and_context() {
    use mod_api::{Host, ServerHost, WindBurstEvent, dispatch_wind_burst};
    struct TestHost;
    impl Host for TestHost {}
    mod_api::impl_host_areas!(TestHost);
    mod_api::impl_server_host_areas!(TestHost);
    impl ServerHost for TestHost {}
    struct Claims;
    #[mod_api::__subscriptions]
    impl Hooks for Claims {
        fn wind_burst(&self, _host: &mut dyn ServerHost, event: &mut WindBurstEvent<'_>) {
            assert_eq!(event.dimension, "minecraft:the_nether");
            assert_eq!(event.center, [1.5, 70.0, -2.5]);
            event.blocks.retain(|p| p[0] != 1);
        }
    }
    let mods: mod_api::Mods = vec![ModEntry::new("claims", Box::new(Claims))].into();
    mod_platform::testing::with_mods(mods, || {
        let mut blocks = vec![[3, 70, -2], [1, 70, -2], [2, 70, -2]];
        dispatch_wind_burst(
            &mut TestHost,
            "minecraft:the_nether",
            [1.5, 70.0, -2.5],
            &mut blocks,
        );
        assert_eq!(blocks, vec![[3, 70, -2], [2, 70, -2]]);
    });
}
