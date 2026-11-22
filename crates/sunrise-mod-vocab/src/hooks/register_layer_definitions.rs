//! Entity model layer registration, baking and drawing.

pub trait Hooks {
    /// Register suppliers for entity model layers (NeoForge's
    /// `EntityRenderersEvent.RegisterLayerDefinitions`). Called once at client startup.
    /// Use [`RegisterLayerDefinitionsEvent::register`] to bind each layer ID to its supplier;
    /// accepted suppliers run once when the layer table is frozen. Bake the realized
    /// definitions through `ClientHost::bake_layer`. API @API:model_layers@.
    #[notes = r#"## Entity model layers

`Hooks::register_layer_definitions` is the client registration-phase counterpart of NeoForge 26.3's
`EntityRenderersEvent.RegisterLayerDefinitions`. It receives a `RegistrationContext` once at
client startup. Call `event.register(id, supplier)` to stage a supplier that builds a
`LayerDefinition`. Dedicated and integrated server roles do not fire this client event.
IDs have the form `<namespace>:<path>#<layer>` matching vanilla's `ModelLayerLocation`. Both
separators are required; namespace and path follow vanilla `Identifier` rules and may be empty.
The layer string is unrestricted, including empty. Malformed IDs are logged and dropped. An exact
collision with a vanilla layer key in `entity-models.json` fails client startup (matching vanilla
`LayerDefinitions.createRoots`). When multiple registrations target the same ID, the last
registration wins (matching NeoForge `ClientHooks.registerLayerDefinition`).

The game deduplicates accepted registrations (last-wins) and invokes each winning supplier once
when assembling the frozen layer table. Overwritten suppliers never run. A supplier that is
disabled, refuses or panics leaves no definition. Bakes and resource reloads do not call
suppliers again.

A `LayerDefinition` holds a `MeshDefinition` of named parts whose `CubeListBuilder` boxes sit
at a `PartPose`, on a texture of the given size. Texture offsets, mirroring and
`CubeDeformation` match vanilla's model builders. This listener is compiled in the API tests:

```rust,ignore
{{#include ../../../../crates/sunrise-client/tests/it/mod_api_hooks.rs:register_layer_definitions}}
```

`ClientHost::bake_layer` bakes a registered layer or a vanilla layer by its
`entity-models.json` name (`minecraft:zombie#main`) into an owned `ModelPart` tree. Bake once
and keep it. Before each frame, call `reset_all_poses` and set the parts' positions and
rotations as `setupAnim` does. A layer unavailable from the cached mob types is read from
resources for each bake, so do not bake it every frame.

Draw the tree with `BlockEntityDrawContext::model_part`, using the same draw context the mob
or block entity submit hook received. There is no separate Host drawing helper. Positions
are texels, +y down and the front towards -z; convert the mob's frame with scale
`[1.0, -1.0, -1.0]` then translation `[0.0, -1.501, 0.0]`, as `LivingEntityRenderer` does.
The sprite is the entity texture ID and the tint is `0xRRGGBB`."#]
    #[hook(
        receiver = "associated",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "layers",
        context = "registration",
        answered_by = "mod",
        registration_append = "custom",
        neoforge = "EntityRenderersEvent.RegisterLayerDefinitions",
        version = 4
    )]
    fn register_layer_definitions(layers: &mut Vec<Answered<LayerDefinitionRegistration>>);
}
