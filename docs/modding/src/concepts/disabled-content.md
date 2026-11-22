# Switching content off

Switching a mod off suspends its code, not its registered content. Blocks keep their appearance,
collision, loot and data-only behaviour. Where the content owner's Part subscribed to a world-writing
behaviour, the disabled block is inert rather than inheriting its vanilla template's implementation:

- `random_tick` skips the template tick. `tick_block` holds the scheduled occurrence before dispatch
  to any Part, including enabled Parts. Re-enabling the owner dispatches each held occurrence once,
  with its original deadline, priority and order.
- `can_be_hydrated` leaves the farmland's moisture and block state unchanged on random ticks
  unless an enabled mod's session water ticket or an earlier enabled Part waters it. Rain does not
  wet held farmland, and holding does not prevent trampling. Moisture and dry-out checks resume
  when the owner is switched back on. Farmland without a hydration Part from its owner follows
  its usual rules.
- `update_shape` keeps the input state. `can_survive` keeps the existing block, including a block
  settling after a piston move or a fall. `may_place_on` and `can_sustain_plant` likewise keep
  a plant that already occupies the cell or is settling there. These survival rules do not let a
  player bypass support checks to place a new block or plant.
- `use_item_on` and `use_without_item` pass without invoking the block template's interaction.
- `is_valid_bonemeal_target` refuses bone meal. For `on_tree_grow`, the soil under the trunk stays
  unchanged and the rest of the tree grows normally.
- `dispense_method` retains the original input stack and emits no dispense event unless an earlier
  enabled Part supplies a valid reply. A rejected reply does not restore template behaviour.
- `tool_modified_state` suppresses an unchanged template result, not a listener's replacement.
  The global `block_tool_modification` hook still hears the candidate for each attempted ability, and
  the tool can try its next ability if there is no result.
- Block entities belonging to the disabled mod do not tick, including through native tick loops.
  Their persistent state stays in the world and resumes when enabled.

Except for scheduled block ticks, these defaults occupy the disabled owner's usual place in Part
dispatch: earlier enabled Parts can answer first. A disabled mod's Part attached to another owner's
block does not make that block inert. Read-only queries such as path classification, light, fluid
state and loot retain their ordinary answers. Attacks, explosions, environmental hazards and
player-initiated removal still use data and template rules: switching the owner off does not make a
block immune to those actions. `piston_push_reaction` has no inert answer: the disabled owner's
Part does not override the data or template reaction. Pistons may still move or break the block
according to those rules.

Items and entities keep their registered data and default engine behaviour while their Part
callbacks are disabled. They are not globally frozen by this block-specific policy. In particular,
item use code is skipped but ordinary data-driven item behaviour remains; entity movement and
vanilla AI are not suspended. Do not switch a mod off as a way to pause its entities.
