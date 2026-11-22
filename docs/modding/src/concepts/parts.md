# Parts

Use a part to attach behavior to a specific registry type rather than listen to every event of that
kind. You attach parts once, at startup, in `Hooks::register`. For example, this `BlockPart` gives
`minecraft:stone` a comparator output of 15:

```rust,ignore
use mod_api::{
    AnalogOutputEvent, BlockPart, HasAnalogOutputEvent, Hooks, LevelHost, LevelRandomHost, Registrar,
    RegistrationContext,
};

#[derive(Default)]
struct MyMod;
struct Gauge;

#[mod_api::sunrise_part]
impl BlockPart for Gauge {
    fn has_analog_output(&self, _: &dyn LevelHost, event: &mut HasAnalogOutputEvent) {
        event.output = true;
    }
    fn analog_output(&self, _: &mut dyn LevelRandomHost, event: &mut AnalogOutputEvent) {
        event.output = 15;
    }
}

#[mod_api::sunrise_mod]
impl Hooks for MyMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let stone = r.block_key("minecraft:stone");
        r.block_part(stone, Gauge);
    }
}
```

## Part types

| Trait | Target |
|---|---|
| `BlockPart` | A block type, your own or vanilla's |
| `BlockEntityPart` | A block entity type, with block behavior through `BlockPart` |
| `ItemPart` | An item type, your own or vanilla's |
| `MenuPart` | A menu type: a mod's, or one of vanilla's that a mod block opens |
| `FluidPart` | A mod world fluid type |
| `EntityPart` | An entity type, your own or vanilla's; registered through `Registrar::entity_part` |

- Annotate each part trait implementation with `#[mod_api::sunrise_part]`. For scoped hooks, the
  attribute subscribes the part only to the methods you implement.
- `BlockEntityPart` extends `BlockPart`: provide a separate `BlockPart` implementation and annotate
  both implementations.
- Parts implement `Send + Sync + 'static`.
- In `has_analog_output`, answer from the block state alone. In `analog_output`, do not change the
  world. The game clamps the comparator signal to the range 0 to 15.

## Equipment eligibility

Implement `ItemPart::can_equip` to allow or refuse a stack in the candidate slot for a living
entity. `event.can_equip` starts with the vanilla data answer; set it to override the answer, or
leave it alone. Read live target state through its handle and the supplied read-only `Host`, for
example `event.entity.health(host)` or `event.entity.entity().position(host)`.

## Fire eligibility

`BlockPart::fire_odds` receives a read-only `LevelSide` on both sides. Its ignite odds
control fire placement and shape prediction on the client, including flint-and-steel
prediction; burn odds are used only by the server. The face points toward the fire.
Return `None` to keep the table. Client fire animation still uses vanilla's built-in table.

## Campfire smoke sources

`BlockPart::smoke_source` answers whether its block makes a campfire above it a signal fire.
Return `Some(true)` to enable signal smoke, `Some(false)` to refuse it, or `None` to keep
vanilla's hay-bale check. It runs on the server and during client placement prediction.
Its `LevelSide` supplies a read-only level on the asking side: answer from the supplied
state and level, with no authoritative writes. Parts attached by tag use that side's tag view.
