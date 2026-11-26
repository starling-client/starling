# Drawing resolved models

Register standalone models during `standalone_models`, then resolve their `ModelRef`
through `RenderAssets::model` after resources are published. `submit_custom_geometry`
(and registered debug renderers) receive `WorldDrawContext`: submit transient copies
with `draw_model_instances`, or retain copies in a group. Update retained groups before
render passes execute in `prepare_render_buffers`. GUI scenes, mobs and kept/spinning
renderers use their CPU routes; groups are a world-drawing facility, not another renderer.

## Local player skins

Implement `Hooks::resolve_player_skin` to change the local player's appearance
without changing their profile. Upload the skin with
[`create_dynamic_texture`](dynamic-textures.md) as a 64×64 texture once per
skin file. Normalize legacy 64×32 skins before uploading. Keep the texture id
and its model width, then select them in the hook:

```rust,ignore
fn resolve_player_skin(&self, _: &dyn ClientHost, event: &mut ResolvePlayerSkinEvent<'_>) {
    if let Some(skin) = self.selected_skin() {
        *event.selection = Some(PlayerSkinOverride::new(skin.texture.clone(), skin.model));
    }
}
```

Use `PlayerModel::Wide` for four-pixel arms and `PlayerModel::Slim` for
three-pixel arms. The selection starts at `None` each frame; listeners can leave
an earlier answer alone or clear it with `*event.selection = None`. A missing,
released or wrongly sized texture keeps the original texture **and** model. A selected
texture is copied on the GPU into the player's skin slot each frame it is selected.
The selection affects the third-person avatar and its skin layers, first-person
arms, and inventory and creative previews. It does not change the cape, tab
list, or anyone else's view.

To animate the avatar's arm separately from the item, use `RenderHandEvent`'s
`HandDrawContext`. It starts in first-person view space under the hand sway,
before the normal equip and swing transforms. `MainHand` here selects a
physical arm; the callback's `Hand` selects a logical slot.

```rust,ignore
fn render_hand(&self, host: &dyn ClientHost, event: &mut RenderHandEvent<'_, '_>) -> Flow {
    if event.render.hand != Hand::Main {
        return Flow::Continue;
    }
    let side = if host.gui_options().main_hand_left { MainHand::Left } else { MainHand::Right };
    let draw = &mut *event.draw;
    draw.push();
    draw.translate([0.3, -0.5, -0.7]);
    draw.render_arm(side);
    draw.pop();
    if let Some(stack) = event.render.stack {
        draw.push();
        draw.translate([0.2, -0.3, -0.7]);
        draw.item(stack, ItemDisplayContext::Fixed, None, 0);
        draw.pop();
    }
    Flow::Cancel // replace this hand's default; Continue would add to it
}
```

The arm uses the frame's selected skin and width, that side's sleeve setting,
and the draw context's fixed light. Explicit draws do not fire `render_arm`.
Cancelling defaults leaves these draws and the other hand intact.

## Third-person held items

Attach an `ItemPart` with `Registrar::item_part` (or `item_part_tag`) to tint an
item in players', mobs' or armor stands' hands in the world, including vanilla
items. `held_item_tint` runs on the client just before the resolved item is
submitted. Use the holder's current level entity ID and logical `Hand::Main`
or `Hand::Off`, not the left or right arm:

```rust,ignore
#[mod_api::sunrise_part]
impl ItemPart for ChargedTint {
    fn held_item_tint(
        &self,
        _host: &dyn ClientHost,
        event: &mut HeldItemTintEvent<'_>,
    ) -> Option<u32> {
        self.is_charged(event.entity_id, event.hand).then_some(0xFFA040)
    }
}
```

The event also borrows the rendered stack and carries the frame's `partial_tick`.
The first `Some(0xRRGGBB)` wins; return `None` to let the next Part answer or keep
the model's colors. The multiplier applies to that draw's RGB before lighting,
including a shield's base and banner patterns. It leaves alpha, glint, outline
and pose alone, and does not affect first person, GUI previews, worn armor, item
frames or dropped items. Keep the handler fast: it can run once for each held
stack submitted during a frame. Rendering is not a source of combat state; see
[Syncing remote item cooldowns](networking.md#syncing-remote-item-cooldowns).

## Entity model layers

At client startup, implement `Hooks::register_layer_definitions` (hook version 5) and call
`event.register("mymod:golem#main", create_golem_layer)`, where `create_golem_layer`
returns a `LayerDefinition` built from a `MeshDefinition` and the texture size.
The supplier is `Fn() -> LayerDefinition + Send + Sync + 'static`: capture owned
inputs, not the event or its registration context. Write the model `Identifier` as
`namespace:path` (with `:` even if a half is empty), followed by `#` and the layer name.
The suffix after the first `#` is preserved verbatim.

Duplicate mod registrations for the same ID overwrite earlier suppliers (last-writer-wins)
before freeze, so overwritten suppliers never run. Malformed IDs, collisions with existing
vanilla layer definitions, or an unavailable/invalid vanilla `entity-models.json` fail
startup freeze cleanly before invoking any supplier or publishing. The game calls each
winning supplier once when it freezes the layer table. A panicking supplier leaves no
layer and is reported in the error log and client error toast under its mod's name.
Other suppliers, including that mod's other suppliers, still run. The frozen table
retains the definitions, not the suppliers. Call
`ClientHost::bake_layer("mymod:golem#main")` during renderer setup, then keep the
resulting `ModelPart` tree for posing and drawing. Baking again and reloading
resources reuse the definition.

## Worn humanoid equipment

An item attaches one `ClientItemExtensions` Part in
`Hooks::register_client_item_extensions`, on the client at every resource load before any
reload listener applies. The validated bindings are installed with the new models. A panic discards
that mod's registration batch and disables its registration callback for the rest of the session.
The event's `extensions: &mut dyn ClientItemExtensionsRegistrar` registers the
models, textures and item providers together. There is no startup provider
registration path: request your `ItemKey` during ordinary registration, then
attach its provider during this resource-load event.

```rust,ignore
let inner = event.extensions.register_model("mymod:inner_chest", &inner_tree)?;
let outer = event.extensions.register_model("mymod:outer_chest", &outer_tree)?;
let inner_texture = event.extensions.register_texture("mymod:entity/equipment/inner")?;
let outer_texture = event.extensions.register_texture("mymod:entity/equipment/outer")?;
event.extensions.client_item_extensions(item, Arc::new(ChestLayers {
    models: [inner, outer],
    textures: [inner_texture, outer_texture],
}))?;
```

Implement `ClientItemExtensions` using the borrowed draw inputs and the
reload-bound references. If the equipment asset's `humanoid` layers specify
`mymod:chest_inner` and `mymod:chest_outer`, the renderer expands them to the
sprite names used in this match:

```rust,ignore
impl ClientItemExtensions for ChestLayers {
    fn armor_texture(
        &self,
        _host: &dyn ClientHost,
        _stack: &ItemStackView<'_>,
        _layer_type: EquipmentLayerType,
        layer: &EquipmentLayer<'_>,
        _default_texture: &str,
    ) -> ArmorTextureRef {
        match layer.texture {
            "mymod:entity/equipment/humanoid/chest_inner" => self.textures[0],
            "mymod:entity/equipment/humanoid/chest_outer" => self.textures[1],
            _ => ArmorTextureRef::DEFAULT,
        }
    }

    fn armor_layer_model(
        &self,
        _host: &dyn ClientHost,
        _stack: &ItemStackView<'_>,
        _layer_type: EquipmentLayerType,
        _layer: &EquipmentLayer<'_>,
        index: u32,
    ) -> ArmorModelRef {
        self.models.get(index as usize).copied().unwrap_or(ArmorModelRef::VANILLA)
    }
}
```

Trees are API-owned `ModelPart` trees, built directly or baked from a
`LayerDefinition`. Register them once per resource load, never during a frame.
Register only sprites the provider may select as overrides: `register_texture`
binds each requested sprite and its UVs for the current load. Vanilla/pack/CIT
selection requires no registration. Model names may use any valid namespace.
Duplicate model names, non-finite geometry or poses, unknown textures or items,
and a second provider for an item return an error from that registrar call.
Other successful registrations still install. A duplicate item error names both
providers and leaves the first in place. Disabling the owning mod stops its provider.
A server-only package cannot declare this client registration hook.

The worn stack must first pass the equippable component's asset and matching-slot
checks, and the asset must have layers. Only the visible equipped item's provider
is queried; random-texture inventory enumeration does not run frame callbacks.
Every callback takes `&dyn ClientHost` and a live `&ItemStackView<'_>`. Read a
single stored component override with `stack.get(&descriptor)` or explicitly
call `stack.project()` if you need an owned stack. Callbacks for individual
layers also borrow `&EquipmentLayer<'_>`; its `texture` is the asset's texture
name before any renderer override.

1. `default_dye_color` runs once. By default it reads the stack's dyed colour
   override and returns packed ARGB, or zero when undyed.
2. `armor_layer_tint_color` runs in asset layer order, with the layer and its index.
   Exactly zero skips the layer, without calling its texture or model method.
   The default is white for a nondyeable layer, or its dye/optional undyed colour
   for a dyeable layer. Without either colour, a dyeable layer is skipped.
3. `armor_texture` receives the layer type, layer and `default_texture: &str`:
   the renderer's texture name after pack/CIT selection, or the asset's name if
   the renderer could not bind it. Return `ArmorTextureRef::DEFAULT` to keep that
   selection. Returning a registered sprite is final for that layer; it is not
   replaced by another random-texture or custom-model texture.
4. `armor_layer_model` chooses the registered tree for that layer. Returning
   `ArmorModelRef::VANILLA` keeps the original model. The humanoid pose is copied
   onto the chosen tree, with only the current slot's parts visible.

Per-layer model selection deliberately differs from NeoForge's one model per
piece. It lets a chest item draw an inner shape with one texture and an outer
shape with another, without painting both textures on both shapes. There are no
additional per-piece `humanoid_armor_model` or `generic_armor_model` callbacks.
The trim draws once on the last material layer actually drawn's tree; if no
material drew, it uses vanilla's model. The trim does not call the model method
again. Babies do not draw trims. Foil goes on the first drawn material when there
is no trim, and on the trim otherwise.

Model and texture references are scoped to the resource load. A stale reference
uses vanilla's model or the supplied default texture and is logged each time
without disabling the provider. If a callback is refused or the equipped stack
is invalid, the refusal is reported each time and the whole piece draws as
vanilla for that draw (even if an earlier layer succeeded), without disabling
subsequent valid draws. A panic also discards that piece's custom layers, logs
and shows an error toast naming the mod, and disables that mod's provider for
that item for the rest of the session, including across resource reloads.
Callbacks work with borrowed draw inputs and select reload-bound references,
not geometry or rebakes.

## Standalone model bakers

Implement `Hooks::standalone_models` on the client. It receives a `RegistrationContext`,
not a runtime `Host`. Call `event.register_model(id)` to bake a file as-is. To build
a result from one or more files, call `event.register(id, dependencies, baker)`:

```rust,ignore
event.register_model("mymod:block/shaft");

event.register("mymod:shaft_east", ["mymod:block/shaft"], |bake| {
    bake.append_dependency("mymod:block/shaft", [0, 1, 0], true)
});

event.register(
    "mymod:machine",
    ["mymod:block/base", "mymod:block/overlay"],
    |bake| {
        bake.append_dependency("mymod:block/base", [0; 3], false)?;
        bake.append_dependency("mymod:block/overlay", [0; 3], false)
    },
);
```

The baker is `Send + Sync + 'static` and may run on a resource worker. Capture
owned inputs, not the event or its registration context. The
`StandaloneModelBakeContext` is borrowed only for that invocation and gives no
`Host`. Return `Ok(())` to publish the assembly or
`Err(StandaloneModelBakeError::Rejected(reason))` to leave it out.

The quarter-turn counts are negative turns, modulo four, applied X then Y then Z
about the block centre, exactly as in blockstate baking. UV locking uses the inverse
face transformation during baking; rotating a draw pose is not equivalent. For a Y
quarter-turn a north wall becomes east but stays upright either way; a north-pointing
arrow on the top turns east without UV locking and retains its world orientation with it.

Quads are appended in call order, retaining each part's positions, materials and quad
metadata in one model. The particle is the first available part particle, and model-level
ambient occlusion is enabled only if every appended part enables it. Declare all the model
files the callback may append. Dependencies load through the normal model library; they are
not automatically registered as results. As with blockstate and item models, a missing or rejected
parent uses the missing model at that link, and the child's own geometry still wins. This also
applies to files registered with `register_model`.
An undeclared append, a missing or rejected declared file, a bake error or a refused callback
discards the whole model and logs the registering mod. Ignoring an append error still discards it.

Result IDs and plain file IDs require a nonempty namespace and path, which need not equal
the registering mod's ID. A successfully baked plain file wins over any baker for that ID;
otherwise the first successful baker in registration order wins. Register once at
startup; eligible bakers run against current files at each resource load. Registration
does not expose earlier entries for editing. Resolve result IDs in
`model_baking_completed`, after atlas publication and at the model manager's apply step before any listener ordered
after it applies. Successful reloads invalidate old handles, while refused prepares retain them. The result is baked quads, not a mod-defined baked object.

## Colour, hidden faces and translucency of copies

`ModelInstance` carries two more fields, set with `with_color` and `with_hidden_faces`:

- `color` (`0xAARRGGBB`, default white) multiplies every face of the copy, tinted or not, after
  `tint`; its alpha multiplies every face's alpha. `tint` keeps its meaning (tinted faces only).
- `hidden_faces` (bits 0 to 5: down, up, north, south, west, east) leaves out the faces whose
  `cull` side is set, so a structure of many copies can drop the sides its neighbours cover.

A copy whose colour alpha is below `0xFF` draws every quad in the translucent material described below.
A model with translucent faces otherwise keeps its per-quad routing: only those faces take the translucent
material. Copies with a colour other than white, hidden faces, a model with translucent faces or a block state model are baked on the CPU
instead of drawn as GPU instances; GPU-routed copies past GPU buffer room also fall back to CPU baking.
`overlay` (the hurt tint) has no effect on a copy on the translucent or block state model route, including any
opaque faces it holds. `color` (the whole baked block model, multiplied at draw time) and `hidden_faces` (a
cull mask the caller supplies) are Sunrise's options for mod copies.

`block_state_model(state)` (on `RenderAssets`) resolves a `BlockState` as one model: the parts
its blockstate file selects for its properties, weighted alternatives at the fixed seed
`BlockEntityDrawContext::block(state)` uses (which likewise takes a resolved `BlockState`),
each tinted face in the block's default tint. A block vanilla also draws with a special
renderer (a chest, shulker box, banner, head, bell, decorated pot and the other blocks of
vanilla's `BuiltInBlockModels`) adds that renderer's geometry with no block entity data, as a
block display shows it: a closed chest, a plain banner. It draws through
`draw_model_instances`, instance groups and `draw.terrain_model`, not `draw.model`.
Resolve it once per state in `client_setup` and `client_resources_reloaded`.

## Terrain materials in a block entity

`draw.model(model, tint)` draws a resolved model as an entity model. A block entity, mob or
player submit hook that wants the terrain's own materials calls
`draw.terrain_model(model, options)` instead. That covers a machine's belts, shafts and
casings, cut out and shaded as blocks are:

```rust,ignore
let belt = TerrainUvShift::new("mymod:block/belt", "mymod:block/belt_scroll", [1.0, 0.5], [0.0, scroll]);
let options = TerrainModelOptions::default()
    .with_material(TerrainMaterial::Cutout)
    .with_uv_shift(belt);
draw.terrain_model(self.belt, options)?;
```

- Faces sample the animated block sprites (mipmapped, linear minification and nearest
  magnification, as vanilla samples a moving block), cull back faces and write colour and depth like
  opaque terrain. Light is the draw's own, raised by each quad's emission.
- `material`: `Model` keeps each quad's `render_type` (solid, `cutout`, `cutout_mipped`), and
  draws a translucent quad in the translucent material (below). `Solid`, `Cutout` (alpha under
  0.1 discarded) and `CutoutMipped` (under 0.5) choose one material for all quads.
- `shade`: `Model` follows each quad's `shade` flag, `Off` turns directional shading off, and
  `Entity` shades every quad by the level's entity light directions. Full-bright light never
  turns shading off.
- `uv_shift` draws faces of exactly one sprite with another, their UVs scaled then moved; if no
  face uses `source_sprite`, the shift is a no-op. `uv_shrink_ratio` (default `0.0`) pulls UVs
  towards each face's centre first, for a model baked with an older `FaceBakery`'s atlas shrink.
  The engine never guesses it.
- `tint` (`0xRRGGBB`) multiplies only quads with a tint index.
- `color` (`0xAARRGGBB`, default white) multiplies every quad's colour after its own and the
  tint, alpha included; white skips the multiply. An alpha below `0xFF` puts every quad in the
  translucent material.
- **Translucent material.** The one vanilla draws a translucent block with outside the terrain
  (`translucent_moving_block`): alpha blended, back faces culled, depth tested **and written**,
  fragments under alpha 0.1 discarded, the quad's directional shade baked into its colour. The
  renderer sorts the quads of all translucent models far to near each frame, as vanilla's
  `sortOnUpload` does, so submission order does not matter.
- A stale or unknown handle returns `ModelError` without appending geometry. Otherwise quads are
  submitted individually up to the device's vertex and index buffer capacity for the combined
  entity batch: missing sprites (including a missing
  shift target) fall back to `minecraft:missingno`, and quads with an unsupported render type or
  non-finite positions or UVs are skipped individually rather than refusing the whole model.

## Extra models in a block's mesh

A block whose look depends on data a block state can't hold (a cable showing each side's
channel count) gives the chunk mesher extra models to draw as part of it. On the client,
`host.set_block_extra_models(pos, &models)` sets the resolved standalone models drawn at `pos`:

```rust,ignore
let parts = [self.core, self.side_counts[count]];
host.set_block_extra_models(pos, &parts)?;
```

- The models are drawn after the block's own, unturned from its lower north-west corner (with its
  offset), in its light and tints. Each quad keeps its render layer (solid, cut out,
  translucent). A face with a cull direction is hidden by the neighbour there, as the block's
  own faces are. On air they draw nothing.
- A list replaces the position's last one; an empty list clears it. The section is meshed again
  only when the list changes, so set it when the data changes, not each frame.
- Native client mods set the list through the live client Host. `NotLoaded` means the position
  is outside a resident column or the level height; `Model` means the handle is stale or is not a
  registered standalone model. A refused call changes nothing.
- After the block geometry, extra faces are drawn in block and list order. Drawing stops before
  the next face whose worst-case expansion cannot fit the section's upload budget, so the last
  model may be partially drawn.
- The list is the client's own, never saved or sent. A resend of a resident column preserves
  attachments at unchanged cells; unloading the column clears them. A block state or level
  change also clears the affected lists. Set the list again from block entity data or the mod's
  own client map, on `client_block_changed`, `client_chunk_load` or a payload. No block entity
  is required.
- Mod tint providers are asked before the mesh is scheduled, not by the attachment pass on the
  worker. A provider that panics supplies white. A handle a resource reload made stale draws
  nothing until you set the lists again from the new handles.

The reload and setup contexts do not expose this setter. Resolve new handles in
`client_resources_reloaded`, then set the lists again from a live client callback such as
`client_tick_pre`:

```rust,ignore
fn client_resources_reloaded(&self, context: &dyn ClientReloadHost, _: &mut ClientResourcesReloadedEvent) {
    let mut parts = self.parts.lock().unwrap();
    parts.core = context.render_assets().model(&self.core_id).ok();
    parts.reapply = true;
}

fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
    let mut parts = self.parts.lock().unwrap();
    if std::mem::take(&mut parts.reapply) {
        for (&pos, cable) in &self.cables.lock().unwrap() {
            let _ = host.set_block_extra_models(pos, &parts.models_for(cable));
        }
    }
}
```

## Persistent groups

Group methods take either the live client host or the scoped client setup or published
reload context. The scoped contexts do not lend a world.

- `host.instance_groups().create_group(model, capacity)` reserves storage for `capacity` copies of a model.
  Initial drawn length is zero.
- `group.write(host, first, copies)` atomically overwrites a range and extends drawn
  length if needed. Unwritten slots start as white, full-bright identity copies at
  world origin. Invalid coordinates or light, or a range beyond capacity, change nothing.
- `group.set_len(host, len)` changes drawn length without changing reserved capacity.
  Shrinking to zero draws nothing; growing restores retained copies, including hidden
  slots written earlier.
- `group.set_group_transform(host, transform)` sets an identity-by-default row-major
  3×4 affine pose. For each local vertex the composition is exactly
  `world = origin + group_transform * (copy_transform * local)`.
  The world-space f64 origin is not rotated or scaled. Non-finite pose values change nothing.
- `draw.draw_instance_group(group)` explicitly submits the group's live copies for this
  view. Repeated submissions of the same group in one view each draw again. Culling uses
  the composed pose.
- `draw.instance_group(group)` draws a retained group under the draw context's current pose,
  in entity renderers, entity layers, block entities and screen scenes. A stale or destroyed
  group returns `GroupError::Stale` and draws nothing.
- `host.prepare_item(stack, display, seed)` resolves an item's model once and returns a
  `PreparedItemRef`; `draw.prepared_item(item)` then draws it as `draw.item` would, without
  resolving the model again. A stale handle draws nothing and returns `ModelError::StaleModel`.
- `group.destroy(host)` retires the handle and releases capacity and CPU caches.
  Copies already accepted for a view remain drawn; subsequent calls return `Stale`.
- `Hooks::prepare_render_buffers` runs once per world view before any pass-bearing render stage.
  If a classic pack owns the sky disc or celestial work, `RenderLevelStage::AfterSky` fires once
  after that work, following the pack's world import. Preparation
  lends `&WorldRenderView<'_>` (camera, world partial tick, game time, improved-transparency state,
  and visible `RenderableSection`s) and `&dyn ClientHost` so mods can update retained
  `InstanceGroups` before the render passes execute.
  To draw at a specific stage, prepare engine lines or quads with `event.context.lines(stage, &lines)`
  or `.quads(stage, &quads)`. Keep the returned `PreparedWorldDraw` for this view. At the matching
  `render_level_stage` callback, call `draw(batch)` on `event.context.world_pass()` or
  `event.context.oit_pass()` as appropriate. Draws run in call order, without uploading from the
  pass; handles cannot be used in a different view or stage. With a classic pack that owns feature
  drawing, the two feature stages follow the pack's own opaque and translucent groups. They run
  only if the pack's world color buffer can blend engine geometry. `AfterSky` and `AfterLevel` lend no
  pass, and stages skipped by the renderer are not replayed later.
  Drawing values (`WorldLine`, `WorldQuad`, `WorldDepth`, `NameTagOptions`, and `DrawError`)
  and stage selectors (`RenderLevelStage` and `OitStage`) are client-local. To synchronize a
  draw effect, send only its data in your own typed payload and choose the stage on the client.

Per-view draw storage grows dynamically without fixed per-view or per-mod copy caps,
and copies past GPU instance buffer room fall back to CPU baking. Culled copies are
omitted before staging. For transient `draw_model_instances` calls, copies are culled
against the frustum first and any individual visible copy with non-finite coordinates
or transforms, or light channels outside `0..=15`, is skipped without rolling back
valid copies in the same slice.

Groups cache converted records on the CPU. Unchanged groups avoid pose, normal and
tint conversion, but cull and stage visible copies each frame. Every visible
GPU-routed copy uploads each frame; groups do not retain a private GPU buffer.
Frequent whole-group pose changes rebuild every live record and can cost more than
uncached transient copies. Shader-pack selection, missing prepared GPU records and
overlays use the same CPU baking path as transient batches, including tint and light. Translucent
copies and copies of a block state model take the terrain model materials, not the
block-entity program a `shaders/` pack replaces.
There is no simulation-tick work.

A destroyed group, a group after resource reload or world close, or a group whose
creating mod was disabled returns `GroupError::Stale`. Re-enabling starts without
groups: recreate them from the newly published model handles. Slot reuse never revives
an old handle.

## Still fluids in a block entity

A block entity, mob or player submit hook draws a tank's contents with
`draw.fluid_box(&stack, bounds, options)`, where `bounds` is
`[x0, y0, z0, x1, y1, z1]` in blocks under the draw's current pose:

```rust,ignore
let options = FluidBoxOptions::new(false, 4.0 / 1024.0);
draw.fluid_box(&tank.stack, [0.125, 0.0625, 0.125, 0.875, level, 0.875], options);
```

- The stack's fluid is `minecraft:water`, `minecraft:lava` or any fluid with a look
  (`data/<mod>/sunrise/fluid/<name>.json`: `sprite`, `tint`; [fluid looks](../content-mods/fluid-container-models.md)
  holds the format). A stack with components is drawn like one without: they change neither the
  sprite nor the tint, so a mod colours a potion-like stack with `with_argb` from its own
  components. A registered fluid with no look or a missing sprite falls back to
  `minecraft:missingno`; an empty or unresolved fluid draws nothing.
- Each face tiles the fluid's animated still sprite (a look's `sprite`; a look with no sprite
  draws its tint as a flat colour) once per block, cut at whole-block lines.
  Each tile keeps its place in the sprite. The box is not inset. `uv_shrink_ratio` is the
  sprite's `TextureAtlasSprite.uvShrinkRatio`, and `render_bottom` adds the bottom face.
- The colour is the fluid's tint (water `0xFF3F76E4`, lava white, a mod fluid's look `tint`, grey
  `0xFF9A9A9A` when it has none, as `DrawContext::fluid` draws it), never the biome's water colour. `with_argb` replaces it, alpha
  included. Faces have no directional shading. Sky light is the draw's own, and block light
  rises to the fluid's `light_level` (lava 15, water 0, a mod fluid's block's).
- Each fluid's look is resolved once per resource load; a box does one indexed read.
- The faces use a material of their own: sorted far to near, alpha blended, culled from behind,
  and depth-writing. They draw with the block entities, before the level's translucent terrain,
  under shader packs too.
- A stack of 0, an empty or unresolved fluid, or reversed or non-finite bounds draws nothing.
  Valid quads are submitted individually up to the device's vertex and index buffer capacity for
  the combined entity batch without artificial coordinate or quad caps.

## Carried block entities

A mod that moves blocks (a contraption drawn from an entity renderer) draws the block entities
they carry with `draw.block_entity(&entity)`, vanilla's `BlockEntityRenderDispatcher` extract and
submit. `entity` is a `SceneBlockEntity`: the type id, the block state and the saved fields. Build
it once and again whenever its data changes; nothing ticks it.

```rust,ignore
draw.push();
draw.translate(offset);
draw.set_light(sampled_light); // the caller samples light itself
draw.block_entity(&carried)?;
draw.pop();
```

- The pose and light are `draw`'s; the overlay is vanilla's `NO_OVERLAY`. The block entity's own
  position is never read, and the dispatcher's `shouldRender` culling is skipped: the caller's
  entity was already culled.
- A vanilla block entity draws through the engine's renderer for its block state, a mod's through
  its `BlockEntityPart` submit. A type with no renderer draws nothing.
- Without a level a double chest draws as a single closed chest, an enchanting table as its
  closed book, a conduit inactive, an end portal and gateway on every face with no beam, and a
  beacon with no beam. Chest lids, the bell's shake, the enchanting book and conduit rotation stay
  at rest.
- One level of nesting: a `block_entity` call from inside a submit that `block_entity` itself
  dispatched returns `BlockEntityDrawRefusal::Nested`, because input a mod controls must never
  recurse without end.

## Before and after a living entity draws

Each mob and player in view gets the render hook pair once a frame, NeoForge's
`RenderLivingEvent` and `RenderPlayerEvent`:

1. `render_player_pre(player, who, draw, model, canceled)`, for a player only (`who` is its
   profile name and whether it is this window's own);
2. `render_living_pre(entity, draw, model, canceled)`, before the entity's model;
3. the model and its vanilla layers, or in their place the renderer a mod registered for its type
   ([entity renderers](entity-renderers.md)), then the layers mods added to its type
   ([entity layers](entity-layers.md));
4. `render_living_post(entity, model, draw)`;
5. `render_player_post(player, who, model, draw)`, for a player only.

This is the order within one entity. In the ordered body pass, the hooks run at each body's slot in
level join order, interleaved with general entity renderer submissions. The local body is not
submitted while the camera follows another entity (except in studio freecam). The `entity_outline`
queries for mobs, players and dropped items also follow this pass's join order; projectiles are
queried in a later pass. The ender dragon and end crystals draw at their slots without adding
these hooks or outline queries.

The model and layers draw before the post hooks. The entity's name tag, leash, flames and shadow draw
in later passes of the same frame, after the post hooks. A post hook cannot see or replace the flames
or shadow. A registered renderer draws in place of the model without changing the later passes.

`draw` starts at the entity's feet, turned with its body (+z the way it faces, +x its left, +y up,
in blocks), and keeps what each hook draws. Setting `event.cancelled` in a Pre hook skips the
entity's model (or its registered renderer), layers, name tag, leash and outline.
The flames of a burning entity and its shadow aren't part of its drawing: the frame queues them
after it, and a cancel leaves them. Canceling Player Pre skips all three remaining hooks.
Canceling only Living Pre skips Living Post but still calls Player Post with `model: None`.
A mod that replaces one entity's draw can submit its own model in Living Pre and cancel; to
replace every entity of a type, register a renderer instead. A culled entity gets neither pair.
The game checks once a frame whether any mod listens to the four hooks and builds nothing for
them if no mod does.

## Read the final body model

A registered layer and both Post hooks get `Option<&dyn FrameModel>` for the callback. When
present, it is the selected body's final pose after animation, with visibility after Skin
Customization and Pre edits. It includes nonhumanoids, custom resource-pack models and a posed
body that submitted only layers. `None` means no body pose was produced, for example when a
registered replacement renderer drew instead.

Use `model.part("head")` for the first exact depth-first name match, or start at `model.root()`
and follow `first_child` and `next_sibling` with `model.part_at(index)` to walk the model. Names
and links are resolved when the model is baked, but indices are local to this borrow: look up the
part again on a later draw. A missing name or out-of-range index returns `None`. `part.pose()`
gives a parent-relative `PartPose`: translation in texels, rotation in radians, and unitless scale.
When attaching geometry, follow `parent` links and compose those transforms from the root down.
Check `part.visible` to follow the body's visibility: it is false if the part or any ancestor is
hidden. True permits traversal but doesn't guarantee any cubes were drawn.

## Hide parts before drawing

`render_player_pre` and `render_living_pre` borrow the selected body as `event.model`, a
`&mut dyn FrameModelEdit`. It has the same hierarchy lookups as [the final model](#read-the-final-body-model)
but lends the bind pose, before animation and Skin Customization. Both Pre hooks share this
view for the draw: Living Pre sees the hides made in Player Pre.

Find a part by name (or follow the hierarchy) and pass its index to `hide`:

```rust,ignore
if let Some(index) = event.model.part("nose").map(|part| part.index) {
    event.model.hide(index);
}
```

This hides a villager's nose without replacing the model. Use `"head"` for the player's head or
`part.model_name()` when starting from a `HumanoidPart`. A missing name returns `None`; an invalid
index makes `hide` return `false` without changing anything. Neither refuses the draw.

- The visibility edits apply after animation and Skin Customization, for this draw only.
- A hidden part takes its children with it: the hat goes with the head, a sleeve with its arm.
- The body loses the part, and so do the layers drawn on the body's own model, as vanilla's
  `getParentModel()` layers do: a villager's type, profession and level, a spider's eyes. The
  armour, held items and other layers keep every part, and the cape, a layer of its own, stays.

## Pose a model after vanilla's last step

One living draw runs four phases, in order:

1. **Select** (`render_living_pre`): pick the body and hide parts.
2. **Animate** (`setupAnim`, with `humanoid_setup_anim` inside it): vanilla poses the model.
3. **Posed** (`living_model_posed`): the model after vanilla's last posing step.
4. **Drawn**: the layers and `render_living_post` read the final pose.

`living_model_posed(entity, layer, model)` fires once for each model that gets posed, the body and
each armour or equipment model that runs its own setup. `layer` is the model's `ModelLayerLocation`
(`minecraft:player#main`, `minecraft:zombie#chestplate`); a model the game names without a namespace reads as
`minecraft:`. A mod layer that redraws stock layers (`StockLayerDraw::draw_stock_layer`) poses the body and those
layers again, and the hook fires again for each, so keep an edit a function of its inputs. `model` is a `&mut dyn FrameModelPoseEdit`: find a part's index as in
[the final model](#read-the-final-body-model), then `set_pose(index, PartPose)` writes position,
rotation and scale, and `reset_pose(index)` returns the part to its bind pose. A non-finite write
returns `false` and the part keeps its pose. Hiding stays in Select. With no listener, no event is built.

## Text and animation in block entities

`BlockEntityDrawContext::text` draws a component under the current pose in font pixels,
with its styles and the frame's light. Its RGB colour is opaque. Glowing text uses full
light; `Some(0)` is a black outline, not an absent outline. `text` and `text_width`
report the styled advance, as vanilla `Font.width` does. Use half that width
to centre a line. `text` returns `TextFormError` without drawing if the component
has unrepresentable metadata; deep local text has no serialization depth cap.
Kept text retains its glyphs, including obfuscated glyph choices,
until it is drawn again. Resource reload invalidates kept geometry.

Spinning parts use the client's animation clock: level-running texture ticks plus an
animation phase, not the camera's partial tick or the server's day time. Pausing and
`/tick freeze` hold the animation where it was and resume without a jump; `/tick step`
advances it by whole ticks. The world's own render interpolation still uses 1 while
frozen. Camera motion and server time updates do not reset animation. Held-arm, hand,
mob and scene draws share this animation clock.

Directional shading on `quad` and `cube` follows `spin`, but light stays fixed by default.
To use the level's sky and block light, call `BlockEntityDrawContext::level_light` in a world
block-entity or entity renderer. Each element uses one level-light sample. The centre is the
unit-cell centre for `block`, `model` and `terrain_model`, the box midpoint for `cube`, or the
mean of the corners for `quad`, transformed by the pose and any applied `spin`. Baked emission
and other per-channel light floors still apply. Items and text keep the fixed light.

After `keep`, spun elements sample the cell they turn into each frame without a new submit.
Stationary elements keep their original sample until the draw is invalidated. If the centre
falls outside representable cell coordinates, the element uses its fixed light instead.
Level-less scenes, previews and mob model frames also stay fixed. `light()` always reports the
fixed setting; `set_light` returns subsequent draws to fixed light.

For a hologram, a glowing display or a projected item, call `BlockEntityDrawContext::unlit(true)`:
what `item`, `block`, `cube`, `quad`, `model` and `model_part` draw next skips directional face
shading and draws at full-bright light whatever `set_light` or `level_light` say. The lightmap's
top cell still applies, so night, `darkness` and coloured lights still tint it. An enchanted item
keeps its glint. It lasts until `unlit(false)` or the end of the draw; `terrain_model` keeps its
own shading option.

To draw a texture of your own (a dynamic texture a mod made), get a render type from
`RenderAssets::entity_solid`, `entity_cutout` or `entity_translucent` with the texture's id and call
`BlockEntityDrawContext::textured_quad(render_type, corners, uv, tint)`. Unlike `quad`, the UVs are
in the texture's own unit square (0 to 1), not texels of 16. The same texture gives the same render
type, so keep it. Light and `unlit` apply as for `quad`. The call returns false, drawing nothing,
for an id that is not a live dynamic texture, a first-person hand draw or a coordinate that is not
finite. A texture released after the quad was submitted draws the missing texture. Resource-stack
textures are not accepted. Under a shader pack these quads are drawn by the
pack's entity program (the block entity program in a block entity's draw) with the texture bound
as the entity texture.

## Copycat faces

Ask `block_sprites` for the material's side faces, then use each face's `texture` and
`quad_on(side, from, to)` to draw it on your box. The result supplies outward-wound
corners and a UV rectangle in texels, preserving the material's rotations and mirrors
and cropping to your box. It returns `None` for a slanted or sheared texture mapping
that cannot be represented by a rectangle; use another drawing route rather than
substituting an upright texture.

Multiply tinted faces by `block_tint_color(state, face.tint_index, pos)` at their world
position. Untinted faces return `None`. Both block and fluid faces use the mesher's current
biome colours, including updates from `refresh_biome_colors`. Water faces use tint index 0
and honour resource-pack water colormaps; other water indices return `None`. This differs
from `fluid_box`, whose colour comes from its fluid stack.
