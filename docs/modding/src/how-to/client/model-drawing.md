# Drawing resolved models

Register standalone models during `standalone_models`, then resolve their `ModelRef`
through `RenderAssets::model` after resources are published. World render hooks receive
`WorldDrawContext`: submit transient copies with `draw_model_instances`, or retain
copies in a group. GUI scenes, mobs and kept/spinning renderers use their
CPU routes; groups are a world-drawing facility, not another renderer.

## Local player skins

Implement `Hooks::resolve_player_skin` to change the local player's appearance
without changing their profile. Upload the skin with
[`create_dynamic_texture`](dynamic-textures.md) as a 64×64 RGBA image once per
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
freed or wrongly sized texture keeps the original texture **and** model.
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

At client startup, implement `Hooks::register_layer_definitions` and call
`event.register("mymod:golem#main", create_golem_layer)`, where `create_golem_layer`
returns a `LayerDefinition` built from a `MeshDefinition` and the texture size.
The supplier is `Fn() -> LayerDefinition + Send + Sync + 'static`: capture owned
inputs, not the event or its registration context. Use a `<namespace>:<path>#<layer>`
ID matching vanilla's `ModelLayerLocation`. Include both separators even when a
component is empty. Namespace and path follow vanilla's `Identifier` rules, and the
layer can be any string. An exact collision with a vanilla layer key in
`entity-models.json` fails client startup.

When multiple registrations share an ID, the last registration wins and overwritten
suppliers never run. The game calls each winning supplier once when it freezes the
layer table, in registration order. The frozen table retains the definitions, not the
suppliers. Call `ClientHost::bake_layer("mymod:golem#main")` during renderer setup,
then keep the resulting `ModelPart` tree for posing and drawing. Baking again and
reloading resources reuse the definition.

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

Trees are API-owned `ModelPart` trees, built directly or baked from a
`LayerDefinition`. Register them once per resource load, never during a frame.
Model names belong to a namespace owned by the registering mod (its manifest
namespaces or a content package it serves via `served_by`). Duplicate model names,
non-finite geometry or poses, an unknown texture, an unknown item and two
providers for one item refuse the resource-load registration by name. A duplicate
item diagnostic names both providers. A completed callback's registration error
still refuses preparation even if it ignored the returned error. A
[failed registration callback](../../concepts/load-phases.md#registration-and-setup)
loses only its own models, textures and providers;
other callbacks can still register. Do not keep model or texture references
from a discarded registration. Disabling the owning mod stops its Part.
A server-only package cannot declare this client registration hook.

The worn stack must first pass the equippable component's asset and matching-slot
checks, and the asset must have layers. Only the visible equipped item's provider
is queried; random-texture inventory enumeration does not run frame callbacks.
Every callback takes `&dyn ClientHost` and `&ItemStack`:

1. `default_dye_color` runs once and returns packed ARGB, or zero when undyed.
2. `armor_layer_tint_color` runs in asset layer order, with the layer and its index.
   Exactly zero skips the layer, without calling its texture or model method.
   The default is white for a nondyeable layer, or its dye/optional undyed colour
   for a dyeable layer. Without either colour, a dyeable layer is skipped.
3. `armor_texture` receives the layer type, layer and the resolved default texture
   reference, including pack/CIT selection. A returned override is final for that
   layer; it is not replaced by another random-texture or custom-model texture.
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
returned during drawing uses vanilla's model or the supplied default texture,
with one diagnostic per provider until reload, never a panic. A failed callback
discards all of the piece's custom layers, draws the piece with vanilla behaviour,
and disables that provider until reload, with one diagnostic. A failure discards the custom drawing even if an earlier layer succeeded.
Callbacks work with references and layer values, not geometry or rebakes.

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
instead of drawn as GPU instances; the 16,384-copy budget bounds that cost.
`overlay` (the hurt tint) has no effect on a copy on the translucent or block state model route, including any
opaque faces it holds. `color` (the whole baked block model, multiplied at draw time) and `hidden_faces` (a
cull mask the caller supplies) are Sunrise's options for mod copies.

`block_state_model(state)` (on `RenderAssets`) resolves a block state as one model: the parts
its blockstate file selects for its properties, weighted alternatives at the fixed seed
`BlockEntityDrawContext::block` uses, each tinted face in the block's default tint. It draws
through `draw_model_instances`, instance groups and `draw.terrain_model`, not `draw.model`.
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
- `uv_shift` draws faces of exactly one sprite with another, their UVs scaled then moved.
  `uv_shrink_ratio` (`0.0..=1.0`, default `0.0`) pulls UVs towards each face's centre first,
  for a model baked with an older `FaceBakery`'s atlas shrink. The engine never guesses it.
- `tint` (`0xRRGGBB`) multiplies only quads with a tint index.
- `color` (`0xAARRGGBB`, default white) multiplies every quad's colour after its own and the
  tint, alpha included; white skips the multiply. An alpha below `0xFF` puts every quad in the
  translucent material.
- **Translucent material.** The one vanilla draws a translucent block with outside the terrain
  (`translucent_moving_block`): alpha blended, back faces culled, depth tested **and written**,
  fragments under alpha 0.1 discarded, the quad's directional shade baked into its colour. The
  renderer sorts the quads of all translucent models far to near each frame, as vanilla's
  `sortOnUpload` does, so submission order does not matter.
- A stale handle returns `TerrainModelError::Model(ModelError)`. If any quad can't be drawn, the result is
  `TerrainModelError::Refused` and nothing is drawn. That covers an unknown render type, a
  missing sprite, a pose that flattens the model, a ratio out of range, and a shift
  whose numbers aren't finite, whose sprites are unknown or whose source sprite no face uses.

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

- `host.create_group(model, capacity)` reserves up to 16,384 copies of a model.
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
- `draw.draw_instance_group(group)` draws the live length once per view. Repeated
  submissions in that view are ignored. Culling uses the composed pose.
- `group.destroy(host)` retires the handle and releases capacity and CPU caches.
  Copies already accepted for a view remain drawn; subsequent calls return `Stale`.

Each mod may reserve 65,536 copies across all its groups; a zero-capacity group
reserves one slot. Destroying a group releases that reservation immediately. A view
accepts 16,384 visible copies shared across transient batches, groups and listeners,
and at most 16,384 group submissions. Culled copies do not consume the visible-copy
budget. Refusal is atomic: no prefix of a refused submission draws.

Groups cache converted records on the CPU. Unchanged groups avoid pose, normal and
tint conversion, but cull and stage visible copies each frame. Every visible
GPU-routed copy uploads each frame; groups do not retain a private GPU buffer.
Frequent whole-group pose changes rebuild every live record and can cost more than
uncached transient copies. Shader-pack selection, missing prepared GPU records and
overlays use the same CPU baking path as transient batches, including tint and light. Translucent
copies and copies of a block state model take the terrain model materials, not the
block-entity program a `shaders/` pack replaces.
There is no simulation-tick work.

Only the creating mod can access a group. A foreign handle returns `GroupError::Stale`,
as does a destroyed group, a group after resource reload or world close, or a group
whose mod was disabled. Re-enabling starts without groups: recreate them from the
newly published model handles. Slot reuse never revives an old handle.

## Still fluids in a block entity

A block entity, mob or player submit hook draws a tank's contents with
`draw.fluid_box(&stack, bounds, options)`, where `bounds` is
`[x0, y0, z0, x1, y1, z1]` in blocks under the draw's current pose:

```rust,ignore
let options = FluidBoxOptions::new(false, 4.0 / 1024.0);
draw.fluid_box(&tank.stack, [0.125, 0.0625, 0.125, 0.875, level, 0.875], options)?;
```

- The stack's fluid is `minecraft:water`, `minecraft:lava` or any fluid with a look
  (`data/<mod>/sunrise/fluid/<name>.json`: `sprite`, `tint`; [fluid looks](../content-mods/fluid-container-models.md)
  holds the format). A stack with components is drawn like one without: they change neither the
  sprite nor the tint, so a mod colours a potion-like stack with `with_argb` from its own
  components. A registered fluid with no look is `FluidBoxError::MissingSprite`; an unresolved
  fluid and the empty fluid are `FluidBoxError::UnsupportedFluid`.
- Each face tiles the fluid's animated still sprite (a look's `sprite`; a look with no sprite
  draws its tint as a flat colour) once per block, cut at whole-block lines.
  Each tile keeps its place in the sprite. The box is not inset. `uv_shrink_ratio` is the
  sprite's `TextureAtlasSprite.uvShrinkRatio` (`0.0..=1.0`), and `render_bottom` adds the bottom face.
- The colour is the fluid's tint (water `0xFF3F76E4`, lava white, a mod fluid's look `tint`, grey
  `0xFF9A9A9A` when it has none, as `DrawContext::fluid` draws it), never the biome's water colour. `with_argb` replaces it, alpha
  included. Faces have no directional shading. Sky light is the draw's own, and block light
  rises to the fluid's `light_level` (lava 15, water 0, a mod fluid's block's).
- Each fluid's look is resolved once per resource load; a box does one indexed read.
- The faces use a material of their own: sorted far to near, alpha blended, culled from behind,
  and depth-writing. They draw with the block entities, before the level's translucent terrain,
  under shader packs too.
- A stack of 0 draws nothing and returns `Ok` without checking `bounds`. Non-finite or reversed
  bounds, bounds beyond 1,048,576 blocks, a box of more than 65,536 tiled quads and a ratio out of range are
  `FluidBoxError::InvalidBox`. Missing fluid sprites return
  `FluidBoxError::MissingSprite`. A refusal draws nothing.

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

The order today: the model and the layers are drawn before the post hooks. The entity's name tag, its leash,
and the flames of a burning entity and its shadow are each drawn in a later pass of the same frame, after the
post hooks. A post hook doesn't see or replace the flames and the shadow. The reference draws the leash and
the name tag before the post: that move is the follow-up in #1285. A type with a registered renderer has the
same later passes, the renderer in the model's place, so it changes with that follow-up too.

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
