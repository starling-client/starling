# Custom consume effects

In `Hooks::register`, call `register_consume_effect_type` on the engine's
`Registrar`. If you implement `Registrar` yourself, also implement
`ConsumeEffectsRegistrar`. Use your package ID as the type's namespace;
this is how the game finds your `consume_effect` listener. The two functions
in `ConsumeEffectType::new` read and write the
saved map; the game handles the `type` field. Use the full component keys
`minecraft:consumable` and `minecraft:death_protection` in raw stack patches,
including in child stacks. Bare keys are not admitted as overrides of these
components. `load` checks effects on a new stack or in a decoded child such as
a bundle entry before a write is applied. `minecraft:use_remainder` and
`minecraft:sulfur_cube_content` keep their saved templates and check effects
when the child is materialized. `save` supplies the canonical fields with
exact NBT types, including for values received over the network.

Derive `Wire` on the value itself. Its bytes follow the server's registry ID,
just as a built-in effect's stream fields do. Install the registering mod on
both sides: normal configuration sync resolves the server's IDs by key and
checks the Wire schemas. Switching off a live effect listener does not remove
its registered codec from the connection. Return `Err` for invalid data: an
error refuses that value without disabling the codec. A panicking codec
callback is reported with the mod's name in the log and an error toast, and
is disabled for the session. When a codec fails while reopening an existing
save, the item and its saved data are kept but the failed effect does not run.
A save failure on network receipt leaves out that effect, not the rest of the
stack or the other slots in the update. The client hashes the admitted effects
for menu prediction.

```rust
use mod_api::{Compound, ConsumeEffectType, Identifier, Registrar, Tag, Wire};

#[derive(Wire)]
struct Charge { points: u16 }

fn register(r: &mut dyn Registrar) -> Result<(), String> {
    r.register_consume_effect_type(
        Identifier::parse("example:charge").map_err(|e| e.to_string())?,
        ConsumeEffectType::new::<Charge>(
            |fields| match fields.get(&"points".into()) {
                Some(Tag::Int(n)) => u16::try_from(*n)
                    .map(|points| Charge { points })
                    .map_err(|e| e.to_string()),
                _ => Err("expected integer points".into()),
            },
            |effect| Ok(Compound::from([("points".into(), Tag::Int(effect.points.into()))])),
        ),
    )
}
```

An entry then reads `{type: "example:charge", points: 10}` in SNBT, in either
`on_consume_effects` or `death_effects`. The example accepts an NBT `Int`;
accept other numeric tag types in `load` if your data format calls for them.
Implement `Hooks::consume_effect` to handle a consumable's `on_consume_effects`
entry whose type is in your mod's namespace. Only that namespace's enabled
subscriber receives it. The hook runs on the server thread for the consuming
player or mob, including potions, ominous bottles, and witch and trader drinks.
Entries run in the stack's effective consumable list order, interleaved with
built-in effects; stack patches can change or remove the list.

- The event provides the effect kind and its definition payload as a borrowed `Tag`.
- `event.stack` is a read-only snapshot before the stack shrinks, components included.
- `event.user` is the original consuming entity, not necessarily the acting player. The
  handle can be stale before a world is published or if the entity was removed during an
  earlier effect.
- Replacing the stack during an effect does not change what later entries see. The finish
  does not shrink a replacement or generate a remainder from the invalidated use.
- You can apply effects, play sounds, teleport entities and drop items through the host.
- An unclaimed kind does nothing.
- Consumable and death protection payloads are decoded once for item definitions
  and stack component patches. The event borrows the retained payload. Player death
  protection uses this hook; mob death protection does not dispatch it.
- An invalid registered payload on a decoded stack refuses a new stack or edit.
  In a deferred template, the codec reports the refusal when the child is
  materialized. The refused effect does not run, and the other effects remain
  usable. Reopening a save likewise retains refused data without running it.

To apply an effect without consuming an item, use
[`LivingRef::apply_consume_effect`](consumable-effects.md).

## Food eligibility

`Hooks::can_eat` receives the eating `player` on both sides, with `stack = None`
for cake and candle cake. Read food through `event.player.food(host)` and effects
through `host.living(event.player.entity())`. On the client these are the local
player's copies of server-sent state. Return `Some(true)` to permit eating,
`Some(false)` to refuse it, or `None` to keep `vanilla_can_eat`. Always-eat
eligibility comes from the food component, not a separate event field.
