# Death sources at the loot hook

`LootDrops::damage_source` carries the death's damage type and causing/direct entity IDs.
Non-death drops have no source. Environmental deaths retain their type, with 0 for
absent or unknown IDs. `player` is remembered kill credit, not necessarily the attacker.

`tool` is the breaking or using tool (`LootContextParams.TOOL`), and is always `None`
for entity deaths. To read the attacker's main hand, resolve `damage_source.causing`
with `host.entity(id)`, then `host.living(entity)`, and call
`living.hand_item(host, Hand::Main)`. Handle resolution can fail for an absent attacker.
This is a live read, including all stack components: changes during `living_death`
and by earlier `loot_drops` listeners are visible to later readers. No hand snapshot
is carried in the context. Native and wasm use the same Host operations.

Projectile direct IDs are currently 0; melee direct IDs equal the causing ID.
The shooter's live hand is not the projectile's recorded firing weapon. Reading
that recorded weapon through the damage source is separate work.
