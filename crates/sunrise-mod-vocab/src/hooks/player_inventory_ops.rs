//! Prose for the server's inventory and menu operations of `PlayerRef` and `MenuRef`.

pub trait Notes {
    #[notes = r#"## Selected slot, dropping a stack and closing a menu

`PlayerRef::set_selected_slot(host, slot)` makes `slot` (0 to 8) the hotbar slot a player holds
and tells their client with `SetHeldSlot`; a new slot ends a main-hand use in progress, as the
client's own switch does, and `player_switch_hotbar_slot_pre` is not asked, since it is for the
player's own input; it reaches the client of a guest acting in their own turn too.
`PlayerRef::drop_stack(host, stack, thrown_by_player)` is `Player.drop(stack, false,
thrown_by_player)`: the stack is thrown from the eyes as an item entity and `item_toss` may
cancel it. It takes nothing from an inventory, so take the stack out through the item places
first; that is what separates it from giving a stack that falls back to a drop when the
inventory is full. It returns `Ok(None)` once the item is in the world, and otherwise hands the
stack back untouched as `Ok(Some(stack))` (empty, a stale or dead player, a vetoed toss) for the caller to put back,
so a refused drop never loses items. As with other checked stack writes, a failure to convert
the API-owned stack returns `StackWriteError` before any game operation. API stacks are nonempty;
the engine empty-stack refusal remains internal.

`MenuRef::close_container(host)` closes a player's open menu on a server as
`ServerPlayer.closeContainer` does. A live handle close queues `ContainerClose` before tearing
down the menu and returning the carried stack, and fires `player_container_close` once. It
returns `false` when a disconnect is pending; if accepted during a click, the close runs only
when that click commits. The player's own inventory menu is never closed. A client closes its
own screen with `MenuRef::close`."#]
    fn player_inventory_ops_note0();
}
