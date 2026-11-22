# Pressing vanilla key mappings

A client mod can press any of the player's vanilla key mappings as their keys do: hold one down and
let it go, or click it. The game then acts through its own key handling, with every check it makes for
the player's keys, so the server sees only the packets of a normal client, at the vanilla moments.

During `client_setup`, resolve `host.vanilla_key_mapping("key.sprint")` and
`host.vanilla_key_mapping("key.attack")`. Store each successful `VanillaKeyMappingRef` in your mod's
client state. In client-setup or world-phase callbacks, call `sprint.set_down(host, true)` and
`attack.click(host)` with those same handles; handle any refusal as described below.

`mapping.set_down(host, down)` sets the held state (`KeyMapping.setDown`), without the click a
press adds. For a hold-style mapping, `false` releases the mapping even if the player is holding
its key or controller button; `true` holds the mapping without creating a second device hold.
Neither changes the physical input, and the next input for that mapping takes over again.
`mapping.click(host)` adds one click (`KeyMapping.click`), without holding the mapping.
A press of the key itself does both; a mod that wants a held attack that also starts at once calls both.

Attack, use, pick, inventory, swap-hands and drop clicks drain completely on the client tick
(`while (key.consumeClick())`), with no fixed per-tick or queue-length cap. The hotbar differs, as in
vanilla: each number key keeps its own click counter, and a selection drain consumes **one click per
key**, in order from key 1 to key 9 (`if (keyHotbarSlots[i].consumeClick())`), so the highest key
clicked wins and a key's further clicks wait for later drains.

Writes accept any resolved vanilla mapping (movement, gameplay, inventory, creative, multiplayer,
miscellaneous and debug). They act on the mapping, not its current physical binding, and are available
under `ClientSetup` access (both during `client_setup` and in world callbacks, with or without a
loaded world). Both return `Result<(), MappingError>`:

| Error | When |
|---|---|
| `MappingError::Unavailable` | the client backend is unavailable or the call is outside client-setup/world phases |
| `MappingError::Stale` | the handle does not belong to the current client; resolve its name again |

An unknown name fails at resolution (`MappingError::Unknown { key }`), not at either write.

## What the game does with them

The call only sets the held flag or adds the click, including during setup or behind a
screen. The client tick’s key step (`handleKeybinds`) does the rest. For example,
`key.chat` opens chat on that tick, not inside the mod callback. Opening a screen
releases the live mappings before any later keybind in the same tick can act:

- Attack, use and pick clicks are taken only with no screen or overlay open. The miss cooldown, busy
  hands and the item checks of `startAttack` and `startUseItem` apply, an entity attack respects the
  item's attack range, and the target is the one under the crosshair.
- An attack on a block sends the break start with its prediction sequence, then the punch packet; an
  attack on nothing sends the punch packet and sets the miss cooldown to 10 ticks (not in creative).
  With the attack mapping not held, the same tick lets go of the break and clears the cooldown, as a
  tap between two ticks does: hold it (`attack.set_down(host, true)`) for a break that goes on.
- A held use repeats every four ticks and does nothing more while the item is in use; letting go
  releases it (a bow shoots).
- A click on an inventory key acts as its key's press: a hotbar slot is selected on a subsequent tick, and
  drop, the inventory and the hand swap act as they do for the key. Ordinary inventory
  opens locally, releasing pending swap and drop clicks; the inventory key on a mount with
  its own menu sends the server a request instead, so swap and drop still follow it.
- Sneak, sprint, attack and use toggle when their toggle option is on: `true` flips them and `false`
  does nothing, as `ToggleKeyMapping.setDown`.

With a screen or overlay open, a click is dropped and a press holds nothing. Opening one drops direct
held-state writes, as `KeyMapping.releaseAll` does; set the state again once the screen is closed.

`interaction_key_mapping_triggered` fires at the attack or use as it does for the player's click, and
its cancel and swing work the same. `key_mapping_pre` is not asked: it is about physical key and mouse
button events.

The game has no bare swing to add: in 26.3 a swing sends nothing of its own, and the attack mapping
produces the swing and its packet.

## Reading live vanilla mappings

Resolve a vanilla mapping name such as `key.forward` once with `host.vanilla_key_mapping(name)`
during `client_setup`. Keep the `VanillaKeyMappingRef` for later callbacks. Rebinding, replacing options,
or changing worlds or screens does not invalidate it. Save the canonical mapping name, not the handle;
resolve the name again for a new client. Resolution returns `MappingError::Unknown { key }` for a name
that is not a vanilla mapping. Reads return `Stale` if the handle belongs to a different client and
`Unavailable` when the client read cannot run in the current phase or backend.

- `mapping.is_down(host)` reads the logical held state (`KeyMapping.isDown`) of any resolved vanilla
  mapping, including controller input, toggles and logical overrides.
- `mapping.physical_down(host)` follows the current binding and ignores toggles, conflict context,
  modifiers and logical writes. It returns `Ok(false)` when no physical key or button is bound.
- `mapping.binding(host)` borrows the current binding name (`&str`) from the host, for example
  `key.keyboard.w`. Read it again after a rebind; the borrow lasts only as long as the host borrow.
- `mapping.translated_key_message(host)` builds the current Controls label when you need to display it.
  It returns `MappingError::Label` if the label cannot be represented as API `Text`.

Use the borrowed name for comparisons within the callback, not a binding hash. If you need the
name after the host borrow ends, copy it explicitly. To hand a hold-style mapping back to the
player after a logical override, read `mapping.physical_down(host)`, then pass the successful value
to `mapping.set_down(host, down)`.

`ClientHost::is_key_down` still takes a physical key or mouse button name (`key.mouse.left`), rather
than a mapping name. For mappings your mod registers, see
[Registered key mappings](../native-mods/13-runtime-packages-and-identity.md#registered-key-mappings).
