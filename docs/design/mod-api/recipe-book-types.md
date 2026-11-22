# Design: custom recipe-book settings

> The native contract below still applies. The WebAssembly guest dispatch route was removed.
> Host and guest player swaps still refer to LAN players.
> See [recipe book types](../../modding/src/concepts/recipe-book-types.md) for the current native API.

Design for [#1023](https://github.com/arismoko/horizon/issues/1023).
Storage, registration, player settings handles and negotiated synchronization
are implemented. This is a Sunrise design; vanilla 26.3 is the reference for the
four existing books. No other project's implementation is a reference.

## Player settings operations

`PlayerRef::recipe_book_settings(host, &key)` reads an authoritative
`RecipeBookSettings { open, filtering }` pair. Fresh registered books read
`false, false`; unavailable players and unregistered keys return `None`.
`PlayerRef::set_recipe_book_settings(host, &key, settings)` is a server-only
write. It leaves every other book untouched, synchronizes changed flags and
notifies the existing `player_recipe_book_settings_change` moment once.
That notification now carries `RecipeBookIdentity`: one of the four vanilla
identities or `Custom(RecipeBookTypeKey)`, never a fabricated vanilla ordinal.
The values explicitly derive `Wire`.

A client mod reads `PlayerRef::synced_recipe_book_settings` and submits intent
with `PlayerRef::request_recipe_book_settings` through a `ClientHost`. Engine
menu panels use the same view through `Game::custom_book_settings` and
`Game::request_custom_book_settings`. Until a negotiated snapshot arrives, or
while a revision gap needs a replacement snapshot, reads return `None` and
the panel must stay disabled. Requesting never changes saved flags. Disconnecting
with pending intent restores the server's last accepted pair on the next join.

## Registration and ownership

Add an API-owned `RecipeBookTypeKey`, a validated namespaced resource id, and
register declarations in the existing mod registration phase. Registration
returns a typed key; it does not extend `BookType` or its ordinals. Require an
explicit namespace owned by the registering mod, lowercase resource-id syntax,
a nonempty namespace and path, and at most 256 UTF-8 bytes. Reject `minecraft:`.
Duplicate keys fail registration with the key and both declaring mod identities;
invalid ids report the mod, supplied id and violated rule. Never silently select
a winner. Freeze the registry when registration finishes, before loading player
state or negotiating connections. Late registration returns an explicit frozen
registry error. Resource reload does not reopen it.

Declarations are shared content on server and client. A client
must register a key to negotiate it; a server-only declaration can retain state
but cannot make an unmodified client display a book. Reads belong on a player's
recipe-book settings handle with `&dyn Host`; authoritative writes take
`&mut dyn ServerHost`. A client UI requests a change through its client settings
handle; it cannot directly mutate server state. Do not add player-id helpers to
`Host`. Registration uses its existing registration receiver.

Book identities are separate from `RecipeBookCategory` in
`crates/sunrise-mod-vocab/src/item/recipe_book.rs`. Categories group recipes and provide
icons; a book owns two flags. A screen associates categories with a registered
book explicitly through `MenuScreenLayout.recipe_book`. Neither category registration nor
recipe-type registration implicitly registers a book. Recipe unlocking and
placement remain separate from displaying the declared recipe layouts.

### Registration API

In `Hooks::register`, call `r.recipe_book_type("demo:cooking_pot")` on the
existing `Registrar`. It returns `Result<RecipeBookTypeKey, String>`; failures
include the declaring mod, supplied ID and rule and refuse installation even
if ignored. Keep the returned key (`mod_api::item::recipe_book::RecipeBookTypeKey`)
for screen bindings. `as_str()` returns its stable identity and `is_registered()`
checks membership in this process's installed registry. No public unchecked
constructor exists. The key explicitly derives `Wire`.

For Java-compatible flat persistence, call
`r.recipe_book_type_with_fields(id, open_field, filtering_field)` instead. Both
methods return the same key and enforce the same ID rules. The explicit field
names are case-sensitive nonempty strings of at most 256 UTF-8 bytes, with no
character-set restriction. Reject equal names and collisions with vanilla's eight
flag fields, `recipes`, `toBeDisplayed`, `sunrise:book_settings`, or another book's
fields. Invalid field declarations refuse installation even if ignored.

This registration does not create a category, settings handle, UI or network
payload. Successful installation freezes the registry; resource reload does
not reopen it. The settings and synchronization operations described here are
implemented on the typed player handle.

## Storage, persistence, copy and replace

Keep `Settings.open` and `Settings.filtering` as `[bool; 4]`. Add a custom map
from validated key to a value `{ open: bool, filtering: bool }` alongside them in
`crates/sunrise-client/src/recipe_book.rs`. Missing keys read as `(false,false)`.
There is no fifth vanilla index. Use one shared map implementation for local,
host and guest players, persistence and payload application.

For books without explicit fields, persist the map inside `recipeBook` under `sunrise:book_settings`, a compound
whose keys are book ids and whose values contain byte booleans `open` and
`filtering`. Write both flags, including zero, and sort keys for stable output.
Missing extension means an empty map. Preserve well-formed entries even when
the owning mod is absent; registration is required for live access and sync,
not for retention. Parse the extension independently of vanilla recipe-list
validation so an invalid recipe list does not erase retained custom flags.

Books declaring explicit fields instead read their flags directly from those
fields in `recipeBook`. Save each true flag as Byte(1) and omit each false flag,
as vanilla's `TypeSettings` codec does. Do not also write those books to the
extension and do not fall back to extension entries for them. Like existing custom
flags, read the flat fields independently of recipe-list validation. In-memory
state and synchronization continue to use the book ID, not its saved field names.

Bound the extension to 1024 entries, 256 bytes per key and 512 KiB encoded data.
Check lengths before allocating or iterating nested values. Accept only a
compound of compounds with both flags byte 0 or 1; skip malformed individual
entries with a diagnostic. Reject a duplicate key or an over-limit extension
as a whole, leaving the custom map empty and vanilla settings intact. The NBT
reader must enforce the byte/depth budget before materializing this subtree.
Registration also caps custom types at 1024, reporting overflow at registration.
Do not evict absent-mod entries to make room: a write exceeding the cap fails
without changing state. This bounds retention as well as network input.

Copy is an independent value copy of all four vanilla pairs and every custom
entry, including absent-mod entries. Mutating either player afterward cannot
change the other. Replace assigns the complete source settings value, clearing
destination-only entries; it is never map merge. Existing recipe and highlight
sets retain their existing whole-Settings copy semantics. Import without the
`recipe_book` section preserves the complete destination value. Import with the
section replaces it, including clearing custom entries when the extension is
missing. Host/guest swaps must move the complete value.

## First sync and subsequent changes

The server owns persisted player state. Negotiate a custom-settings capability
and the intersection of registered keys during connection setup. Send no
extension payload without successful negotiation. Namespaced keys, not process
local registration indices, identify entries on the wire. Vanilla settings and
seen/add/remove packets retain their current fields, ordinals and meaning;
a vanilla client receives exactly the existing four-book traffic.

After player load and ordinary initial recipe-book packets, send one bounded
custom snapshot on the negotiated mod payload channel, before enabling custom
book UI. Snapshot includes every negotiated key, explicitly `(false,false)` when
absent from storage. Applying it replaces the client's custom view. Retained
unregistered or unnegotiated keys never leave the server. Snapshot delivery is
also required on imported-state replacement and every reconnect. Clear the old
client view and pending requests at disconnect; never upload that cache as a
replacement for server state.

Use reliable ordered delivery on the connection. A snapshot starts a fresh
connection epoch at revision 0; each accepted server change increments the
revision. Updates contain epoch, revision, key and both flags. Clients apply
only the current epoch in revision order; a gap disables the custom UI until
a fresh snapshot arrives. Client requests carry the current epoch and a
monotonic request number; the server validates ownership of the connection's
player, capability, negotiated key, booleans and bounds before applying once.
Reject stale/duplicate requests without mutation; answer with authoritative
state so the UI can settle. Later valid requests are processed in arrival order.
Server writes use the same apply path, then send the resulting update. Notify
gameplay once after an accepted change, never for rejection or an unchanged
pair, through the existing settings-change moment extended to typed identity.

Cap payloads at 512 KiB and snapshots at 1024 distinct entries; updates contain
exactly one entry. Reject malformed/oversized payloads before allocation or
mutation, including duplicate snapshot keys. Rate-limit requests to 20 per
second per player; excess requests are rejected and cannot alter flags. Client
UI may show pending intent but only authoritative replies enter saved state.
The design called for single-player to use the same server apply operation without serialization; this build instead preserves the existing integrated server's byte-pipe connection and identical negotiated, rate-limited authority semantics, because a typed local route needs per-connection ordered queues bound to integrated player ownership and reconnect lifetime.

## Worked book and acceptance cases

A cooking mod registers `demo:cooking_pot` during registration and binds its
cooking screen to that key. The screen reads `(false,false)` initially. Opening
it requests `(true,false)`; the server accepts, stores that pair and sends it
back. Selecting the craftable-only filter requests `(true,true)`. A category
such as `demo:soups` can appear in that screen without owning either flag.
These are proposed operations, not compilable API examples.

Use these independent expectations in implementation tests:

| Case | Expected result |
| --- | --- |
| Fresh pot and `demo:oven` | Both `(false,false)`; opening pot leaves oven `(false,false)` |
| Save pot `(true,false)`, load, reconnect | Pot exactly `(true,false)` before UI opens; all four vanilla pairs unchanged |
| Disconnect with pending `(false,true)` not accepted | Reconnect snapshot restores server's `(true,false)` |
| Save with mod absent, then reinstall | Retained pot `(true,false)` becomes accessible again; no absent key was synced |
| Copy A with pot `(true,false)` into B; set A to `(false,true)` | B remains `(true,false)` |
| Replace B containing oven `(true,true)` with A containing only pot `(true,false)` | B map contains exactly pot `(true,false)`; oven reads default |
| Load without extension | Empty custom map; registered pot reads `(false,false)` |
| Unknown, wrong-side, stale or oversized update | No flags change and no gameplay notification |
| Add category `demo:soups` | No new settings entry or vanilla ordinal |
| Vanilla peer | Existing four-book packet bytes and behavior match vanilla 26.3 |

## Current routes to update

References below are verified against this worktree at `84e57e6a79`; paths are
relative to `crates/sunrise-client/src/`. Implementation must keep each route
on the shared settings operations rather than duplicate custom-map handling.

| Route | Current file:line | Required handling |
| --- | --- | --- |
| UI toggles and send | `game/items/recipe_book.rs:644`, `:827` | Dispatch vanilla identity to existing packet; custom identity to request |
| Remote settings application | `game/items/recipe_book.rs:1339` | Keep vanilla assignment separate from custom snapshot/update |
| Server validation | `lan/hub.rs:4491` | Preserve four-type ordinal validation; validate custom payload separately |
| Gameplay notification | `game/network/lan/mod.rs:2853` | Notify after shared authoritative apply |
| Initial sync and reconnect | `lan/hub.rs:4001`, callers `:1551`, `:1704` | Send negotiated snapshot after ordinary packets |
| Initial packet construction and resend | `lan/hub.rs:63`, `:4077` | Preserve vanilla packet construction; include custom snapshot on replacement |
| Persistence | `save.rs:4426`, `:4601`; `recipe_book.rs:926`, `:963` | Load/write extension with shared bounded codec |
| Host to guest and guest to host | `lan/playerdata.rs:291`, `:633` | Clone the complete value independently |
| Import preservation and replacement | `lan/import.rs:408`, `:460` | Preserve or replace whole map according to section presence |
| Acting-player swaps | `game/network/lan_actions/turns.rs:354`, `:808` | Swap complete Settings, including custom map |

The vanilla reference queries are `rea_vanilla.py method
net.minecraft.stats.RecipeBookSettings replaceFrom` and `rea_vanilla.py method
net.minecraft.stats.ServerRecipeBook sendInitialRecipeBook`. Custom registration,
retention and negotiation are Sunrise decisions, not claims about vanilla.

## Recipe displays and category membership (#828)

The `recipe_book_categories` declaration now carries `target_menu`, `book`,
`search_group` and `displays` beside the category id, recipe type and icon.
A type supplies `RecipeBookDisplayEntry` values for its recipes during the
existing recipe-table declaration callback. The callback can read the Host's
recipe table and interpret its own JSON; the engine does not impose a JSON
schema on mod recipes.

`RecipeBookDisplay` names the shaped, shapeless, furnace, stonecutter and
smithing layouts. `RecipeSlotDisplay` describes slots using API-owned values:
items, stacks, tags, fuel, potion/component transforms, dyeing, trimming,
remainders and composites. All three display types explicitly derive `Wire`.
Named resource references are validated against the owning resource generation
at load. Display stacks are materialized when the panel's content changes.

The engine's `ModBookIndex::build` boundary validates declarations atomically
against resource tables. Unknown resources, invalid layouts, duplicate recipe
ownership and conflicting book membership return `DisplayLoadError` values.
A book has one menu and one search group; no search group can aggregate two
books. Search membership is derived from the book's categories, so two mod
books remain isolated and vanilla books have no mod search members.

The live recipe-table declaration path validates the entire batch before
publishing any categories. A refused batch retains its typed load error for
that generation and publishes no partial book; reload or disconnect clears
the cache. Multiple categories may share a recipe type. The engine's panel
projection requires both the declared target menu and registered book key,
and a category selected in another book cannot be reused as a tab here.

### Hosting the panel in a mod screen

Register a client factory for the menu. The window uses that factory to build a fresh
`MenuScreen` with local `MenuScreenLayout` geometry; see the
[recipe-bound example](../../modding/src/how-to/items/recipe-book-categories.md#binding-a-book-to-a-menu).
Set `layout.recipe_book` to the registered key. A bound menu must name its bound key,
and a different menu cannot display that book. Leave the field `None` for an unbound
screen without a book. Use struct update syntax for optional layout fields; the defaults
do not provide valid dimensions or populate slots. The open panel uses the normal
narrow-window coverage and wide-window menu shift.

The panel is disabled until the connection has an authoritative settings
snapshot for the registered key. Opening and filtering submit requests through
the shared custom-settings route; neither click optimistically changes the
saved flags. Tabs show this book's category icons, with a search tab aggregating
only this book. Four category tabs fit on each tab page; separate arrows reach
additional categories while search stays visible. Search matches displayed result names. The craftable filter
matches the display's required input alternatives against player inventory and
visible menu contents, not the custom recipe's opaque JSON logic.

Recipe buttons cycle through their declared result displays. Clicking a button
opens a display preview using the shared recipe-book overlay painter, including
stonecutter and smithing ingredient layouts. Larger ingredient grids scale to fit
the preview instead of spilling into adjacent alternatives. These previews do not place recipes,
create ghost slots, or send a custom id through vanilla placement packets;
placement and ghosts are tracked separately in #2599.

A declaration's `book` is a registered `RecipeBookTypeKey` from #2415; it does not
implicitly register a book or create settings. The index checks the key against
the frozen registry. No vanilla book ordinal or packet meaning changes.

## Transport and cost

The dedicated `sunrise:recipe_book_settings` channel is used only after the
server announces mod transport and the client offers its frozen registered
keys. The server intersects those keys with its own registry before replying
with the initial snapshot. Vanilla clients never send the offer and receive no
custom settings. The bounded binary codec refuses oversized payloads, invalid
flags, trailing bytes and duplicate snapshot keys before applying anything.

Negotiation, snapshots and requests do work only on connection or a settings
change. There is no per-tick mod scan or new tick allocation. A connection's
request replay guard and rate allowance survive rejected requests and repeat
offers. Disconnect removes the connection authority and clears pending client
intent; the saved server map remains independent of the client view.

The response budget covers every payload, including malformed input and
snapshot requests. Once exhausted, a negotiated peer receives a one-byte busy
control reply instead of another large snapshot. The client disables the custom
view and coalesces a retry after one second; a busy response schedules the next
retry, and a successful authoritative snapshot cancels it. This bounds response
amplification without leaving a lost resnapshot or pending UI intent unresolved.

## Toggle layout and large books

`MenuScreenLayout.recipe_book_toggle` optionally reserves a 20 × 18 GUI-pixel toggle:
`RecipeBookToggle { x, y, from }` puts it `x` pixels right of the menu's left edge and `y` below the
menu's top edge (`from: Menu`) or below the window's centre (`from: ScreenCentre`, vanilla's
`height / 2` with integer division, so `leftPos + 5, height / 2 - 49` is `x: 5.0, y: -49.0`).
Like vanilla's screen-specific button positions, this is space the screen author must keep
clear of slots and labels. The anchor and its coordinate origin are local to the window,
not part of the transmitted recipe-book declaration.

Without an anchor, the client tries an external right, left, then above-menu
gutter, rejecting positions covered by the overlay panel. Menus wider than 176
GUI pixels, windows narrower than 379 GUI pixels, or windows narrower than the
menu width plus 202 GUI pixels use overlay mode instead of shifting slots. If
the toggle does not fit in the viewport in the closed layout or in the open
layout, the book is refused with a diagnostic identifying the mod block and menu.
The diagnostic requests `MenuScreenLayout.recipe_book_toggle` when absent, or reports an
anchored toggle outside the viewport when the declared anchor does not fit. The menu still opens unchanged; the
engine never clamps a toggle onto mod slots or clips the menu to make room.

The client caches resolved result/search metadata for the screen binding.
Stock changes re-evaluate craftability and filtering, but only the selected
page's twenty recipes materialize ingredient previews. Search still considers
the entire declared book, not just the visible page.

## Menu-owned destination roles

During shared registration,
`r.menu_recipe_book(menu, &book, recipe_type, layout)` binds a content menu
to a registered book, a recipe type and its destination roles. The menu namespace
must belong to the registering mod. Duplicate bindings and unknown books refuse
installation even when the registration error is ignored. The roles are frozen with
the other registrations; `RecipeBookLayout` derives `Wire`.

`RecipeBookLayout` declares one of four destination maps:

- `Crafting { width, height, slots, result }` supports shaped and shapeless
  displays. Slots are row-major container indices.
- `Furnace { input, fuel, result }` supports cooking displays.
- `Stonecutter { input, result }` supports stonecutting displays.
- `Smithing { template, base, addition, result }` supports smithing displays.
- `Mod` declares no roles: the menu's `MenuPart` answers a click with `recipe_book_place` and places with
  `ServerHost::place_recipe_in_grid`, vanilla's `ServerPlaceRecipe.placeRecipe` with the mod's own grid.

These are container slots, not the appended player inventory. Overlapping roles,
invalid dimensions and indices outside the 256-slot menu bound refuse registration.
Book loading checks against the actual content menu size. If the menu has an
instantaneous `MenuCrafting` backend, its grid and computed result must exactly
agree with the declaration. Without that backend, a crafting map describes real
persistent input storage instead: for example, six cooking-pot inputs in a 3 by 2
grid. Placement never fills its cooked output or fuel, and preserves those slots.
An incompatible display or one that does not fit refuses the declaration
generation rather than truncating ingredients. No permanent slot mask may exclude
a mapped role. The client obtains geometry from the menu's registered screen factory at
open, just as it does for other mod menus. A bound menu's layout must name the
registered book and supply finite coordinates for all visible backing slots.
A layout that disagrees with the mask or extends beyond the backing, supplies
invalid dimensions or coordinates, or names an unknown empty-slot sprite is refused
at open. Every excluded slot retains its backing index; the server's mask and
recipe destinations are independent of any local screen move or hide.

The shared mapper uses vanilla `PlaceRecipeHelper`: a 1 by 1 shaped recipe in
a 3 by 3 grid is centered; a 2 by 2 recipe starts at the upper left and retains
its holes. Shapeless ingredients occupy consecutive cells. Furnace fuel is
included in a preview only when its slot is empty. Station icons never map to
menu ingredients. Result display stacks retain their components and counts.

## Authoritative placement information

A menu-bound recipe type supplies `RecipeBookDisplayEntry.placement` as
`Some(PlacementInfo { cells, shape })`. Each cell is either an authoritative
`Ingredient` predicate or an empty position; `shape: Some([width, height])`
preserves shaped holes, while `None` gives the menu's ordered inputs. The recipe
type owns these predicates, including custom component-sensitive tests. Display
pictures never substitute for them. Use `PlacementInfo::NOT_PLACEABLE` explicitly
for a preview-only recipe. Omitting placement information from a bound recipe
refuses its declaration load rather than guessing ingredients from its icons.

The shared Wire boundary allows 32 structural record levels, including the
category, display-entry and placement-info envelope. Nested ingredient shapes
must fit that complete envelope; the codec refuses excess depth with `WireError`.
Opaque NBT has its separate,
iterative vanilla limit of 512 container levels; it does not spend one structural
level per NBT container. Wide, shallow cooking grids do not consume extra depth.

Bound menus use the server's negotiated catalogue, not a reconstruction from
local client recipe JSON. Each request names the exact published recipe and
original display alternative, scoped to the connection epoch, open window,
screen nonce, server menu instance and declaration generation. Left click places
one increment; Shift requests the maximum. Right click opens the alternatives
preview, whose left-click selection preserves the original source index even
after filtering and craftability sorting. Unbound books remain preview-only.

The server tests the recipe type's authoritative predicates against exact
component-aware stack classes and rechecks its live context after callbacks.
It commits placement atomically, or refuses without publishing a partial plan.
Persistent machines retain their real fuel and cooked output; placement does not
craft their output. Only a current server missing-ingredients response installs
ghost hints. Every new click, window change, disconnect or declaration reload
revokes pending hints. Ghosts use the shared vanilla painter and the active
screen's native slot geometry, never built-in slot coordinates.

Displays are pictures, not ingredient predicates; declaring a result ghost never
grants a real stack. NeoForge 26.3 public documentation names no registration
counterpart for vanilla's internal `GhostSlots.setInput` and `setResult`; these
declaration names are Sunrise's.
