//! The `section_visibility` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A chunk section in the player's current level is about to mesh: return one
    /// `SectionVisibility` or `None`. `input` is the raw section with its one-block border, read
    /// before any mod hid a state. `SectionCell::State` gives the canonical `BlockState`.
    /// Biome reads give the stored registered key, or `None` for a missing column or an
    /// unresolved biome slot.
    /// Asked on the client world thread before the section goes to a meshing worker, including
    /// remeshes after a neighbouring column arrives or unloads, a resource reload, or
    /// `ClientHost::refresh_render_visibility`. A first all-air compile is
    /// skipped if no mod contributes section geometry. Other levels drawn through level views
    /// do not call this hook. Answers
    /// only take away, and every mod's are combined, so no mod brings back what another hid. An
    /// answer that breaks a bound is refused whole, logged once, and the section draws without
    /// it. With no mod listening, the mesher pays nothing.
    #[notes = r#"## Section visibility

`section_visibility` decides, per chunk section, what the terrain mesh of that section draws: the whole section
(`SectionVisibility::hide_terrain()`) or, per cell, the block model, the fluid and a six-bit face mask
(`SectionVisibility::keep` with `CellVisibility` records). It extends `block_render_visibility`, which hides a
state everywhere, for decisions that depend on where a block is: a cutaway above the player, an x-ray of one
area, a structure preview that hides the terrain under it. No NeoForge counterpart has been verified.

```rust,ignore
fn section_visibility(&self, _: &dyn ClientHost, event: &mut SectionVisibilityEvent<'_>)
    -> Option<SectionVisibility>
{
    if event.input.origin[1] >= 64 {
        return Some(SectionVisibility::hide_terrain());
    }
    // Hide every cell of the mod's marker block, wherever it is in the section.
    let mut cells = Vec::new();
    for y in 0..16 {
        for z in 0..16 {
            for x in 0..16 {
                if event.input.cell([x, y, z]) == Ok(SectionCell::State(self.marker)) {
                    cells.push(CellVisibility::hidden([x as u8, y as u8, z as u8]));
                }
            }
        }
    }
    SectionVisibility::keep(cells).ok()
}
```

The input is the 16-cubed section and a one-block border on all six sides, edges and corners included (5832
cells), with each cell's light, stored biome and deterministic model seed. A cell is the state the world holds,
before any visibility answer: an ore a `block_render_visibility` part hides still reads as the ore. A neighbouring
column not loaded reads `Missing`: not air, but neither occluding nor matching; the section is asked again when
the column arrives or unloads. `SectionVisibilityInput::biome` returns the stored `BiomeKey` before biome zoom
and blending or `None` for a missing column or an unresolved slot. It never returns the rendering default in
place of a missing registered key. Reads beyond the border are refused.

Answers only take away. A face mask removes faces after normal culling and never forces a culled face to show. A
cell with neither model nor fluid no longer occludes, so its neighbours' faces toward it draw (also across a
section border); a cell with faces removed occludes only through the faces it keeps, and the section's
see-through graph is worked out again from what still occludes, erring toward drawing what is behind. Unculled
quads go with the model. `hide_terrain()` drops the section's block models, fluids and section geometry quads and
hides nothing behind it; block entities that draw themselves still draw. All mods' answers are combined by
intersection. Fluid heights and flow still use the captured states; visibility only affects which faces draw.

`keep` refuses the first record beyond one section's worth. A returned answer is checked whole: a cell outside
0 to 15, a face mask above 63 or a cell named twice refuses the answer with one log line
(`E_SECTION_VISIBILITY_...`), and the section draws as if the mod had not answered. A listener that panics is switched off
for this hook for the session, alone: the other listeners are asked as before, and every section
is asked again without the one that failed, so no mask it gave earlier stays. The decision is asked on the client world thread, so
it may read the `ClientHost`; it changes when the section meshes again, which a mod asks for with
`ClientHost::refresh_render_visibility`, never by polling."#]
    #[hook(
        receiver = "read",
        dispatch = "api_carrier",
        side = "client",
        contract = "collect",
        out = "answers",
        answered_by = "mod",
        version = 3,
        neoforge = "none verified"
    )]
    fn section_visibility(
        input: &SectionVisibilityInput,
        answers: &mut dyn FnMut(Answered<SectionVisibility>),
    );
}
