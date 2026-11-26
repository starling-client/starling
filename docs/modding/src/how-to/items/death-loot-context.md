# Death sources and weapons at the loot hook

## Roll death loot without dropping it

On a server host, call `entity.roll_death_loot(host, table, source, last_damage_player)`
with an `EntityRef` for a live living entity in the current level. Pass `None` for its
own loot table (including any saved override), or pass `Some(&table_id)` to roll a named
table in the same `ENTITY` context. The result is owned item stacks, not item entities.
This neither hurts the target nor drops equipment, custom death drops, or experience.

```rust,ignore
let damage_type = Identifier::parse("minecraft:player_attack")?;
let attacker = player.entity();
let source = LootDamageSource::new(&damage_type, Some(attacker), Some(attacker));
let drops = entity.roll_death_loot(host, None, source, Some(player))?;
```

`LootDamageSource::new` takes the causing entity and then the direct entity. For an
arrow hit those are the shooter and the arrow; for melee they are usually the same.
Pass the credited player separately: it supplies `LAST_DAMAGE_PLAYER` (used by
`killed_by_player`) and the player's luck. The attacker supplies enchantment levels
for loot functions such as `enchanted_count_increase`. The origin is the target's
position, and there is no `TOOL`. A roll uses the entity's nonzero death-table seed if
set; otherwise it advances the table's random sequence or the level's random stream.
Stale handles, non-living targets, unknown tables and unregistered damage types return a
`DeathLootError` before any draw.

The roll asks `enchanted_entity_loot` for enchantment levels, as a real death does. A
listener of that hook that calls `roll_death_loot` rolls again inside itself, so it must
stop its own recursion.

## Read the source of an actual death

`LootDrops::damage_source` carries the death's damage type and causing/direct entity IDs.
Non-death drops have no source. Environmental deaths retain their type, with 0 for
absent or unknown IDs. `player` is remembered kill credit, not necessarily the attacker.

`context.weapon_item(host)` is the death-loot read corresponding to vanilla's
`DamageSource.getWeaponItem`. It returns an owned snapshot that retains all stack
components, including custom names, enchantments and durability.

| Death source | Weapon returned |
| --- | --- |
| Bow arrow | The bow recorded when the arrow was fired |
| Crossbow arrow | The crossbow recorded when the arrow was fired |
| Thrown trident | The projectile's own original trident stack |
| Player in a Riptide spin | The stack recorded when the spin started |
| Other living direct entity | Its live main hand when the listener asks |
| Absent or unavailable weapon, or non-death drops | `None` |

Projectile weapons are captured only when damage is accepted and remain bound to that
accepted source. A later refused hit cannot replace them. They survive the projectile's
removal before loot runs. A bow named
“Longbow” with Power II reads as that bow after its shooter switches to a stick.
Melee remains live: a diamond sword with Sharpness V reads as an axe if the attacker
switches to the axe before the read. Earlier loot listeners' hand edits are visible.

`tool` is the breaking or using tool (`LootContextParams.TOOL`), and is always `None`
for entity deaths. For the causing entity's current hand independently of the damage
source's weapon, resolve `damage_source.causing` with `host.entity(id)`, then
`host.living(entity)`, and call `living.with_hand_item(host, Hand::Main, |stack| ...)`.
Read the item, count or damage directly in the callback:

```rust,ignore
let held_item = living.with_hand_item(host, Hand::Main, |stack| stack.item());
```

An empty or unavailable hand returns `None` without calling the callback. Use
`stack.project()` inside the callback for components or an owned stack; a projection
can refuse even when the hand is occupied. A shooter's current hand and its
projectile's recorded weapon are separate values; only `context.weapon_item(host)`
uses the projectile's record. `living.with_weapon_item(host, |stack| ...)` reads only
the given living entity's weapon on the server, including its active Riptide stack
when present.

Projectile direct IDs are 0; melee direct IDs equal the causing ID.
The recorded projectile weapon is carried separately in the call-scoped death context.

The live fallback follows `damage_source.direct`, rather than the causing entity. A
harming potion has a nonliving direct entity and therefore never returns its thrower's
held sword. If that direct entity is unavailable, the read returns `None`.
