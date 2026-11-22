//! Real LAN merchant result takes, with native listeners and the production coordinator.
use super::{Game, native_guest_turn_tests::with_guests};
use mod_api::{
    Flow, Hooks, HostSide, HostTransaction, ItemPlace, ItemResource, ItemStackedOnOtherEvent,
    ModEntry, Mods, PlayerTradeEvent, ServerHost, TradeWithVillagerEvent,
};
use std::cell::{Cell, RefCell};
use std::sync::{Arc, mpsc};
use sunrise_client::lan::{Out, PlayerSender};
use sunrise_client::{
    items::ItemStack,
    merchant::{Session, Trader},
    mobs::finalize::{self, Attr, Op},
};
use sunrise_protocol::packets::play::{Clientbound, ContainerClick};

#[derive(Clone, Copy)]
enum PlayerWrite {
    Unrelated,
    ShiftDestination { restore: bool },
}

#[derive(Clone, Copy)]
enum EffectWrite {
    ReplaceSlowness,
    ReapplySpeed,
    Clear,
    ReplaceTransient,
}

const SPEED: &str = "minecraft:movement_speed";

#[derive(Clone, Default)]
struct Script {
    panic_on: Option<usize>,
    panic_on_completion: bool,
    answer: bool,
    invalid: bool,
    /// An answer whose result stack is not a valid one for the game.
    invalid_stack: bool,
    edit_other_merchant_and_board: bool,
    effect_write: Option<EffectWrite>,
    player_write: Option<PlayerWrite>,
    raise_second_price: bool,
    refuse_stacked: bool,
    replace_stacked_result: bool,
}
thread_local! {
    static SCRIPT: RefCell<Script> = RefCell::new(Script::default());
    static CALLS: Cell<usize> = const { Cell::new(0) };
    static STACKED: Cell<usize> = const { Cell::new(0) };
    static COMPLETED: Cell<usize> = const { Cell::new(0) };
    static STAGED_WRITES: Cell<bool> = const { Cell::new(false) };
    static USES: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    static IN_ACTION: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}
fn script(value: Script) {
    SCRIPT.with(|s| *s.borrow_mut() = value);
    CALLS.set(0);
    STACKED.set(0);
    COMPLETED.set(0);
    STAGED_WRITES.set(false);
    USES.with(|uses| uses.borrow_mut().clear());
    IN_ACTION.with(|a| a.borrow_mut().clear());
    super::decided_action::DECIDED.set(0);
}
struct Mod;
#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn trade_with_villager(
        &self,
        host: &mut dyn ServerHost,
        event: &mut TradeWithVillagerEvent<'_>,
    ) {
        trade(host, event);
    }

    fn player_trade(&self, _: &mut dyn ServerHost, _: &mut PlayerTradeEvent<'_>) {
        COMPLETED.set(COMPLETED.get() + 1);
        assert!(
            !SCRIPT.with(|s| s.borrow().panic_on_completion),
            "completion listener failed"
        );
    }
}
/// The same trade listener, also hearing `item_stacked_on_other`: the result pickup is both.
struct Both;
#[mod_api::__subscriptions]
impl Hooks for Both {
    fn trade_with_villager(
        &self,
        host: &mut dyn ServerHost,
        event: &mut TradeWithVillagerEvent<'_>,
    ) {
        trade(host, event);
    }

    fn item_stacked_on_other(
        &self,
        host: HostSide,
        event: &mut ItemStackedOnOtherEvent<'_>,
    ) -> Flow {
        Stacked.item_stacked_on_other(host, event)
    }
}

struct Stacked;
#[mod_api::__subscriptions]
impl Hooks for Stacked {
    fn item_stacked_on_other(&self, _: HostSide, event: &mut ItemStackedOnOtherEvent<'_>) -> Flow {
        STACKED.set(STACKED.get() + 1);
        let (refuse, replace) = SCRIPT.with(|s| {
            let s = s.borrow();
            (s.refuse_stacked, s.replace_stacked_result)
        });
        assert!(!refuse, "stacked listener failed");
        if replace {
            event.result.handled = true;
            event.result.clicked = mod_api::ClickedStackResult::Set(None);
            event.cancelled = true;
            return Flow::Cancel;
        }
        Flow::Continue
    }
}
fn trade(host: &mut dyn ServerHost, event: &mut TradeWithVillagerEvent<'_>) {
    {
        let call = CALLS.get() + 1;
        CALLS.set(call);
        IN_ACTION.with(|a| a.borrow_mut().push(host.transaction_current().is_some()));
        assert_eq!(event.merchant, 40);
        USES.with(|uses| uses.borrow_mut().push(event.offer.uses));
        let s = SCRIPT.with(|s| s.borrow().clone());
        if let Some(write) = s.player_write {
            let key = mod_api::__private::item_key("minecraft:dirt").unwrap();
            let dirt = ItemResource::of(&mod_api::ItemStack::new(key, 1).unwrap());
            let place = ItemPlace::PlayerSlot {
                player: 12,
                slot: match write {
                    PlayerWrite::Unrelated => 20,
                    PlayerWrite::ShiftDestination { .. } => 8,
                },
            };
            let tx = host
                .transaction_current()
                .expect("trade listener is inside the action");
            assert_eq!(place.insert(host, &tx, &dirt, 2), 2);
            if matches!(write, PlayerWrite::ShiftDestination { restore: true }) {
                assert_eq!(place.extract(host, &tx, &dirt, 2), 2);
            }
            STAGED_WRITES.set(true);
        }
        if s.raise_second_price && call == 2 {
            event.offer.special_price_diff = 1;
        }
        if s.answer {
            event.offer.uses = 6;
            event.offer.max_uses = 19;
            event.offer.demand = -7;
            event.offer.special_price_diff = -4;
            event.offer.price_multiplier = 0.25;
            event.offer.xp = 11;
        }
        if s.invalid {
            event.offer.price_multiplier = f32::NAN;
        }
        if s.invalid_stack {
            event.offer.result.set_damage(u32::MAX);
        }
        if s.edit_other_merchant_and_board {
            let other = host.merchant(host.entity(41).unwrap()).unwrap();
            let mut offer = other.offers(host).unwrap().remove(0);
            offer.uses = 6;
            assert!(other.set_offer(host, 0, offer).unwrap());
            host.add_objective(
                "temporary",
                "dummy",
                mod_api::Text::literal("Temporary"),
                mod_api::ObjectiveRenderType::Integer,
            )
            .unwrap();
        }
        if let Some(write) = s.effect_write {
            let other = host.living(host.entity(41).unwrap()).unwrap();
            let key = |id| mod_api::__private::effect_key(id).unwrap();
            let add = |host: &mut dyn ServerHost, id| {
                let effect = mod_api::ActiveEffect::new(key(id), 200, 0, false, true, true);
                assert!(other.add_effect(host, &effect));
                assert!(other.potion_affected(host, effect.id));
            };
            match write {
                EffectWrite::ReplaceSlowness => {
                    assert!(other.remove_effect(host, key("minecraft:slowness")));
                    add(host, "minecraft:speed");
                    add(host, "minecraft:absorption");
                }
                EffectWrite::ReapplySpeed => {
                    assert!(other.remove_effect(host, key("minecraft:speed")));
                    assert!(!other.potion_affected(host, key("minecraft:speed")));
                    add(host, "minecraft:speed");
                }
                EffectWrite::Clear => {
                    host.transaction(|host, _| {
                        assert!(other.remove_all_effects(host));
                        Ok::<_, ()>(())
                    })
                    .unwrap();
                    assert!(!other.potion_affected(host, key("minecraft:speed")));
                    assert!(!other.potion_affected(host, key("minecraft:slowness")));
                }
                EffectWrite::ReplaceTransient => {
                    add(host, "minecraft:speed");
                    let explicit = mod_api::AttributeModifier::new(
                        mod_api::Identifier::parse("test:explicit").unwrap(),
                        0.0625,
                        mod_api::AttributeOperation::AddValue,
                    );
                    assert!(other.add_attribute_modifier(
                        host,
                        SPEED,
                        explicit,
                        mod_api::ModifierLifetime::Transient
                    ));
                }
            }
            STAGED_WRITES.set(true);
        }
        if s.panic_on == Some(call) {
            panic!("trade listener failed");
        }
    }
}
fn with_mod(test: impl FnOnce()) {
    let mods: Mods = vec![ModEntry::new("trade-observer", Box::new(Mod))].into();
    mod_platform::testing::with_mods(mods, test);
}
fn with_both(test: impl FnOnce()) {
    let mods: Mods = vec![ModEntry::new("trade-and-stack", Box::new(Both))].into();
    mod_platform::testing::with_mods(mods, test);
}

fn open(game: &mut Game<'_>, payment: u32) -> ContainerClick {
    use super::{
        native_merchant_refs::tests::{offer, spawn},
        trading::MerchantState,
    };
    spawn(game, 40, "minecraft:villager");
    game.mobs[0].extra.profession = sunrise_client::mobs::villager::ProfessionState::from_index(5);
    let mut offer = offer("minecraft:wheat", "minecraft:emerald");
    offer.base_cost_a.stack.count = 1;
    offer.uses = 0;
    offer.max_uses = 10;
    offer.demand = 0;
    offer.special_price_diff = 0;
    offer.price_multiplier = 0.0;
    game.mobs[0].extra.trade = Some(Box::new(Trader {
        offers: vec![offer.clone()],
        ..Trader::default()
    }));
    game.mobs[0].extra.flag2 = true;
    let mut session = Session::new(vec![offer], true);
    // Avoid a level-up changing the one-offer test while still awarding real trade XP.
    session.level = 5;
    session.profession = "minecraft:farmer".to_owned();
    game.lan_open_merchant(
        12,
        MerchantState {
            session: Box::new(session),
            scroll: 0,
            dragging: false,
            focused: None,
            mob: Some(40),
            name: None,
        },
    );
    game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.inventory[9] = Some(ItemStack::new("minecraft:wheat", payment));
    });
    game.lan_select_trade(12, 0);
    let window = game.lan.as_ref().unwrap().play[&12]
        .merchant
        .as_ref()
        .unwrap()
        .window;
    ContainerClick {
        container: window,
        state: 0,
        slot: 2,
        button: 0,
        input: 0,
        changed: Vec::new(),
        carried: None,
    }
}
#[test]
#[ignore = "daily"]
fn refusal_restores_crafted_score_and_another_merchants_offers() {
    use mod_api::{ObjectiveRenderType, prelude::ScoreboardServerHost};
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            use super::native_merchant_refs::tests::{offer, spawn};
            spawn(game, 41, "minecraft:villager");
            game.mobs[1].extra.trade = Some(Box::new(Trader {
                offers: vec![offer("minecraft:stick", "minecraft:emerald")],
                ..Trader::default()
            }));
            ScoreboardServerHost::add_objective(
                game,
                "crafted",
                "minecraft.crafted:minecraft.emerald",
                mod_api::Text::literal("Crafted"),
                ObjectiveRenderType::Integer,
            )
            .unwrap();
            let player = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(12)
                .unwrap()
                .name
                .clone();
            script(Script {
                edit_other_merchant_and_board: true,
                panic_on: Some(1),
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            unchanged(game, 2);
            assert_eq!(CALLS.get(), 1);
            assert!(game.score.board.objective("temporary").is_none());
            assert!(game.score.board.score(&player, "crafted").is_none());
            let other = game.mobs[1].extra.trade.as_ref().unwrap();
            assert_eq!(other.offers[0].uses, 1);
        })
    });
}

#[test]
#[ignore = "daily"]
fn an_invalid_offer_restores_another_merchants_effects_and_modifiers() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            super::native_merchant_refs::tests::spawn(game, 41, "minecraft:villager");
            let mob = &mut game.mobs[1];
            mob.add_effect("minecraft:slowness", 300, 0);
            mob.effects_dirty = false;
            let speed = mob.movement_speed().to_bits();
            script(Script {
                invalid: true,
                effect_write: Some(EffectWrite::ReplaceSlowness),
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 1);
            assert!(
                STAGED_WRITES.get(),
                "effect writes must succeed before refusing the trade"
            );
            unchanged(game, 2);
            let other = &game.mobs[1];
            assert_eq!(other.extra.effects.len(), 1);
            assert_eq!(other.extra.effects[0].id, "minecraft:slowness");
            assert_eq!(other.extra.effects[0].duration, 300);
            assert_eq!(other.movement_speed().to_bits(), speed);
            assert_eq!(other.absorption, 0.0);
            assert!(!other.effects_dirty);
            assert!(game.transactions.is_empty());
        });
    });
}

/// Observe the order from gameplay's store and independently through the mod API's live map.
fn speed_modifier_ids(game: &Game<'_>) -> [Vec<String>; 2] {
    let attrs = game.mobs[1].extra.attributes.as_deref().unwrap();
    let legacy = attrs
        .modifiers
        .iter()
        .chain(&attrs.transient)
        .filter(|row| row.attr == Attr::MovementSpeed)
        .map(|row| row.id.to_string())
        .collect();
    let other = game.living(game.entity(41).unwrap()).unwrap();
    let live = other
        .attribute_modifiers(game, SPEED)
        .unwrap()
        .into_iter()
        .map(|row| row.id.to_string())
        .collect();
    [legacy, live]
}

#[test]
#[ignore = "daily"]
fn a_refused_trade_restores_modifier_order_after_reapplying_the_same_effect() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            super::native_merchant_refs::tests::spawn(game, 41, "minecraft:villager");
            let mob = &mut game.mobs[1];
            mob.add_effect("minecraft:speed", 300, 0);
            finalize::add_modifier(
                mob,
                Attr::MovementSpeed,
                "test:survivor",
                0.25,
                Op::AddMultipliedTotal,
            );
            let other = game.living(game.entity(41).unwrap()).unwrap();
            assert!(game.living_set_equipment(
                other,
                mod_api::EquipmentSlot::Head,
                Some(ItemStack::new("minecraft:diamond_helmet", 1)),
            ));
            assert_eq!(other.attribute_value(game, "minecraft:armor"), Some(3.0));
            let expected = ["minecraft:effect.speed", "test:survivor"];
            for ids in speed_modifier_ids(game) {
                assert_eq!(ids, expected);
            }
            script(Script {
                invalid: true,
                effect_write: Some(EffectWrite::ReapplySpeed),
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert!(STAGED_WRITES.get(), "Speed was removed and re-added");
            unchanged(game, 2);
            for ids in speed_modifier_ids(game) {
                assert_eq!(ids, expected);
            }
            assert_eq!(other.attribute_value(game, "minecraft:armor"), Some(3.0));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_refused_trade_restores_modifier_order_after_a_nested_effect_clear() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            super::native_merchant_refs::tests::spawn(game, 41, "minecraft:villager");
            let mob = &mut game.mobs[1];
            mob.add_effect("minecraft:speed", 300, 0);
            mob.add_effect("minecraft:slowness", 400, 0);
            finalize::add_modifier(
                mob,
                Attr::MovementSpeed,
                "test:survivor",
                0.25,
                Op::AddMultipliedTotal,
            );
            let expected = [
                "minecraft:effect.speed",
                "minecraft:effect.slowness",
                "test:survivor",
            ];
            for ids in speed_modifier_ids(game) {
                assert_eq!(ids, expected);
            }
            script(Script {
                invalid: true,
                effect_write: Some(EffectWrite::Clear),
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert!(
                STAGED_WRITES.get(),
                "the nested clear committed inside the refused trade"
            );
            unchanged(game, 2);
            for ids in speed_modifier_ids(game) {
                assert_eq!(ids, expected);
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_refused_trade_restores_a_replaced_transient_in_place() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            super::native_merchant_refs::tests::spawn(game, 41, "minecraft:villager");
            let mob = &mut game.mobs[1];
            finalize::add_transient_modifier(
                mob,
                Attr::MovementSpeed,
                "minecraft:effect.speed",
                0.125,
                Op::AddValue,
            );
            finalize::add_transient_modifier(
                mob,
                Attr::MovementSpeed,
                "test:survivor",
                0.25,
                Op::AddValue,
            );
            for ids in speed_modifier_ids(game) {
                assert_eq!(ids, ["minecraft:effect.speed", "test:survivor"]);
            }
            script(Script {
                invalid: true,
                effect_write: Some(EffectWrite::ReplaceTransient),
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert!(
                STAGED_WRITES.get(),
                "the colliding effect and the explicit write must succeed"
            );
            unchanged(game, 2);
            for ids in speed_modifier_ids(game) {
                assert_eq!(
                    ids,
                    ["minecraft:effect.speed", "test:survivor", "test:explicit"]
                );
            }
            assert_eq!(
                finalize::transient_modifier_amount(
                    &game.mobs[1],
                    Attr::MovementSpeed,
                    "minecraft:effect.speed"
                ),
                Some(0.125)
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn exploration_trade_maps_share_nested_reservations() {
    use super::maps::MapData;
    with_guests(|game| {
        let start = game.peek_map_id();
        let make = || MapData::fresh(0.0, 0.0, 0, "minecraft:overworld");
        for commit in [false, true] {
            let outer = game.transaction_open();
            assert_eq!(game.new_map_id(make()), start);
            let take = game.transaction_open();
            let trade_id = game.peek_map_id();
            assert_eq!(trade_id, start + 1);
            game.register_exploration_maps(vec![(trade_id, [640, 64, -640], 2, String::new())]);
            game.transaction_close(&take, true);
            assert_eq!(game.new_map_id(make()), start + 2);
            assert!(!game.maps.data.contains_key(&trade_id));
            game.transaction_close(&outer, commit);
            assert_eq!(game.peek_map_id(), if commit { start + 3 } else { start });
            if commit {
                assert!(game.maps.data[&trade_id].unlimited);
                assert!(!game.maps.data[&(start + 2)].unlimited);
            } else {
                assert!(!game.maps.data.contains_key(&trade_id));
            }
        }
    });
}

fn record_packets(game: &mut Game<'_>) -> mpsc::Receiver<Out> {
    let (tx, rx) = mpsc::channel();
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .update_guest(12, |guest| {
            guest.sender = PlayerSender::new(tx, Arc::default());
        })
        .unwrap();
    rx
}

fn packets(rx: &mpsc::Receiver<Out>) -> Vec<Clientbound> {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            _ => Vec::new(),
        })
        .map(|body| {
            let mut r = sunrise_protocol::buf::Reader::new(&body);
            Clientbound::decode(r.varint().unwrap(), r.rest()).unwrap()
        })
        .collect()
}

fn corrected(rx: &mpsc::Receiver<Out>, window: i32, payment: u32) {
    let received = packets(rx);
    let [
        Clientbound::MerchantOffers(offers),
        Clientbound::ContainerSetContent(content),
    ] = received.as_slice()
    else {
        panic!("expected just the authoritative offers and contents");
    };
    assert_eq!((offers.container, content.container), (window, window));
    assert_eq!(offers.offers[0].uses, 0);
    assert_eq!(content.items[0].as_ref().unwrap().count as u32, payment);
    assert_eq!(content.items[2].as_ref(), Some(&offers.offers[0].result));
    assert!(content.carried.is_none());
}

fn count(stack: Option<&ItemStack>) -> u32 {
    stack.map_or(0, |s| s.count)
}
fn result(game: &Game<'_>) -> u32 {
    let guest = &game.lan.as_ref().unwrap().hub.guests[0];
    guest
        .inventory
        .iter()
        .flatten()
        .filter(|s| s.item.as_ref() == "minecraft:emerald")
        .map(|s| s.count)
        .sum::<u32>()
        + count(
            guest
                .carried
                .as_ref()
                .filter(|s| s.item.as_ref() == "minecraft:emerald"),
        )
}
fn dirt(game: &Game<'_>) -> u32 {
    count(game.lan.as_ref().unwrap().hub.guests[0].inventory[20].as_ref())
}
fn stat(game: &Game<'_>) -> i32 {
    game.lan.as_ref().unwrap().play[&12]
        .stats
        .values
        .get("minecraft:custom")
        .and_then(|v| v.get("minecraft:traded_with_villager"))
        .copied()
        .unwrap_or(0)
}
fn unchanged(game: &mut Game<'_>, payment: u32) {
    assert_eq!(result(game), 0);
    assert_eq!(dirt(game), 0);
    assert_eq!(stat(game), 0);
    let session = game.lan_trade_session(12).unwrap();
    assert_eq!(session.offers[0].uses, 0);
    assert_eq!(session.xp, 0);
    assert!(game.orbs.is_empty());
    let host = game.lan.as_ref().unwrap();
    let slots = &host.play[&12].merchant.as_ref().unwrap().slots;
    assert_eq!(count(slots[0].as_ref()), payment);
    assert_eq!(slots[2], Some(ItemStack::new("minecraft:emerald", 1)));
}

#[test]
#[ignore = "daily"]
fn accepted_trade_keeps_the_collect_answer() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            let rx = record_packets(game);
            script(Script {
                answer: true,
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 1);
            assert_eq!(super::decided_action::DECIDED.get(), 1);
            assert_eq!(IN_ACTION.with(|a| a.borrow().clone()), [true]);
            assert_eq!(result(game), 1);
            assert_eq!(dirt(game), 0);
            assert_eq!(stat(game), 1);
            let session = game.lan_trade_session(12).unwrap();
            assert_eq!(session.xp, 7);
            let offer = &session.offers[0];
            assert_eq!(
                (
                    offer.uses,
                    offer.max_uses,
                    offer.demand,
                    offer.special_price_diff,
                    offer.xp
                ),
                (6, 19, -7, -4, 11)
            );
            assert_eq!(offer.price_multiplier, 0.25);
            assert_eq!(COMPLETED.get(), 1);
            let received = packets(&rx);
            let [
                Clientbound::MerchantOffers(offers),
                Clientbound::ContainerSetContent(content),
            ] = received.as_slice()
            else {
                panic!("an edited offer must refresh before the new contents");
            };
            assert_eq!(
                (offers.container, content.container),
                (click.container, click.container)
            );
            assert_eq!(offers.offers[0].uses, 6);
            assert_eq!(content.carried.as_ref().unwrap().count, 1);
            assert!(!game.orbs.is_empty());
            assert!(game.transactions.is_empty());
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_panicking_completion_listener_cannot_drop_success_packets() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            let rx = record_packets(game);
            script(Script {
                answer: true,
                panic_on_completion: true,
                ..Script::default()
            });
            let outer = game.transaction_open();
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(COMPLETED.get(), 0);
            assert!(packets(&rx).is_empty());
            game.transaction_close(&outer, true);
            assert_eq!(COMPLETED.get(), 1);
            assert_eq!((result(game), stat(game)), (1, 1));
            assert_eq!(game.lan_trade_session(12).unwrap().offers[0].uses, 6);
            let received = packets(&rx);
            let [
                Clientbound::MerchantOffers(offers),
                Clientbound::ContainerSetContent(content),
            ] = received.as_slice()
            else {
                panic!("expected committed offers and content packets");
            };
            assert_eq!(
                (offers.container, content.container),
                (click.container, click.container)
            );
            assert_eq!(offers.offers[0].uses, 6);
            assert_eq!(content.carried.as_ref().unwrap().count, 1);
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_panicking_trade_listener_undoes_the_take_and_the_next_trade_works() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            script(Script {
                panic_on: Some(1),
                player_write: Some(PlayerWrite::Unrelated),
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 1);
            unchanged(game, 2);
            assert!(game.transactions.is_empty());
            script(Script::default());
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 0, "the panicking listener stays disabled");
            assert_eq!((result(game), stat(game)), (1, 1));
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_second_trade_panic_undoes_the_entire_shift_click() {
    with_mod(|| {
        with_guests(|game| {
            let mut click = open(game, 3);
            click.input = 1;
            script(Script {
                panic_on: Some(2),
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 2);
            assert_eq!(super::decided_action::DECIDED.get(), 2);
            unchanged(game, 3);
            assert!(game.transactions.is_empty());
            script(Script::default());
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 0, "the panicking listener stays disabled");
            assert_eq!((result(game), stat(game)), (3, 3));
        })
    });
}

#[test]
#[ignore = "daily"]
fn an_outer_abort_undoes_an_accepted_trade() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            script(Script::default());
            let rx = record_packets(game);
            let seal = game.transaction_open();
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!((result(game), dirt(game), stat(game)), (1, 0, 1));
            assert!(game.orbs.is_empty(), "orb creation waits for root commit");
            assert_eq!(COMPLETED.get(), 0);
            assert!(packets(&rx).is_empty());
            game.transaction_close(&seal, false);
            assert_eq!(COMPLETED.get(), 0);
            corrected(&rx, click.container, 2);
            unchanged(game, 2);
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_non_finite_collect_answer_refuses_the_whole_take() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            script(Script {
                invalid: true,
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 1);
            unchanged(game, 2);
        })
    });
}

#[test]
#[ignore = "daily"]
fn no_subscriber_uses_the_ordinary_trade_without_a_decision() {
    mod_platform::testing::with_mods(Mods::default(), || {
        with_guests(|game| {
            let mut click = open(game, 3);
            click.input = 1;
            script(Script::default());
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(super::decided_action::DECIDED.get(), 0);
            assert_eq!(CALLS.get(), 0);
            assert_eq!((result(game), stat(game)), (3, 3));
            assert_eq!(game.lan_trade_session(12).unwrap().offers[0].uses, 3);
            assert_eq!(game.lan_trade_session(12).unwrap().xp, 21);
            assert!(game.transactions.is_empty());
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_result_pickup_heard_as_trade_and_stacked_event_is_one_accepted_action() {
    with_both(|| {
        with_guests(|game| {
            let click = open(game, 2);
            script(Script {
                answer: true,
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!((CALLS.get(), STACKED.get()), (1, 1));
            assert_eq!((result(game), dirt(game), stat(game)), (1, 0, 1));
            assert!(game.transactions.is_empty());
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_trade_refused_inside_the_stacked_pickup_undoes_the_whole_take() {
    for (trade_subscribed, entry) in [
        (true, ModEntry::new("trade-and-stack", Box::new(Both))),
        (false, ModEntry::new("stacked-only", Box::new(Stacked))),
    ] {
        mod_platform::testing::with_mods(vec![entry].into(), || {
            with_guests(|game| {
                let click = open(game, 2);
                let rx = record_packets(game);
                if trade_subscribed {
                    script(Script {
                        panic_on: Some(1),
                        ..Script::default()
                    });
                    assert!(game.lan_merchant_click(12, &click));
                    assert_eq!((CALLS.get(), STACKED.get()), (1, 1));
                    unchanged(game, 2);
                    corrected(&rx, click.container, 2);
                }
                // Even without a trade listener, an attempted result replacement or a failed
                // pickup leaves the take untouched and corrects the client's result slot. Run
                // the panicking case last because a panicking callback is retired for the session.
                for script_value in [
                    Script {
                        replace_stacked_result: true,
                        ..Script::default()
                    },
                    Script {
                        refuse_stacked: true,
                        ..Script::default()
                    },
                ] {
                    script(script_value);
                    assert!(game.lan_merchant_click(12, &click));
                    assert_eq!((CALLS.get(), STACKED.get()), (0, 1));
                    unchanged(game, 2);
                    corrected(&rx, click.container, 2);
                }
                assert!(game.transactions.is_empty());
            })
        });
    }
}

#[test]
#[ignore = "daily"]
fn an_invalid_result_stack_refuses_the_whole_take() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            script(Script {
                invalid_stack: true,
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 1);
            unchanged(game, 2);
            assert!(game.transactions.is_empty());
        })
    });
}

#[test]
#[ignore = "daily"]
fn shift_click_reads_each_use_and_applies_the_second_price_before_the_next_trade() {
    with_mod(|| {
        with_guests(|game| {
            let mut click = open(game, 3);
            click.input = 1;
            script(Script {
                raise_second_price: true,
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(USES.with(|uses| uses.borrow().clone()), [1, 2]);
            assert_eq!((result(game), stat(game)), (2, 2));
            let gm = game.lan.as_ref().unwrap().play[&12]
                .merchant
                .as_ref()
                .unwrap();
            assert_eq!(gm.slots[0].as_ref().unwrap().count, 1);
            assert!(gm.slots[2].is_none());
        })
    });
}

#[test]
#[ignore = "daily"]
fn player_writes_refuse_the_frozen_take_without_recomputing() {
    with_mod(|| {
        with_guests(|game| {
            let mut click = open(game, 3);
            let rx = record_packets(game);
            for (input, player_write) in [
                (1, PlayerWrite::ShiftDestination { restore: false }),
                (1, PlayerWrite::ShiftDestination { restore: true }),
                (0, PlayerWrite::Unrelated),
            ] {
                click.input = input;
                script(Script {
                    player_write: Some(player_write),
                    ..Script::default()
                });
                assert!(game.lan_merchant_click(12, &click));
                assert_eq!(CALLS.get(), 1);
                assert!(
                    STAGED_WRITES.get(),
                    "the scripted slot writes must succeed before refusal"
                );
                unchanged(game, 3);
                assert!(game.lan.as_ref().unwrap().hub.guest(12).unwrap().inventory[8].is_none());
                corrected(&rx, click.container, 3);
            }
        })
    });
}

#[test]
#[ignore = "daily"]
fn observing_a_trade_keeps_the_plain_packet_sequence() {
    let trace = |mods: Mods| {
        let mut bodies = None;
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                let click = open(game, 2);
                let rx = record_packets(game);
                script(Script::default());
                assert!(game.lan_merchant_click(12, &click));
                let trace = packets(&rx);
                assert!(matches!(
                    trace.as_slice(),
                    [Clientbound::ContainerSetContent(_)]
                ));
                bodies = Some(trace.into_iter().map(|p| p.encode()).collect::<Vec<_>>());
            })
        });
        bodies.unwrap()
    };
    let observed: Mods = vec![ModEntry::new("trade-observer", Box::new(Mod))].into();
    assert_eq!(trace(Mods::default()), trace(observed));
}
