# Item pickup notices

Listen to the server hook `item_picked_up_post` (NeoForge's `ItemEntityPickupEvent.Post`) to track
items a player receives from the ground. It fires once per pickup that put anything into the
inventory: when the whole stack went in, and when only part of it did and the rest stayed on the
ground, as when 8 apples lie there and only 5 fit.

It does not fire when `item_picked_up` refused the pickup or the inventory took nothing. A creative
player always takes the whole stack.

It carries the player's UUID, the item entity, `original`, a copy of the stack as it lay, and
`remaining`, the item entity's live remainder: `None` after a whole pickup, the part that did not
fit after a partial one. When the notice arrives, the amount the inventory took is `original`'s
count less `remaining`'s. Set `remaining` to change what stays on the ground, or `None` to empty the
entity; that changes the ground, not what went into the inventory, so read the amount before editing
`remaining`. Removing the entity in the listener keeps it removed, for the host's player and LAN
guests alike: there is no take animation, the stat and criterion run.

For a quest counter or custom pickup sound, count the amount as it stood when the notice arrived:
after any listener edits `remaining`, `original` less `remaining` need not equal what the inventory
took.
