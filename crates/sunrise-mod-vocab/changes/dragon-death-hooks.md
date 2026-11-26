# The ender dragon fires the living death hooks

The ender dragon fires `living_death` and `living_death_post` when its killing blow lands, as `LivingEntity.die` does in vanilla 26.3 (`EnderDragon.handleKillingBlow` only starts the dying flight); a cancelled `living_death` skips the kill credit and the post hook. Before, the dragon fired neither. The hooks' signatures are unchanged.

A mod that special-cased the dragon's death (polling its dying phase) can listen to `living_death` / `living_death_post` instead.
