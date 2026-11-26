# Hiding terrain per section and per cell

`block_render_visibility` hides a block state everywhere. `section_visibility` decides where: for each chunk
section about to mesh in the player's current level, a client mod reads the raw section and returns what of
it draws. A cutaway above the player, an x-ray of one area or a structure preview that hides the terrain under
it are such answers. No NeoForge counterpart has been verified.

```rust,ignore
fn section_visibility(&self, _: &dyn ClientHost, event: &mut SectionVisibilityEvent<'_>)
    -> Option<SectionVisibility>
{
    if event.input.origin[1] > self.cut_above {
        return Some(SectionVisibility::hide_terrain());
    }
    let mut cells = Vec::new();
    for y in 0..16 {
        for z in 0..16 {
            for x in 0..16 {
                if event.input.cell([x, y, z]) == Ok(SectionCell::State(self.stone)) {
                    // Only the stone's top face draws.
                    cells.push(CellVisibility::new([x as u8, y as u8, z as u8], true, true, 1 << 1));
                }
            }
        }
    }
    SectionVisibility::keep(cells).ok()
}
```

## What the mod reads

`event.input` is the section and a one-block border on every side, edges and corners too: 18 cubed cells, local
-1 to 16 on each axis, with each cell's sky and block light, stored biome and deterministic model seed. A cell
is the world's canonical `BlockState`, before any visibility answer, so an ore a
`block_render_visibility` part hides still reads as the ore. A column that is not
loaded reads `SectionCell::Missing`: not air, but neither occluding nor matching anything. `input.biome(local)`
returns `Ok(Some(&BiomeKey))` for a registered biome, before biome zoom and blending. It returns `Ok(None)` for a
missing column or for an unresolved biome slot in a loaded column, such as a save's biome from an absent pack.
It never substitutes a rendering default. Reads beyond the border return `SectionVisibilityRefusal::OutsideInput`.

## What the answer does

- `SectionVisibility::hide_terrain()`: none of the section's block models, fluids or section geometry quads draw,
  and the section hides nothing behind it. Block entities that draw themselves, entities and collision are untouched.
- `SectionVisibility::keep(cells)`: each `CellVisibility` names a cell (0 to 15 on each axis) and whether its model and its fluid
  draw, and a face mask: bit 0 down, 1 up, 2 north, 3 south, 4 west, 5 east, 63 for all.

Answers only take away. A face mask removes faces after the normal culling and never shows a face the game
culled. A cell with neither model nor fluid stops occluding, as a hidden state does, so its neighbours' faces
toward it draw, across a section border too; its block entity still draws. A cell with faces removed occludes only
through the faces it keeps, and the section's see-through graph is worked out again from what still occludes.
Unculled quads (a torch's) go with the model. Translucent faces stay in the translucent pass. Every mod's answer
is combined with the others' by intersection: no mod brings back what another hid.

## Bounds and refusals

Each registration returns at most one answer. Return `None` to leave the section alone. The `keep` constructor
returns `E_SECTION_VISIBILITY_TOO_MANY` at the first record past one section's worth, without collecting the
rest. The game checks a returned answer whole: an out-of-section cell (`E_SECTION_VISIBILITY_OUTSIDE_SECTION`),
a face mask above 63 (`E_SECTION_VISIBILITY_FACE_MASK`) or a cell named twice (`E_SECTION_VISIBILITY_DUPLICATE`)
is logged once per mod and code, and the section draws as if the mod had not answered.
A listener that panics (`E_SECTION_VISIBILITY_PANIC`) is switched off for this hook for the session, alone: the
other listeners are asked as before and the level is remeshed without it, so no mask it gave earlier remains
in a cached answer or can publish from a queued mesh.

The snapshots, frozen plans and cached masks share a memory budget with section geometry. A section waits
for room before it captures anything or asks a listener. Cached answers have a separate ceiling within that
budget: at the ceiling a section meshes without its new mask (`E_SECTION_VISIBILITY_CAPACITY`) and is asked again
when room frees up. An existing mask can still be replaced or cleared. Section geometry counts visibility storage
against its per-section byte ceiling. `SectionVisibility::check` is the same per-answer check, for a mod's own tests.

## When it is asked

On the client world thread, before a section of the player's current level goes to a meshing worker, including
remeshes after a neighbour changes, arrives or unloads, or the resources reload. A section's first all-air compile
skips the hook when no mod contributes section geometry. Other levels drawn through level views do not call
the hook: the input and host describe the player's current level only.

A mod whose decision changes calls `ClientHost::refresh_render_visibility` to rebuild the current level's
meshes. Nothing is polled per tick or per frame. With no mod listening, nothing is captured or asked and the
mesher runs as before.
