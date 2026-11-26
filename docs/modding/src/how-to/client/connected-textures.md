# Connected textures for your blocks

A block whose texture joins its neighbours' (a casing, glass, a scaffold, a tank) is declared as data: a JSON file
that says which blocks and sprites it covers, which neighbours a face looks at, and which tile of a sprite sheet
each outcome draws. This declaration runs no mod code when the game meshes chunks.

The files are ordinary assets, read through the resource source: a resource pack overrides one by its path, as it
overrides your models, and they are read again when resources reload.

## A rule

`assets/<mod>/sunrise/connected/<name>.json`, one rule per file:

```json
{
  "method": "table",
  "blocks": ["mymod:casing"],
  "sprites": ["mymod:block/casing"],
  "faces": ["sides"],
  "connect": "block",
  "orient": "default",
  "sheet": "mymod:block/casing_connected",
  "table": "casing_full"
}
```

| Field | Meaning |
|---|---|
| `method` | `"table"`, the only method a mod file declares. |
| `blocks` | Block ids, each a string or `{"block": id, "state": {"axis": ["x", "z"]}}` (a state property and the values it may have). An id without a namespace is in your mod. |
| `tag` | A block tag (`mymod:casings`, with or without `#`) as another way to name blocks. An unknown or empty tag refuses the file. |
| `sprites` | The sprites the rule replaces. Leave it out to replace whatever sprite the block's faces use; name it to leave other sprites of the same block alone. A rule names `blocks`, `tag`, `sprites`, or several. |
| `faces` | `down`, `up`, `north`, `south`, `west`, `east`, `sides` or `all` (the default). |
| `connect` | What a neighbour must be to join: `"block"` (the default: the same block), `"state"` (the same block state), `"sprite"` (the same sprite on that face), `{"tag": "mymod:casings"}` (any block of a tag), `{"same": ["axis"]}` (the same block with equal values of the listed state properties: a pillar that joins only pillars along its own axis), or `{"height": {...}}` (by collision height, below). |
| `connect_group` | Optional boolean, `false` by default. When `true`, the block and each joining neighbour (including a covering block under `cover`) must also share the same non-empty mod-published connection group (`LevelRef::set_connection_group`). Blocks with no group join nothing. |
| `orient` | Where a face's left, right, up and down are: `"default"` (a vanilla cube's), `"axis"` (turned by the block's `axis` property, as a pillar's model is), or `"texture"` (the quad's own texture directions, so a turned or mirrored model turns its neighbours too: use it for a pane, whose vanilla model mirrors some faces, and for turned blocks whose model rotates the texture). A pane here is any block of vanilla's `IronBarsBlock` class (glass panes, iron bars, and a mod block declared on that template), whatever its name: it takes the cube's directions for its tiles and hides the strips shared with a stacked pane. |
| `sheet` | The sprite the tiles are cut from. |
| `table` | A table: the name of a shared one, or the table itself. |
| `weight` | Rules are tried in order of weight, highest first, then by file path; the first that applies wins. |

A file the game refuses (a missing sprite, a table whose length doesn't fit its tests, an unknown tag or test)
is logged by its path and left out; the block keeps its own sprite.

## A table

A table says which neighbours a face looks at and what each outcome draws. Shared tables live in
`assets/<mod>/sunrise/connected_tables/<name>.json`, so many blocks can use one; a rule's `table` may also hold the
same object inline.

```json
{
  "tests": ["left", "right"],
  "cover": false,
  "grid": [4, 1],
  "tiles": [3, 2, 0, 1]
}
```

- **`tests`** are the neighbours that count, from `up`, `up_right`, `right`, `down_right`, `down`, `down_left`,
  `left` and `up_left`. They are in the face's own directions (see `orient`): "right" on a north face of a plain
  cube is west. Each may appear once.
- **`cover`**: a neighbour does not join when the cell in front of it (on the face's side) would join as well and
  has a full face toward the neighbour, so a texture does not run across a step that something else covers. It is
  the rule's own `connect` that decides whether the front cell joins. Only that one cell is read: a second block
  stacked in front of it is not looked at.
- **`grid`**: columns and rows of the sheet, which is cut into square tiles, counted row by row from the top left
  (tile 0 is the top left, the next is its right neighbour). A tile with nothing in it draws nothing. Animated
  sheets work as other animated textures do.
- **`tiles`**: one tile per outcome of the tests: `2^len(tests)` entries. The outcome is a number built from the
  tests in the order you list them, the first being the lowest bit. With `["left", "right"]` it is
  `left + 2 * right`, so `tiles` reads: alone, joined on the left only, on the right only, on both. A table that
  does not have that many entries, or a tile outside the grid, refuses the file.

A corner that only counts when both its sides join is not a special case: give the entries where the corner is set
without its sides the same tile as without it.

Create's omnidirectional type, as an example, is a table of all eight tests, an 8 x 8 grid and 256 tiles; a
cross is four tests, a 4 x 4 grid and 16 tiles.

## Joining by height

`"connect": {"height": {...}}` joins a neighbour by how high its collision shape reaches, for roofs, slabs and
steps whose blocks join by height and not by name. A height is the highest point of a block state's collision
shape, in blocks; a state with no shape has none. A neighbour joins when any clause that is on holds:

| Clause | Joins when |
|---|---|
| `same` | the neighbour is exactly as high as this block, and is one of `of` (block ids, and tags with a `#`; any block when `of` is left out) |
| `higher` | the neighbour is higher than this block |
| `lower` | the neighbour is lower than this block (a cell with no shape is lower than every block) |
| `above` | the cell above the neighbour has a collision top, whatever the neighbour is |
| `side` | a neighbour on an edge of the face also joins when `higher` or `above` holds for either cell beside this block along the face's other axis. It needs `higher` or `above`. |

Heights are compared exactly: they come from the published collision shapes, read once when the resources load
(and only when a rule joins by height), and a state whose shape depends on the world has no cached shape, so it counts as having none, like air.

One rule per case, in order. Create's copper roofs, for example, are a rule for the bottom slab
(a `blocks` entry with `"state": {"type": ["bottom"]}`) that joins with `higher`, and a rule for the other roof blocks that joins with
`same`, `above` and `side`, both on the `up` face.

## Joining by a connection group

Some multiblock structures (fluid tanks, boilers, vaults) decide at runtime which touching blocks belong to the same
structure: two 2 × 2 tanks placed side by side share the same block state, but their textures must not merge across
the seam between them.

Set `"connect_group": true` on the rule and publish an opaque `u64` group identifier per block position from the
server with `LevelRef::set_connection_group(host, pos, Some(group))` (or `None` to clear it; read on either side
with `LevelRef::connection_group(host, pos)`):

```rust
{{#include ../../../examples/src/client_misc.rs:connection_group}}
```

- **Combined with `connect`.** `connect_group` narrows `connect`: a neighbour joins only when both the rule's
  `connect` predicate matches and `connection_group(pos) == connection_group(neighbour)` with a `Some` value. A
  position with `None` joins nothing (not even another `None` neighbour). Every `u64`, including `0`, is a valid
  group. When a table enables `cover`, the covering cell must also share the same connection group to suppress a join.
- **Derived visual state.** Connection groups are never saved to disk. For block-entity-backed structures,
  republish them from `block_entity_on_load` when `HostSide::Server`. It runs after saved data is available and
  before the server sends the column, even in a dimension held only by `Loaded` views. For chunks without block
  entities, queue the loaded chunk in `chunk_load` and publish on the next server tick (`chunk_load` receives a
  read-only `&dyn Host`); a loaded-only view does not tick. Also republish whenever your multiblock forms, splits,
  or changes membership.
- **Automatic lifetime.** Replacing a block with one that does not keep block entities (including through bulk
  `edit_blocks`), or attaching a fresh block entity at the same block state, ends the old group's lifetime.
  Changing only a state property of the same block (or weathering a copper chest or copper golem statue)
  preserves it. Writing to an unloaded chunk, an out-of-height position, a carrier-fenced cell, or an unchanged
  group returns `Ok(false)`.
- **Sync and transactions.** Negotiated clients adopt groups with the cell's current authoritative state,
  including in viewed levels. If only the group would be sent, the server sends the current block update first
  in the same bundle, only to negotiated clients. Group changes remesh affected sections and borders. Vanilla
  peers get neither the groups nor any additional updates. A client shares one active chunk cache’s worth of
  group cells across its active and viewed levels; replacing or unloading columns releases their old cells.
  The client refuses an update that grows occupancy beyond that limit. Replacements that do not add cells and
  removals remain allowed after the limit shrinks. A newer column snapshot takes precedence over older group
  corrections still waiting for a prediction acknowledgement. Writes inside a Host transaction are tentative:
  aborting restores the previous group quietly, and committing publishes the final block and group per position
  (including a clear caused by an intermediate replacement).

## What it joins, and what it does not

- Only faces that are drawn are looked at: a face vanilla culls has no connection to work out.
- The neighbours read are the ones within one block, on the face's plane, the cell in front of each for `cover` and the cell above each for `above`.
  The game re-meshes a section when a block within one block of it changes, and the one below when a block on
  its bottom two layers changes (the cell above a neighbour is two up), so a joined texture follows its neighbours.
  That holds in every world, with or without such rules: a block-state change on a section's second layer also
  marks the section below for remeshing if that section exists.
- A rule that also names a block the pack formats (OptiFine's connected textures) name is ordered with them:
  by weight, then by path. Tile rules (a rule with `sprites`) come before block rules.
- There is no hook that decides a connection with code, and a block cannot present another block's look to its
  neighbours (a facade). Blocks that must look like what they cover are outside this form.

The rules, the table and the neighbour geometry live in `sunrise-connected`; OptiFine's `.properties` files are
turned into the same rules by `sunrise-optifine`, so the two formats render through one path.
