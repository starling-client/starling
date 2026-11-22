# Item and tag parts

For an item-specific Part, request its canonical registry key once in `register`:
`let stick = r.item_key("minecraft:stick"); r.item_part(stick, Sharp);`. Bare names use the
`minecraft:` namespace. Requests may refer to content declared later; the final item registry refuses
unknown IDs with the requesting mod. Compare `ItemKey` values directly when you have both keys.
When you need text, `key.id(host)` borrows its canonical name as `Option<&str>`, so you can
compare with `Some("minecraft:stick")` without allocating. Item capability registration and item
helpers use strings.

`r.block_part_tag("minecraft:logs", Rotates)` and `r.item_part_tag("minecraft:axes", Sharp)` attach a part to
every member of a tag (its name without `#`). Each game resolves them against its own tags when its world
opens, again on `/reload`, and when a client joins a server, so the window's game and its server thread each
answer from the tags they play with; a block that leaves the tag is no longer asked. A mod's parts on one
thing run in the order it registered them, by key or by tag. A part a tag gives a thing
that already has that kind of part warns, naming both mods, and the earlier mod's part stays. A tag that
doesn't exist warns. Opening a world fails, naming the mod, when a part is attached to a block or item the game
doesn't have. Scoped hooks run on a game's thread; one dispatched from any other thread (which no game hook
does) can't see tag parts, and warns once.

## Right click on a block

`Hooks::right_click_block` corresponds to NeoForge 26.3's
`PlayerInteractEvent.RightClickBlock`, on the clicking client and server:

- `use_block = TriState::True` allows block use even while sneaking with an item
  in either hand. `Default` skips while sneaking with either hand full; `False` skips block use.
- `use_item = TriState::True` enters the item's `useOn` phase even with an empty
  hand or an item on cooldown. `Default` requires a held stack off cooldown;
  `False` skips this phase. Adventure placement restrictions apply.

A consuming block result ends the click before the item phase. For forced empty-hand
use, `use_item_on_block` fires with `phase = UsePhase::ItemAfterBlock` and
`click.stack = None`; a listener can consume that use. Without a listener answer,
there is no item Part or vanilla item behaviour to run and the empty use passes.
Canceling `right_click_block` skips both uses and substitutes `cancellation_result`.

## Instant item use

`ItemPart::use_item` receives `UseItemEvent { stack, hand, player, player_entity_id }`.
`hand: Hand` is the interaction hand of this click (`Hand::Main` or `Hand::Off`),
including air clicks with identical stacks in both hands. Read it directly when
choosing which held stack to shrink. `player` and `player_entity_id` name who clicked,
as `UseOnEvent` does: on a server the acting player's UUID and entity id (a guest's
own, never the host's), on a client this client's own player. They come with the
event, so a Part does not need `players().acting_player()` to be set while it runs.
`living_used_item_hand` answers only during ordinary active item use; it does not identify an
instantaneous click or intercepted input. The Part runs on both sides with `HostSide`,
after the global right-click event and before the item's own use.
Return `Some(InteractionResult::Success)` to consume and swing on the client,
`Some(SuccessServer)` to consume and swing on the server, or `Some(Consume)`
to consume without a swing. `Some(Fail)` skips this item's own use without
swinging and allows the off-hand sequence to proceed; unlike a block-use Fail,
it does not end the entire click.
`None` (the default) lets later Parts answer; if none answers, the item's own use
runs. `Some(Pass)` stops dispatch to later Parts and continues directly to the
item's own use.
`TryWithEmptyHand` belongs to block use and is treated as Pass here.
A thrown item can return Success on the client and Consume on the server.
The hand field corresponds to NeoForge's
`PlayerInteractEvent.RightClickItem.getHand()`.

`Hooks::right_click_item` runs before `Item.use`, including after the same hand's
unhandled block or entity interaction. A block success or FAIL, or an entity success, stops fallback.
The server fires it for the separate use-item packet, once for that packet's hand.
Canceling skips the item Part and vanilla action.
`Success`, `SuccessServer` and `Consume` end the client's hand loop.
`Success` swings on the client, `SuccessServer` on the server, and `Consume` has no swing.

## Intercept main-hand client input

Use `ClientItemUseBehavior` for local input, for example to open a tool's client UI rather
than use the block under the crosshair. In `Hooks::register`, request the item's key and bind
one behavior to it. The item can be yours, vanilla's or another mod's. An unknown item or a
second registration for the same item refuses startup; there is no last-writer-wins rule.

This example traces the input for an existing `mymod:tool` without doing anything in the world:

```rust
use std::sync::Arc;

use mod_api::{
    ClientHost, ClientItemUseBehavior, Hooks, ItemStack, JavaString, Registrar, RegistrationContext,
};

struct ToolMod;
struct Behavior;

#[mod_api::sunrise_mod]
impl Hooks for ToolMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.item_key("mymod:tool");
        r.register_client_item_use(key, Arc::new(Behavior));
    }
}

impl ClientItemUseBehavior for Behavior {
    fn use_item(&self, _host: &mut dyn ClientHost, stack: &ItemStack, slot: u32) {
        let data = stack
            .components()
            .and_then(|c| c.as_nbt().get(&JavaString::from("minecraft:custom_data")));
        eprintln!(
            "Tool input started in slot {slot}: {} item(s), custom data present: {}",
            stack.count(),
            data.is_some(),
        );
    }

    fn release_using(
        &self,
        _host: &mut dyn ClientHost,
        _stack: &ItemStack,
        slot: u32,
        ticks_held: u32,
    ) {
        eprintln!("Tool input released in slot {slot} after {ticks_held} ticks");
    }

    fn on_stop_using(
        &self,
        _host: &mut dyn ClientHost,
        _stack: &ItemStack,
        slot: u32,
        _ticks_held: u32,
    ) {
        eprintln!("Tool input ended in slot {slot}");
    }
}
```

An enabled owner consumes the input before crosshair handling. This applies to main-hand
input only. The callbacks send neither an item use nor a release packet and do not make the
player use or block with the item. For ordinary behavior on both sides, use
`ItemPart::use_item` and `ItemPart::release_using` instead. To track this local interception,
use the callbacks, not `local_item_use`, `living_used_item_hand` or `living_is_blocking`.

Each callback sees a typed `&ItemStack` and its hotbar slot (`u32`, 0–8). While the same item
kind remains in that slot, each tick and release rebinds the stack to the live slot contents
(matching vanilla `LivingEntity.updatingUsingItem` and `LivingEntity.releaseUsingItem`), so
`release_using` and `on_stop_using` observe live count and component updates. If the slot is
emptied or replaced with a different item kind, `on_stop_using` receives the last valid stack
of the captured item kind. `stack.item()` gives the `ItemKey` and `stack.count()` gives the
count. Read `minecraft:custom_name` or `minecraft:custom_data` as typed `Tag` entries in the
component patch, as above, or decode a component with `stack.get(&descriptor)`.

Both optional end methods also receive the elapsed held ticks. The game clears the capture
before calling either. Releasing the use input—including when a keybind opens a screen and
releases key mappings (matching `Minecraft.handleKeybinds`) or when focus is lost and the next
tick observes the use key up—while the same item kind is still selected in the active slot and
the player can still use it calls `release_using` followed by `on_stop_using`. A reset, such as
a slot or item kind change, death or disconnect, only calls `on_stop_using`. Put actions that
require a release in `release_using` and cleanup that should also run on resets in
`on_stop_using`. A disabled owner receives no callbacks.

## Item use on a block

`Hooks::use_item_on_block` (`UseItemOnBlockEvent`) runs with
`HostSide` before each selected phase: `ItemBeforeBlock`, `Block`, then
`ItemAfterBlock`. `event.click` describes the hit, hand, held stack and player.
Return `Flow::Cancel` to skip the selected phase's Part and vanilla behaviour,
using `*event.cancellation_result` (initially `TryWithEmptyHand`) as its result.

Canceling `ItemBeforeBlock` skips the item's `on_item_use_first` Part. `Pass`
continues to the block phase; any non-`Pass` result ends the block interaction. Cancellation alone does not suppress the later phases when its answer is `Pass`. A block-phase `TryWithEmptyHand`
can run the block's empty-hand behaviour on the main hand.

`ItemPart::use_on`, `Hooks::entity_interact` and `ItemPart::item_interact_mob`
use version 2, run on both logical sides, and expose native `HostSide`.
Their mutable `result` is `Option<InteractionResult>`: `None` defers to later
listeners and the ordinary behaviour; `Some(result)` supplies the replacement
answer. `item_interact_mob` also carries the interaction `hand`.
`ItemPart::use_item` keeps its existing both-side, version-2 contract.

Client execution predicts hands only. Client hosts allow reads and local effects,
not authoritative world, inventory or entity writes; perform those on the server.
Answer on the client as well when prediction must stop the hand loop: an answer
made only on the server cannot suppress an offhand attempt already predicted by
the client.

A client without this mod usually predicts vanilla here; the server corrects it.
For a [server-registered cauldron click](../../how-to/world/cauldron-interactions.md#client-prediction),
the client waits for the server's block phase to pass or be skipped before running
its own `ItemAfterBlock`. The server's later item result does not decide whether
that local phase runs, but it can stop air use and the other hand's turn.

`Success`, `SuccessServer` and `Consume` consume the interaction and end the
client's hand loop. Only `Success` swings on the client; `SuccessServer` swings
on the server and `Consume` never swings. `Fail` ends block interaction only,
not an entity or air-use hand loop. `Pass` and `TryWithEmptyHand` fall through
(the latter can select the block's empty-hand behaviour at the block phase).

## Item wear

`ItemPart::damage_item` is `IItemExtension.damageItem`: the wear a stack of the part's item is about to take
(`ItemStack.hurtAndBreak`), after Unbreaking, as NeoForge passes it. The part reads `event.amount`, the points
about to be lost, and changes it (0 or less leaves the item whole) or returns `Flow::Cancel` for no wear at all.
`event.entity` is who wears or uses the item (`None` for a dispenser's tool), and setting `event.break_now` breaks
the item through the vanilla break whatever durability it has left, as NeoForge's `onBroken` does. It
runs on every wear path: a player's tools, armour, shield and mace, a tool a dispenser uses, and a mob's armour. It is
not asked when Unbreaking already saved the item or for a creative player's tools, and an item with no part costs
one table lookup. With no part, the vanilla amount applies.

To change wear without attaching a Part to an item, use an enchantment's
`minecraft:item_damage` effect (the component Unbreaking uses).
A part does not get the wearing entity;
`host.players().acting_player()` names the player on the paths a player takes.

## Mending repair

`ItemPart::xp_repair_ratio` is `IItemExtension.getXpRepairRatio`: the durability points Mending restores to a
stack of the part's item per point of experience (`ExperienceOrb.repairPlayerItems`), asked on the server once for
each item it repairs, from an orb or from `PlayerRef::give_experience_points`. `event.ratio` arrives as vanilla's:
the item's `minecraft:repair_with_xp` repair divided by `event.experience`, 2.0 for vanilla Mending. The repair
becomes `floor(experience × ratio)`, capped at the item's damage, and the experience it doesn't use goes on to the
next damaged Mending item, then to the player. A ratio left as it arrived keeps vanilla's repair exactly. One that
isn't finite is refused (vanilla's repair is used, with one warning per item), and a negative one counts as 0; a
ratio that takes a positive repair to 0 leaves the experience for the player. The `Host` is read-only.

To change Mending for every item at once,
change the enchantment's `minecraft:repair_with_xp` effect in a data pack; the ratio a part gets arrives with it. To
change it for particular items, vanilla's or another mod's included, attach a part to their item keys.

## Enchanting and pick block

Four methods use a read-only `Host`. `supports_enchantment` runs on both sides and returns an
`Option<bool>`; the other three run on the server and edit an answer in their event.

- `ItemPart::supports_enchantment` is `IItemExtension.supportsEnchantment`: whether the item may receive
  `event.enchantment`. The first `Some(bool)` replaces support; `None` tries the next part and then keeps the
  caller's vanilla rule. Both anvil calculations accept enchanted books by default; `/enchant` and
  compatibility-filtered `enchant_randomly` use `supported_items` membership instead. Creative anvil use still
  bypasses support; enchantment conflicts remain separate.
- `ItemPart::is_primary_item_for` is `IItemExtension.isPrimaryItemFor`: whether selection may offer the pair
  in table offers/clues, loot `enchant_with_levels`, trades and spawn gear. Its default is true for a plain
  book; otherwise it reads support before checking `primary_items` when that set exists. A primary listener
  may change that answer, including withholding an enchantment from a plain book. This is version 2.

- `ItemPart::apply_enchantments` is `IItemExtension.applyEnchantments`: the enchanting table's result for the
  input stack and the list it chose (`event.enchantments`). `event.result` arrives as vanilla's (a book becomes
  an enchanted book, then each level is added). The input slot holds what the part leaves, and
  `player_enchant_item`, the `enchanted_item` trigger and `enchant_item` see it; the levels and lapis paid don't
  change. An invalid stack, or a count other than the input's, is refused, and vanilla's result is kept.
- `ItemPart::is_not_replaceable_by_pick_action` is `IItemExtension.isNotReplaceableByPickAction`: when a pick
  moves an item into a full hotbar, whether the stack in `event.slot` must stay. `event.not_replaceable` arrives
  as vanilla's (the stack is enchanted). The hotbar is asked from the selected slot on until a stack answers
  `false`, and the pick uses that slot; with every stack kept, it uses the selected one.

`supports_enchantment` and `is_primary_item_for` are **install-time snapshots**, not component-dependent
held-stack queries. Each item's parts are asked once per registered enchantment after tags and parts are
installed (server world open or `/reload`, or client tag packets), with the item's default stack. The
engines read the stored answers until the next install or world close. Support resolves before the server
resolves primary answers. Each snapshot owns its answers; publishing a fresh world-loot snapshot copies
the current answers without asking listeners again. The client `Host::enchantment_supports` read is separate:
it checks `supported_items` membership only.

## Hand, break and name on the client

Three client-side methods change how the local player's client shows and handles a held stack. Each `event` field
arrives holding vanilla's answer, which the part may set either way, and an item with no such part costs one table
lookup.

- `ItemPart::should_cause_reequip_animation` is `IItemExtension.shouldCauseReequipAnimation`: whether the
  first-person hand lowers and rises again to show `event.stack`, the stack the hand holds now, in place of
  `event.old`, the stack it shows. `event.slot_changed` says the stack came from another slot (always false for the
  off hand). `event.reequip` starts `false` for the same stack changed in place or an equal one but for its damage.
  It is asked each client tick, for each hand, while the shown and held stacks differ; an empty hand isn't asked.
- `ItemPart::should_cause_block_break_reset` is `IItemExtension.shouldCauseBlockBreakReset`: whether a break
  restarts because the main hand changed. `event.stack` is the stack the break began with and `event.held` the main
  hand now. `event.reset` starts `true` when they aren't the same item with the same components (count left out, damage
  compared). `false` keeps the break and its progress; `true` sends the abort and a new start, from 0. It is asked on
  each tick of a break of the same block, in single player and on a server's client. A `true` for an unchanged stack
  restarts the break every tick, so it never finishes.
- `ItemPart::highlight_tip` is `IItemExtension.getHighlightTip`: the name drawn above the hotbar for the selected
  stack. `event.name` arrives as vanilla draws it (the hover name in its rarity colour, italic when renamed), and the
  text the part leaves is drawn at the same place and fade. It is asked once a client tick while the name shows. The
  name's timer still restarts only when the item or its hover name changes, so a tip that changes every tick doesn't
  keep it up.
- `ItemPart::item_name` is vanilla's `Item.getName(ItemStack)`: the name of `event.stack`, from its components, so
  one item names each stack (a data model after the mob in it, a filled tank after its fluid). `event.name` arrives
  as `None`, the default name: the `minecraft:item_name` component, else the item's translated name. Set it to
  `Some(text)`, a translatable `Text` (`Text::translate`) so each viewer reads it in their language. It replaces the
  default, as the vanilla items that override `getName` do, and a `minecraft:custom_name` or a written book's title
  still wins over it (`ItemStack.getHoverName`); the part isn't asked then. Every place that names a stack reads it,
  once per naming: tooltips, the held item's name above the hotbar, command feedback such as `/give`'s, the anvil's
  rename box and its comparison on both sides, stats, the recipe book's and the creative inventory's search, recipe
  screens and `Host::hover_name`. It takes the loop `context` and no `Host`, so it can't name a stack again: answer
  from the stack alone, and fast.

```rust,ignore
struct DataModel;

#[mod_api::sunrise_part]
impl ItemPart for DataModel {
    fn item_name(&self, _context: &dyn LoopContext, event: &mut ItemNameEvent<'_>) {
        // The mob in the stack's own component names it: "Zombie Data Model".
        let id = ComponentId::new("mymod:data_model").expect("a component id");
        if let Some(Tag::String(mob)) = ItemResource::of(event.stack).component(&id) {
            let mob = Text::translate(format!("entity.{}", mob.to_string_lossy().replace(':', ".")), Vec::new());
            *event.name = Some(Text::translate("item.mymod.data_model.named", vec![mob]));
        }
    }
}
```

## Grindstone, repair combining and container items

Three methods answer item questions that vanilla asks inside its menus and recipe matching. They run on both
sides, the server's menus and the client's own result display, so they take the loop `context` and no `Host`:
answer from `event.stack` and `context` alone, and fast. Each `event` field arrives holding vanilla's answer, which
the part may set either way, and an item with no such part costs one table lookup.

A client without this mod predicts vanilla here; the server corrects it.

- `ItemPart::can_grindstone_repair` is `IItemExtension.canGrindstoneRepair`. `event.can_repair` starts with
  whether the stack is damageable. It is asked for the top input when both grindstone inputs are single stacks of
  the same item. `true` gives one item with the enchantments of both merged and only the curses kept (durability is
  merged only when the item has some); `false` gives two of an item that stacks, when the inputs match, and nothing
  otherwise. The input slots ask it too: a granted stack can be placed. A single enchanted input is never asked.
- `ItemPart::is_combine_repairable` is `IItemExtension.isCombineRepairable`. `event.repairable` starts with
  whether the stack has durability. Two single stacks of the same item in a crafting grid repair each other when
  both answer `true`. A `true` for a stack with no maximum damage is refused, with one warning per item, because the
  result needs one. The anvil keeps vanilla's test.
- `ItemPart::can_fit_inside_container_items` is `IItemExtension.canFitInsideContainerItems`. `event.fits` starts
  `true` for everything but a shulker box. It is asked by a shulker box's slots, a bundle (clicks and the `/item`
  contents) and the item slots a mod gives an item. A hopper filling a shulker box doesn't ask. A `true` for a
  shulker box lets shulker boxes nest: that is the mod's choice, bounded by the NBT readers' depth limit.

When the grindstone inputs change, `grindstone_place` runs inline and lends the top and bottom
stacks and vanilla's result through `event.inputs` (matching NeoForge
`CommonHooks.onGrindstoneChange`). Cancellation is checked first and clears the result slot
without rolling back listener writes. Otherwise leave `event.edits.output` at `None` to keep
vanilla's result and award, or set `Some(stack)` with a valid stack to replace the result. Only
a replacement uses `event.edits.xp`: any negative value falls through to vanilla experience, and
nonnegative values set the award. Only the replacement stack is checked as a write; the borrowed
stacks are not. An invalid replacement is logged and ignored, keeping vanilla's result.

`grindstone_take` runs inline after the click takes the result and lends the pre-take stacks as
`event.inputs.top` and `event.inputs.bottom` (matching NeoForge `CommonHooks.onGrindstoneTake`).
These optional references do not change when listeners edit the answer. `event.edits.xp` starts
at the resolved experience (`>= 0`, from `grindstone_place` or vanilla's pre-rolled award);
non-positive values (`<= 0`) spawn 0 orbs while still firing level event 1042 and replacing the
inputs. The two input writes and the final input-container notification each run
`grindstone_place` before the click drops the taken stack or checks for a repeat. Set `event.edits.new_top` or `event.edits.new_bottom` to leave a stack in that slot;
both start at `None` (empty the slot), and an invalid remainder is logged and ignored as an
empty slot.

Cancelling `grindstone_take` is checked before converting remainders: it keeps the taken result
and any listener writes while skipping experience, level event 1042 and input replacement. A
panicking listener is caught so it cannot unwind across the turn and leaves vanilla/initial
behavior.

After the anvil calculates a result with a left input, `anvil_update` fires live on both
sides for each result calculation (with no answer cache), including a take consuming material
while its left input is still present. On the server the take has already charged the player
at that point. The hook lends `event.inputs`: the left and optional right stacks,
the optional rename text and vanilla's output and costs. Read those values without cloning;
they don't change as listeners edit `event.edits`. Leave `edits.output` at
`StackResult::Keep` to use vanilla's output, use `StackResult::Set(Some(stack))` to replace
it, or use `StackResult::Set(None)` to clear it. To read the current answer, match on
`&event.edits.output`: use `event.inputs.vanilla_output` for `Keep` and `stack.as_ref()` for
`Set(stack)`. The level and material costs start at vanilla's values (`xp_cost` stays signed
across listeners and clamps to `0` when applied, matching NeoForge `AnvilMenu.setCost`).
Changing only the costs doesn't revalidate vanilla's stack. Cancellation clears the output
and both costs; an invalid replacement output is logged and ignored, keeping vanilla's
output while still applying the cost edits.

The server's `anvil_craft_pre` and `anvil_craft_post` both borrow the same pre-take snapshot
through `event.inputs` (`AnvilCraftInputs<'_>`). Read the optional stacks through `inputs.left`
and `inputs.right`, and the taken result through `inputs.output`. Set `event.cancelled` or
return `Flow::Cancel` in Pre to return early before charging levels, consuming inputs,
applying anvil wear and firing Post, without undoing the click's item move (matching
NeoForge `AnvilMenu.onTake`). Post runs after those effects.

These input views are valid only during the callback. Clone an individual stack only if you
need to keep it afterwards. The input views and staged edits are not Wire records.

`MenuPart::menu_slot_rules` answers per menu type and slot, including for a replacement from
`item_stacked_on_other`.

## Stacking onto another item in a menu

`ItemPart::item_stacked_on_other` shares the `item_stacked_on_other` hook, event and
decide/fire/apply transaction with `Hooks::item_stacked_on_other` (NeoForge's
`ItemStackedOnOtherEvent`), covering vanilla 26.3's `Item.overrideStackedOnOther` and
`Item.overrideOtherStackedOnMe` on both sides:

- **Server (authoritative)**: single-player integrated server, LAN host and dedicated server
  receive `HostSide::Server`.
- **Client**: ordinary container/inventory click prediction and creative Survival Inventory tab or
  Item Tab hotbar pickups receive `HostSide::Client`. During client prediction, server
  reconciliation remains authoritative. Creative catalogue display slots are not pickup overrides.

Dispatch runs at the resolved pickup moment: both a direct left/right `Pickup` when no drag is
active and a one-slot left/right `QuickCraft` drag release (matching vanilla
`AbstractContainerMenu.doClick`). Multi-slot drags, middle-button drags and direct pickups that
only reset an active drag do not dispatch.

On each resolved pickup, dispatch follows vanilla
`AbstractContainerMenu.tryItemClickBehaviourOverride` order inside one transaction:

1. Global `Hooks::item_stacked_on_other` listeners (`event.event.direction == StackedOnOtherDirection::Event`),
   authoritative server side only.
2. The carried (cursor) stack's `ItemPart::item_stacked_on_other` (`StackedOnOtherDirection::CarriedOnOther`).
3. Vanilla's carried override (`BundleItem.overrideStackedOnOther`, dispatched by item type rather
   than mere `bundle_contents` component presence), using its real outcome to decide whether to
   stop before step 4.
4. The clicked slot stack's `ItemPart::item_stacked_on_other` (`StackedOnOtherDirection::OtherStackedOnMe`).
5. Vanilla's clicked override (`BundleItem.overrideOtherStackedOnMe`), then the plain pickup click.

`ItemPart` overrides represent both vanilla outcomes inside the transaction:

- **Handled (stop)**: set `event.result.handled = true`, set `event.result.carried` via
  `StackResult::Set(...)` and/or `event.result.clicked` via `ClickedStackResult::Set(...)` or
  `ClickedStackResult::Take(count)`, and return `Flow::Cancel`. Stops later overrides and plain
  pickup.
- **Not handled (continue, optionally with edits)**: leave `event.result.handled = false`,
  optionally edit `event.result.carried` and/or `event.result.clicked`, and return
  `Flow::Continue`. Later overrides and plain pickup observe the edited stacks.

For `event.result.clicked`, direct writes and slot takes are distinct:

- `ClickedStackResult::Set(stack)` writes the clicked slot directly (`Slot.set`), without running
  `Slot.onTake` side effects — clearing an occupied furnace or crafting result slot with
  `ClickedStackResult::Set(None)` produces no smelting experience, leaves `recipes_used` untouched,
  and fires no take hooks.
- `ClickedStackResult::Take(count)` takes `count` items from the clicked slot via `Slot.safeTake` /
  `Slot.onTake` (spawning furnace XP, clearing `recipes_used`, consuming crafting inputs, and
  firing take hooks such as `item_smelted` and `furnace_extract`). `ClickedStackResult::Take` is
  refused for global listeners.

Contradictory `ItemPart` answers (`Flow::Cancel` with `handled = false`, or `Flow::Continue` with
`handled = true`), writing through the host to what the click read, returning an invalid stack,
failing slot placement/pickup rules, or panicking refuses the click and rolls back every write.
Global `Hooks::item_stacked_on_other` listeners preserve their separate contract (stacks may only
be set on `Flow::Cancel` + `handled = true`, result/potion slots cannot be edited directly, and
`Flow::Cancel` with `handled = false` skips vanilla bundle overrides and runs plain pickup).
