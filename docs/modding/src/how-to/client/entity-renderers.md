# Drawing an entity type with your own renderer

You can draw content-defined thrown entities, living entity types (mobs, vanilla's or a content
mod's), primed TNT, experience orbs, paintings, item frames and your own general entity types with
your own renderer. This is the counterpart of
NeoForge 26.3's `EntityRenderersEvent.RegisterRenderers` (`registerEntityRenderer`): the mod
registers one renderer per entity type at start, and the game hands each entity of that type in
view to it, once a frame.

## Write the renderer

A renderer implements `EntityRenderer`. Its `submit` gets the entity's frame and a draw context, the
same one a block entity or mob submit hook draws with (`quad`, `cube`, `block`, `item`,
`model_part`, `push`, `pop`, `translate`, `rotate_about`, `scale`):

```rust,ignore
use mod_api::*;
use std::sync::Arc;

struct Bolt;
impl EntityRenderer for Bolt {
    fn submit(
        &self,
        entity: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        // The pose starts at the entity, not turned: face the way it flies.
        draw.rotate_about([0.0, 1.0, 0.0], -entity.yaw, [0.0; 3]);
        draw.rotate_about([1.0, 0.0, 0.0], entity.pitch, [0.0; 3]);
        let face = Some(CubeFace::new("example:entity/bolt"));
        draw.cube([-1.0, -1.0, -4.0], [1.0, 1.0, 4.0], [face; 6], 0xFFFFFF);
        Ok(())
    }
}
```

| `EntityRenderState` | Meaning |
| --- | --- |
| `id` | The entity's network id, as mod messages name it. |
| `partial` | The frame's partial tick, 0 to 1. |
| `age` | Ticks lived plus the partial tick (`ageInTicks`). |
| `yaw`, `pitch` | The way it faces, in degrees, interpolated to the frame, except for primed TNT and experience orbs, which supply their current rotation (0 is south, positive pitch looks down). |
| `living` | A living entity's own frame (`LivingRenderState`, below); none for any other entity. |
| `tnt_fuse` | Primed TNT's remaining fuse, in ticks, as vanilla draws it (`fuse - partial + 1`); none for any other entity. |
| `orb_value` | An experience orb's value (`ExperienceOrb.DATA_VALUE`); none for any other entity. |
| `painting` | A painting's own frame (`PaintingRenderState`, below); none for any other entity. |
| `item_frame` | An item frame or glow item frame's own frame (`ItemFrameRenderState`, below); none for any other entity. |
| `attachments` | A general entity's frozen synced attachments (below); none for other entity families. |

The pose stack starts at the entity's interpolated position, in blocks, and is not turned. `draw`
also has the partial tick, the game time and the entity's light. What `submit` draws replaces the
type's own drawing; the glowing outline draws around it. Every entity of the type shares the
one renderer, so keep per-entity values out of it: read them from the state. `submit` has no
`Host`; a renderer that needs the world reads it in a hook and keeps what it read. A general entity
type's renderer is the exception (below). A frame is drawn afresh, so `draw.keep()` does nothing
here.

### A general entity's frame

A general entity type (`declare_general_entity`) has no vanilla look. Its render state carries the
row's identity, age, pose and a frozen snapshot of its synced attachments
([attachments](../../concepts/attachments.md)). A type declared with zero tracking chunks stays in
the server simulation, but its rows are not sent to clients for rendering.

```rust,ignore
static FUEL: AttachmentType<i32> = /* declared with sync: true */;

struct Missile;
impl EntityRenderer for Missile {
    fn submit(
        &self,
        entity: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        let fuel = match &entity.attachments {
            Some(snapshot) => snapshot.get(&FUEL).map_err(|error| format!("{error:?}"))?,
            None => None,
        };
        draw.translate([0.0, fuel.copied().unwrap_or_default() as f32, 0.0]);
        // Submit the missile's model at this pose.
        Ok(())
    }
}
```

A row is eligible for extraction immediately upon spawn (starting with an empty attachment
snapshot when the entity has no synced attachments). When the entity has synced attachments at
pairing time, the server bundles the initial attachment sync atomically with the entity spawn
packet (`ServerEntity.addPairing` / `ClientPacketListener.handleBundlePacket`, matching NeoForge
`AttachmentSync`), so the first rendered frame already sees them. This includes negotiated
clients held during a proxy handoff, which can see the level but are not yet visible to others.
Once paired, held viewers receive changes and removals for tracked entities and other players
without waiting for placement. The packet decoder validates the attachment message before
publishing the bundle; a malformed message discards the bundle without applying any of it.
Sync for an unknown entity, including an untracked foreign player, is logged and ignored, not
stored for a future spawn. The engine publishes a new generation at spawn and on each attachment
sync, and a frame keeps that generation even if a later update replaces or removes values.
Use a draw default when `get` returns `Ok(None)` for a
missing key. The first typed read of an undecoded binding can allocate to decode the value;
already-decoded bindings are shared directly without re-running the codec. Two snapshots compare
by key and canonical NBT value, not by their identity, compound iteration order, or decode cache.

`EntityRenderState`, its family-specific state types, and `EntityRenderAttachments` are native
render input, not `Wire` payloads. The attachment's save and sync codecs still apply. `yaw`,
`pitch` and `age` are interpolated to the frame. The pose stack starts at the row's interpolated
position.

### A living entity's frame

A living entity's state has `living: Some(LivingRenderState)`, NeoForge's `LivingEntityRenderState`
under the same names. Angles are in degrees, interpolated to the frame.

| `LivingRenderState` | Meaning |
| --- | --- |
| `body_rot` | The way the body faces (0 is south, 90 west). |
| `y_rot`, `x_rot` | The head's turn from the body and its pitch (both flipped when `is_upside_down`). |
| `death_time` | Ticks since it died plus the partial tick; 0 while alive. |
| `walk_animation_pos`, `walk_animation_speed` | The walk cycle: how far the legs have swung, and how fast (0 to 1); 0 riding or dead. |
| `scale`, `age_scale` | Its `minecraft:scale`, and 0.5 for a baby (else 1). |
| `is_upside_down`, `is_baby`, `is_in_water`, `has_red_overlay` | Named Dinnerbone or Grumm; a baby; in water; hurt or dying. |
| `pose` | Its stance (`Pose::Standing`, `Pose::Sleeping`, `Pose::Crouching`...). |
| `bed_orientation` | The way the bed it sleeps in faces, while it sleeps in one. |

### A painting's frame

A painting's state has `painting: Some(PaintingRenderState)`. `yaw` is
`Direction.get2DDataValue * 90` (0 is south, 90 west), and `pitch` is 0; they are the painting's
current facing, not interpolated (it has no previous rotation). `age` is the unsaved
`Entity.tickCount` plus the partial tick. The pose stack starts at the hanging entity's position
(the centre of its box), not turned.

| `PaintingRenderState` | Meaning |
| --- | --- |
| `facing` | The way it faces out from the wall (`Direction`: south, west, north, east). |
| `width`, `height` | Its size in blocks. |

The variant id (`minecraft:kebab`) is not in the state: a frame allocates nothing, so a renderer
that needs the variant reads it in a hook and keeps what it read.
### An item frame's frame

An item frame or glow item frame's state has `item_frame: Some(ItemFrameRenderState)`. `yaw` and
`pitch` follow `ItemFrame.setDirection`: `get2DDataValue * 90` in yaw on a wall (0 is south, 90
west), or `-90 * getAxisDirection().getStep()` in pitch on the floor and ceiling; they are the
frame's current facing, not interpolated (it has no previous rotation). `age` is the unsaved
`Entity.tickCount` plus the partial tick. The pose stack starts at the hanging entity's position
(the centre of its box), not turned, and not at the block centre vanilla's frame model is posed
from.

| `ItemFrameRenderState` | Meaning |
| --- | --- |
| `facing` | The face it hangs on (`Direction`: down, up, north, south, west, east). |
| `glow` | A glow item frame (`minecraft:glow_item_frame`). |
| `invisible` | The frame model is hidden; the item may still be drawn. |
| `rotation` | `ItemRotation`, in 45-degree steps (`-7..=7`). |
| `has_item`, `has_map` | Whether it holds an item, and whether that item is a filled map. |

The stack itself is not in the state: a frame allocates nothing, so a renderer that needs the item
reads it in a hook and keeps what it read. The renderer replaces the frame model and the item or
map on its face.

A living renderer replaces the type's whole renderer: its model and its vanilla layers (armour, held
items, a worn head), as a NeoForge `LivingEntityRenderer` does. Vanilla's turn, scale and death
tip-over aren't applied for it: the pose stack starts at the feet, not turned, so the renderer turns by
`body_rot` and scales by `scale` itself. Around it the entity keeps everything else:
`render_living_pre` (a cancel skips the renderer), the layers mods added to the type
([entity layers](entity-layers.md)), `render_living_post`, and the frame's later passes (its name tag,
its leash, and its flames and shadow), in the order the type's own model has ([model drawing](model-drawing.md)).
Living Pre borrows the selected vanilla or pack body in bind pose, independently of whether the
registered renderer runs. Its visibility edits apply only if that body draws. When the
replacement succeeds, no such pose was produced: registered layers still run in order and they
and Post receive `model: None`, not the unused bind pose. If the replacement fails, the regular
draw can still supply the final body pose.

## Register it

```rust,ignore
fn register_entity_renderers(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterEntityRenderersEvent<'_>,
) {
    event.renderers.register("example:bolt".into(), Arc::new(Bolt));
}
```

The hook is asked once at start on the client, before resources load. The game keeps the
registrations and resolves each type's renderer on every resource load, the first one and every
reload (F3+T, a resource pack change), so a frame dispatches by slot, with no lookup by name and no
scan over the mods.

- A later successful registration for a type replaces the earlier one silently, whichever mod made
  it, as in NeoForge, also for a vanilla type. A [failed registration callback](../../concepts/load-phases.md#registration-and-setup)
  does not replace the earlier renderer.
- A type the game does not know is logged with its id and left out; nothing panics.
- `submit` returns `Ok(())` to keep what it drew. A renderer that returns an `Err` or panics is logged once and asked no more until the next resource
  load (a reload resolves it again); what it drew that frame is dropped, and until then its type draws
  as it would without one.

## What draws through registered renderers

In the ordered body pass, general, living, primed TNT and experience orb renderers submit at
each entity's slot in level join order. The ender dragon and end crystals keep their own renderers
at their slots.

Registered renderers draw these routes:

- content-defined thrown types: a `data/<id>/sunrise/entity_type/<name>.json` using the
  `minecraft:snowball` template ([content mods](../content-mods.md)) draws the renderer's geometry
  instead of its flying item. Without a renderer, it draws its item;
- living types drawn as mobs, vanilla's (`minecraft:zombie`) and the content mods' living types: the
  renderer draws instead of the model and its vanilla layers;
- primed TNT (`minecraft:tnt`): the renderer draws instead of the flashing TNT cube, and reads
  `tnt_fuse`. `age` is the unsaved `Entity.tickCount` plus the partial tick. Yaw and pitch are
  the TNT's current rotation, not interpolated (it has no previous rotation). Without a
  renderer, the cube is unchanged.
- experience orbs (`minecraft:experience_orb`): the renderer draws instead of the billboarded orb
  quad, and reads `orb_value`. `age` is ticks lived plus the partial tick. The pose stack starts at
  the orb's interpolated position, not the 0.1-block lift vanilla's sprite uses. Without a
  renderer, the quad is unchanged.
- paintings (`minecraft:painting`): the renderer draws instead of the painting's quads, and reads
  `painting`. `age` is the unsaved `Entity.tickCount` plus the partial tick. Yaw is
  `Direction.get2DDataValue * 90` and pitch is 0, not interpolated (it has no previous rotation).
  Without a renderer, the quads are unchanged.
- your general entity types (`declare_general_entity`): the renderer is their only look, asked
  through `submit` with each visible row's frozen attachment snapshot. A general type with no
  renderer draws nothing, which is logged once a resource load, naming the type; a failed
  renderer draws nothing until the next resource load.
- item frames and glow item frames (`minecraft:item_frame`, `minecraft:glow_item_frame`): the
  renderer draws instead of the frame model and the item or map on its face, and reads
  `item_frame`. `age` is the unsaved `Entity.tickCount` plus the partial tick. Yaw and pitch
  follow `ItemFrame.setDirection` and are not interpolated (it has no previous rotation).
  Without a renderer, that geometry is unchanged.

Registrations for other known entity families, including boats, displays, vanilla projectiles, the
player (whose model goes by its skin), the armour stand and the ender dragon (which draw through
routes of their own), are retained without changing their drawing.

To draw over a living renderer instead of replacing it, add a layer
([entity layers](entity-layers.md)).

To draw an entity of any type in a screen, through its renderer, see [entities in screens](entities-in-screens.md).
