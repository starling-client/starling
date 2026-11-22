//! `ItemPart::item_name` (`Item.getName(ItemStack)`) behind every place the game names a stack.
use super::native_entity_refs::tests::{run_fresh_cases, with_game};
use mod_api::{Hooks, ItemNameEvent, ItemPart, LoopContext, Registrar, RegistrationContext};
use sunrise_client::items::ItemStack;
use sunrise_core::nbt::Tag;

const NAME: &str = "game::item_name_tests::a_part_names_its_stacks_everywhere_the_game_names_them";
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
