//! `ItemPart::item_name` (`Item.getName(ItemStack)`) behind every place the game names a stack.
use super::native_entity_refs::tests::{run_fresh_cases, with_game};
use mod_api::{Hooks, ItemNameEvent, ItemPart, LoopContext, Registrar, RegistrationContext};
use sunrise_client::items::ItemStack;
use sunrise_core::nbt::Tag;

const NAME: &str = "game::item_name_tests::a_part_names_its_stacks_everywhere_the_game_names_them";
const RENAME_BOX: &str =
    "game::item_name_tests::a_client_anvil_click_fills_the_rename_box_with_the_parts_name";
const KEY: &str = "item.test.bundle_of_notes";

/// A paper's Part: a stack of more than one is named by `KEY`, one is left to the default.
#[derive(Clone)]
struct Notes;

#[mod_api::__subscriptions]
impl Hooks for Notes {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.item_key("minecraft:paper");
        r.item_part(key, self.clone());
    }
}
#[mod_api::sunrise_part]
impl ItemPart for Notes {
    fn item_name(&self, _: &dyn LoopContext, event: &mut ItemNameEvent<'_>) {
        if event.stack.count() > 1 {
            *event.name = Some(mod_api::Text::translate(KEY, Vec::new()));
        }
    }
}

/// The Part's name wins over the item's own for its stacks, `item_name` component included, in the
/// hover name, the styled tooltip name and the anvil; a stack it leaves keeps the default, its
/// `item_name` included; a custom name wins over it; another item isn't asked. The anvil keeps an
/// unchanged Part name and renames to the item's base name. In its own process: the game it opens
/// publishes process-wide tables.
#[test]
#[ignore = "daily"]
fn a_part_names_its_stacks_everywhere_the_game_names_them() {
    if run_fresh_cases(NAME, "SUNRISE_ITEM_NAME_CHILD", &["names"]) {
        return;
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("names", Box::new(Notes))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        crate::entry::install_client_hooks();
        with_game(|game| {
            game.resolve_parts().unwrap();
            let plain = |t: sunrise_client::text::Text| t.plain();
            // An untranslated key reads as itself: the Part's name, independent of the code.
            let named = ItemStack::new("minecraft:paper", 2);
            assert_eq!(plain(game.stack_hover_name_text(&named)), KEY);
            assert_eq!(plain(game.styled_stack_name(&named)), KEY);

            // The Part replaces an explicit `item_name`; a stack it leaves keeps it.
            let mut labelled = named.clone();
            labelled
                .data_mut()
                .set_other("minecraft:item_name", Some(Tag::String("Label".into())));
            assert_eq!(plain(game.stack_hover_name_text(&labelled)), KEY);
            let mut single = labelled.clone();
            single.count = 1;
            assert_eq!(plain(game.stack_hover_name_text(&single)), "Label");
            assert_eq!(game.mod_item_name(&single), None);

            // A plain single sheet and another item: vanilla's name, no Part asked.
            let sheet = ItemStack::new("minecraft:paper", 1);
            assert_eq!(
                game.stack_hover_name_text(&sheet).component(),
                sunrise_client::stations::hover_name_text(&game.pack.names, &sheet).component()
            );
            assert_eq!(
                game.mod_item_name(&ItemStack::new("minecraft:stone", 2)),
                None
            );

            // `ItemStack.getHoverName`: a custom name wins over the Part's.
            let mut renamed = named.clone();
            renamed.data_mut().custom_name = Some(Tag::String("Diary".into()));
            assert_eq!(plain(game.stack_hover_name_text(&renamed)), "Diary");

            // `ItemStack.getItemName` (`ItemEntity.getName` under its own custom name): the Part's
            // name, the stack's custom name not part of it; a stack the Part leaves keeps its own.
            assert_eq!(plain(game.stack_item_name_text(&renamed)), KEY);
            assert_eq!(plain(game.stack_item_name_text(&single)), "Label");

            // The tooltip's and the hotbar's styled name is the same resolved name.
            assert_eq!(plain(game.styled_stack_name(&labelled)), KEY);
            assert_eq!(plain(game.styled_stack_name(&single)), "Label");

            // The anvil menu itself (`AnvilMenu.createResult`, as the server runs it): the
            // Part's name typed back unchanged makes no result; the item's base name, an
            // edit, renames it; a third name renames it too.
            let env = game.menu_env();
            let anvil = |name: &str| {
                let mut menu = sunrise_client::menu::Menu::new(sunrise_client::menu::Kind::Anvil);
                menu.set_slot(&env, 0, Some(named.clone()));
                menu.anvil_created();
                menu.rename(&env, name);
                menu.slot(2).cloned()
            };
            assert_eq!(anvil(KEY), None, "an unchanged name renames nothing");
            let base = anvil("Paper").expect("the base name is a rename");
            assert_eq!(base.custom_name().as_deref(), Some("Paper"));
            let other = anvil("Minutes").expect("another name is a rename");
            assert_eq!(other.custom_name().as_deref(), Some("Minutes"));

            // ChargedProjectiles.addProjectileTooltip uses getDisplayName for a single arrow.
            let mut arrow = ItemStack::new("minecraft:arrow", 1);
            arrow
                .data_mut()
                .set_other("minecraft:rarity", Some(Tag::String("epic".into())));
            let mut crossbow = ItemStack::new("minecraft:crossbow", 1);
            crossbow.data_mut().charged.push(arrow);
            let tooltip = game.stack_tooltip(&crossbow);
            let (projectile, _) = tooltip
                .iter()
                .find(|(line, _)| line.plain().ends_with("[Arrow]"))
                .expect("a single projectile's bracketed display name");
            assert!(
                projectile.spans.iter().any(|span| {
                    span.text.contains("Arrow") && span.style.color == Some(0xFF55FF)
                })
            );

            // GiveCommand's refused count still links the one-item prototype, with its patch.
            game.player = Some(
                sunrise_client::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.set_allow_commands(true);
            game.build_registries();
            let refusal = game
                .execute_line(
                    r#"give @s minecraft:paper[minecraft:custom_name={text:"Diary"}] 6401"#,
                    game.player_source(),
                )
                .unwrap_err();
            let message = refusal.message.component();
            assert_eq!(message["translate"], "commands.give.failed.toomanyitems");
            let hover = &message["with"][1]["hover_event"];
            assert_eq!(hover["action"], "show_item");
            let shown = sunrise_client::item_data::stack_from_hover(hover).unwrap();
            assert_eq!(&*shown.item, "minecraft:paper");
            assert_eq!(shown.count, 1);
            assert_eq!(shown.custom_name().as_deref(), Some("Diary"));

            // EnchantCommand's refusal names the held stack by `getHoverName().getString()`.
            game.inventory[game.selected] = Some(named.clone());
            let refusal = game
                .execute_line("enchant @s minecraft:sharpness", game.player_source())
                .unwrap_err();
            let message = refusal.message.component();
            assert_eq!(message["translate"], "commands.enchant.failed.incompatible");
            assert!(refusal.message.plain().contains(KEY), "{message}");
            game.inventory[game.selected] = None;

            // ItemEntity.getName: the dropped item takes its name from the Part, not the
            // stack's custom hover name. Command feedback and `name=` selectors agree.
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let run = |game: &mut super::Game<'_>, line: &str| {
                game.execute_line(line, game.player_source())
                    .unwrap_or_else(|e| panic!("{line}: {}", e.message.plain()));
                game.ui.chat.recent_lines(1).concat()
            };
            let summoned = run(
                game,
                r#"summon minecraft:item 4.5 200 4.5 {Item:{id:"minecraft:paper",count:2,components:{"minecraft:custom_name":{text:"Diary"}}}}"#,
            );
            assert!(summoned.contains(KEY), "{summoned}");
            assert_eq!(
                game.dropped[0].stack.custom_name().as_deref(),
                Some("Diary")
            );
            run(
                game,
                r#"summon minecraft:item 6.5 200 4.5 {Item:{id:"minecraft:paper",count:1}}"#,
            );
            assert_eq!(game.dropped.len(), 2);
            let killed = run(game, &format!(r#"kill @e[type=item,name="{KEY}"]"#));
            assert!(killed.contains(KEY), "{killed}");
            assert_eq!(game.dropped.len(), 1);
            assert_eq!(game.dropped[0].stack.count, 1);
            let killed = run(game, r#"kill @e[type=item,name="Paper"]"#);
            assert!(killed.contains("Paper"), "{killed}");
            assert!(game.dropped.is_empty());

            // An entity's own custom name takes precedence over the item name.
            let summoned = run(
                game,
                r#"summon minecraft:item 4.5 200 4.5 {CustomName:{text:"Loose Notes"},Item:{id:"minecraft:paper",count:2,components:{"minecraft:custom_name":{text:"Diary"}}}}"#,
            );
            assert!(summoned.contains("Loose Notes"), "{summoned}");
            let killed = run(game, r#"kill @e[type=item,name="Loose Notes"]"#);
            assert!(killed.contains("Loose Notes"), "{killed}");
            assert!(game.dropped.is_empty());

            // EnchantCommand on a mob: its main-hand stack's getHoverName().getString(), the
            // Part's name for the stack of two it holds.
            run(game, "summon minecraft:zombie 4.5 200 4.5 {NoAI:1b}");
            let zombie = game
                .mobs
                .iter_mut()
                .find(|m| m.kind.id == "minecraft:zombie")
                .expect("the summoned zombie");
            zombie.extra.set_main_hand_stack(Some(named.clone()));
            let refusal = game
                .execute_line(
                    "enchant @e[type=zombie,limit=1] minecraft:sharpness",
                    game.player_source(),
                )
                .unwrap_err();
            let message = refusal.message.component();
            assert_eq!(message["translate"], "commands.enchant.failed.incompatible");
            assert!(refusal.message.plain().contains(KEY), "{message}");
            run(game, "kill @e[type=zombie]");

            // The screen uses the same comparison before sending the rename packet.
            let mut station = super::stations::Station::new(super::stations::Kind::Anvil, [0; 3]);
            station.inputs[0] = Some(named);
            station.name = KEY.into();
            assert_eq!(game.anvil_item_name(&station), "");
            station.name = "Paper".into();
            assert_eq!(game.anvil_item_name(&station), "Paper");
        });
    });
}

/// `AnvilScreen.slotChanged` after a client's anvil click on a server's menu (the click predicted
/// by `anvil_menu_session`, then `anvil_session_end`): the box is set to the input's
/// `getHoverName().getString()` and made editable; a custom name wins over the Part's
/// `Item.getName(stack)`; a stack the Part leaves shows vanilla's name; taking the input out sets
/// the box to "" and makes it read-only. In its own process, as the test above.
#[test]
#[ignore = "daily"]
fn a_client_anvil_click_fills_the_rename_box_with_the_parts_name() {
    use super::execution_role::ExecutionRole;
    use super::stations::{INPUT, Kind, Station};
    use sunrise_client::menu::Input;
    if run_fresh_cases(RENAME_BOX, "SUNRISE_ITEM_NAME_RENAME_BOX_CHILD", &["names"]) {
        return;
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("names", Box::new(Notes))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        crate::entry::install_client_hooks();
        with_game(|game| {
            game.resolve_parts().unwrap();
            game.options.execution_role = ExecutionRole::Client;
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "AnvilTest").unwrap();
            game.remote_set_menu(Some((1, "minecraft:anvil".into())), None);
            game.station = Some(Station::new_kind(Kind::Anvil, [0, 64, 0]));
            let rename_box = |game: &super::Game<'_>| {
                let station = game.station.as_ref().unwrap();
                (station.rename.value.clone(), station.rename.editable)
            };
            // One left click on the input slot: puts the carried stack in, or takes it out.
            let click = |game: &mut super::Game<'_>| {
                assert!(game.remote_menu_ops(&[(Some(INPUT), 0, Input::Pickup)]));
            };
            let input = |game: &super::Game<'_>| game.station.as_ref().unwrap().inputs[0].clone();

            // The Part names a stack of two: the box shows its name, as the anvil menu compares.
            let notes = ItemStack::new("minecraft:paper", 2);
            game.carried = Some(notes.clone());
            click(game);
            assert_eq!(input(game), Some(notes.clone()));
            assert_eq!(rename_box(game), (KEY.to_owned(), true));

            // Taken out again: `setValue("")`, read-only.
            click(game);
            assert_eq!(input(game), None);
            assert_eq!(game.carried, Some(notes.clone()));
            assert_eq!(rename_box(game), (String::new(), false));

            // `ItemStack.getHoverName`: the custom name wins over the Part's name.
            let mut diary = notes.clone();
            diary.data_mut().custom_name = Some(Tag::String("Diary".into()));
            game.carried = Some(diary);
            click(game);
            assert_eq!(rename_box(game), ("Diary".to_owned(), true));
            click(game);
            assert_eq!(rename_box(game), (String::new(), false));

            // One sheet, which the Part leaves: vanilla's name, as a vanilla client sends it.
            let sheet = ItemStack::new("minecraft:paper", 1);
            let vanilla = sunrise_client::lang::returned_to_server(|| {
                sunrise_client::stations::hover_name_text(&game.pack.names, &sheet).plain()
            });
            assert_eq!(vanilla, "Paper");
            game.carried = Some(sheet);
            click(game);
            assert_eq!(rename_box(game), (vanilla, true));
        });
    });
}
