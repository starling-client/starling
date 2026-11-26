# Registering cauldron interactions

Use `Hooks::register_cauldron_interaction` with a `RegistrationContext` and
`RegisterCauldronInteractionEvent`. Retain typed `BlockKey` and `ItemKey` values
from registration. The hook runs on both sides when preparing the resource
snapshot; publication happens only after the complete candidate validates.
Failed preparation retains the previous snapshot. A
[failed registration callback](../../concepts/load-phases.md#registration-and-setup)
contributes no exact, tag or all-dispatcher registrations. Other callbacks can
still register theirs.

## Register an item

Register an exact item on a block's dispatcher:

```rust,ignore
fn register_cauldron_interaction(
    &self,
    _: &RegistrationContext,
    event: &mut RegisterCauldronInteractionEvent<'_>,
) {
    let handler: Arc<dyn Interaction> = Arc::new(Syrup {
        empty_bottle: self.glass_bottle,
    });
    event
        .interactions
        .register(self.empty_cauldron, self.syrup_bottle, handler.clone())
        .unwrap();
    event
        .interactions
        .register(self.syrup_cauldron, self.syrup_bottle, handler)
        .unwrap();
}
```

A custom block opts into dispatch by registration; its name need not end in
`cauldron`. Register the filled target too when an interaction replaces the
empty block with a custom cauldron. `register_to_all(item, handler)` applies
over the complete prepared dispatcher set, including custom blocks registered
later in the hook sequence, in registration order alongside direct `register`
calls so later writes overwrite earlier entries for that item on each block in
place.

Vanilla entries are prebound in the same table. Registering an item that already
has a vanilla or mod handler overwrites it in place (matching
`CauldronInteraction.Dispatcher.put`). Unknown block or item keys fail admission
with `CauldronRegistrationError::UnknownBlock` or
`CauldronRegistrationError::UnknownItem`; admission errors are recorded even if
a callback ignores the returned `Err`, failing preparation when it finishes and
leaving the published snapshot unchanged.

## Register an item tag

An item tag answers for many items at once (NeoForge's `register(id, TagKey, …)`
and `registerToAll(TagKey, …)`):

```rust,ignore
// One block's dispatcher:
event
    .interactions
    .register_tag(self.water_cauldron, "c:buckets/milk", handler.clone())
    .unwrap();
// Or every dispatcher:
event
    .interactions
    .register_tag_to_all("#c:buckets/milk", handler)
    .unwrap();
```

The tag is checked when the item is used, against the tags loaded then, so a
data pack that adds an item to it covers that item after `/reload`. Tag entries
precede exact item entries (matching vanilla `CauldronInteraction.Dispatcher.get`):

1. On `minecraft:water_cauldron`, vanilla's dye tag
   (`minecraft:cauldron_can_remove_dye`) is checked first unless a mod replaces
   that tag entry via `register_tag`.
2. Registered mod item tags on the block are checked in registration order
   (direct `register_tag` and `register_tag_to_all` calls apply in registration
   order across the complete prepared dispatcher set).
3. Exact item entries on the block are checked last.

Registering the same tag again on one block overwrites its handler in place at
its original position, including replacing vanilla's dye tag handler. A string
that is not a valid resource location fails admission with
`CauldronRegistrationError::InvalidTag`.

## Implement `Interaction::interact`

Implement `Interaction::interact(&self, host: &mut dyn ServerHost, input: &CauldronUse) -> CauldronDecision`.
It runs only on the server and receives a `CauldronUse` capturing the click's
inputs before mutation:

- `input.level`: `LevelRef` for the server level owning the cauldron
- `input.pos`: `[i32; 3]` target block position
- `input.state`: `BlockState` of the target block before the action
- `input.player`: `PlayerRef` for the acting player
- `input.hand`: `Hand` used for the click
- `input.held`: `ItemStack` held before the action
- `input.underwater`: `bool` (`CauldronInteractions.isUnderWater`)
- `input.creative`: `bool` indicating whether creative exchange semantics apply

Perform block, item, sound, game event and statistic updates directly through
`host` and the typed handles on `input`:

```rust,ignore
use mod_api::{
    BlockUpdateFlags, CauldronDecision, CauldronUse, GameEventContext, HostTransaction as _,
    Interaction, ItemKey, ItemPlace, ItemResource, ItemStack, ServerHost, SoundSource, StatType,
};

struct Syrup {
    empty_bottle: ItemKey,
}

impl Interaction for Syrup {
    fn interact(&self, host: &mut dyn ServerHost, input: &CauldronUse) -> CauldronDecision {
        if input.underwater || input.held.count() == 0 {
            return CauldronDecision::Pass;
        }
        let Some(filled) = host.block_state_id("example:syrup_cauldron[level=1]") else {
            return CauldronDecision::Pass;
        };
        let bottle = ItemStack::new(self.empty_bottle, 1).unwrap();
        let hand = ItemPlace::PlayerHand {
            player: input.player.uuid(),
            hand: input.hand,
        };
        let replacement = ItemResource::of(&bottle);
        let applied = host.transaction(|host, tx| {
            input
                .level
                .set_block(
                    host,
                    input.pos,
                    filled,
                    BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS,
                )
                .map_err(|_| ())?;
            if hand.exchange(host, tx, &replacement, 1) != Ok(1) {
                return Err(());
            }
            Ok(())
        });
        if applied.is_err() {
            return CauldronDecision::Consume;
        }
        input
            .player
            .award_stat(host, StatType::Custom, "minecraft:use_cauldron", 1);
        input
            .player
            .award_stat(host, StatType::Used, "example:syrup_bottle", 1);
        let at = input.pos.map(|v| f64::from(v) + 0.5);
        input.level.play_sound(
            host,
            "minecraft:item.bottle.empty",
            at,
            SoundSource::Blocks,
            1.0,
            1.0,
            None,
        );
        input.level.game_event(
            host,
            "minecraft:fluid_place",
            at,
            GameEventContext::default(),
        );
        CauldronDecision::Success
    }
}
```

### Outcome and arm swing

The returned `CauldronDecision` maps directly to vanilla's `InteractionResult`:

- `CauldronDecision::Success`: consumes the click and swings the player's arm
  (`InteractionResult.SUCCESS`).
- `CauldronDecision::Consume`: stops normal item use without swinging
  (`InteractionResult.CONSUME`).
- `CauldronDecision::Pass`: leaves normal item use available so the click falls
  through (`InteractionResult.TRY_WITH_EMPTY_HAND`).

### Writes and transactions

Writes apply directly to the world, or join an already-open caller transaction
if one exists. The engine does not wrap every cauldron interaction in an
implicit transaction. Open `host.transaction(|host, tx| …)` yourself when
grouping storage mutations (such as `ItemPlace::exchange` with
`LevelRef::set_block`) so they commit or roll back together.

### Failures

- A panic in `Interaction::interact` disables that mod's cauldron callback for
  the rest of the session (surviving `/reload`), logs the error and surfaces it
  through the client error toast naming the mod without crashing unrelated
  ticks. Subsequent clicks on that entry return `Consume` (without swinging)
  and do not re-invoke the disabled callback.
- Input construction and phase refusals are reported each time they occur,
  return `Consume` without swinging, and are never latched.

## Client prediction

Sunrise clients use the server's published registration keys to identify a
custom cauldron click, even when the handler is installed only on the server.
They do not invoke `Interaction::interact`, predict its writes, or send a
speculative item-use packet, even if the client's block-use phase was skipped.
A terminal client hook answer ends the local click as usual. Otherwise, if the
server's block phase passes or is skipped, the client runs its eligible
`ItemAfterBlock` hook and item-on-block behaviour once, without sending the
block-use packet again. This local phase still runs if the server's item-on-block
phase consumes the click. Only a pass on both sides allows the same hand's air
use and then, if that passes, the other hand's turn.

The original swing policy applies. Changing the selected slot, held stacks,
eye position, aim direction or reach before the result supersedes the pending
item turn. For example, walking without turning cannot redirect an empty
bucket's pending use.

`client_packet_send` still sees `minecraft:use_item_on` for the block-use action.
Canceling that packet stops both the action and its continuation. If a packet
hold drops the action, no continuation waits for a reply. Vanilla handlers
continue to predict locally. Clients that do not negotiate these results still
use vanilla packets, which do not identify which item-use packet belongs to a
block click.
