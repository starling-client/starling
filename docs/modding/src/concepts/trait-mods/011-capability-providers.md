<!-- chapter: Capability providers -->
<!-- separator: 1 -->

A capability is answered by providers, registered by block entity type as NeoForge's
`RegisterCapabilitiesEvent` registers them: obtain `pipe` with the declaration builder or
`r.block_entity_type_key("mymod:pipe")`, then `r.block_entity_capability(&ITEMS, &pipe, |host, at|
Some(Box::new(PipeItems)))` gives a handler for the block entity and side `at` (a `BlockSide`), or none to
leave it to the next provider. A provider may answer for a vanilla block entity type too, using its lookup key.
Providers add up: `level.capability(host, &ITEMS, pos, side)` (`Level.getCapability`, on a `LevelRef`) asks those registered
with `r.block_entity_capability_before_game` first, then the game's own (a block entity's slots by side, by the
rules hoppers follow, and a mod block entity's `block_entity_container`), then the others, each in registration
order, and the first to answer wins. That order is the fork's choice, the game first and `_before_game` to go ahead
of it: NeoForge 26.3's javadoc doesn't state one between mods' providers and the game's. It returns a `Capability`, the handler and who answered. A handler holds
no borrow: each call is given the host, so it may call a
neighbour's capability, and it keeps its state in the block entity's slots, compound or attachments. A provider
is given the block entity and side, so the handler it makes knows where its contents live. Item handlers are
NeoForge 26.3's `ResourceHandler<ItemResource>`: indices from 0 (`size`), what each holds (`resource`, an
`ItemResource`: an item and its components, without a count) and how much (`amount`), how much of a resource an
index holds when it holds that (`capacity`, none where it may not go), and `insert` and `extract`, which take a
`&Transaction` and answer how much moved. There is no `simulate`: a write happens in a transaction
(`host.transaction(|host, tx| …)`, below), and to try one, abort it. `move_resources(host, &from, &to, filter, max,
tx)` (NeoForge's `ResourceHandlerUtil.move`) moves what `filter` accepts, up to `max`, index by index out of `from`
into wherever `to` takes it, each move all or nothing: what `to` won't take stays put. It answers how much moved, or
`Overanswered` (below). Hoppers use a mod's handler
where one is registered, each move in a transaction of its own. A mod's own capability is a
`CapabilityKey::new("mymod:chemicals")`, answered only by parts.

A `Capability` may be held for the tick: look a neighbour's up once per tick and reuse it for
every move that tick, as pipes and machines do. Ask again on the next tick, since the block
entity behind it may be gone by then. For a wasm mod each lookup crosses the sandbox (about
2.5 µs, see [Building a mod to `mod.wasm`](../../how-to/wasm-mods.md)), so holding it matters more there.
