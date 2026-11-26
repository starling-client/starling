//! The `living_model_posed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A living model has been posed (vanilla's RETURN of `EntityModel.setupAnim`; NeoForge has no
    /// event there): after every vanilla posing step, which for a humanoid includes arm priority,
    /// item use, the attack swing, crouch, bob and swim, and before the model draws. Create-style
    /// poses (a player hanging from a chain), emote and animation libraries, carry, sit and lie
    /// mods write here, where nothing vanilla runs afterwards.
    ///
    /// It fires once for each model that gets posed for a living draw: the body, and each armour
    /// or equipment model that runs its own `setupAnim` (`HumanoidArmorLayer`,
    /// `EquipmentLayerRenderer`), so a worn chestplate's model gets a call of its own. `entity` is
    /// the draw's render state. `layer` names the model by vanilla's `ModelLayerLocation`
    /// (`minecraft:player#main`, `minecraft:zombie#chestplate`; a mod-registered layer is
    /// `mymod:golem#main`), so a mod-registered layer or renderer is covered too; a model the
    /// game names without a namespace reads as `minecraft:`. A mod layer that redraws stock layers
    /// (`StockLayerDraw::draw_stock_layer`) poses the body and those layers again, and the hook
    /// fires again for each of them, so keep a listener's edit a function of its inputs. `model`
    /// lends the posed hierarchy as a [`crate::FrameModelPoseEdit`], addressed by the part indices
    /// [`crate::FrameModel::part`] resolves: `set_pose(index, pose)` writes position, rotation and
    /// scale, and `reset_pose(index)` returns a part to its bind pose. There is no `hide`: hide
    /// in `render_living_pre`, and use a zero scale here if a part must vanish mid-pose. A
    /// non-finite write is refused for that call and the part keeps its pose. Listeners run in
    /// dependency order and each sees the earlier edits; edits last for this draw, and the
    /// layers and `render_living_post` read the edited pose through their `FrameModel`.
    ///
    /// The phases of one living draw, in order: Select (`render_living_pre`: pick and hide),
    /// Animate (`setupAnim`, with `humanoid_setup_anim` inside it), Posed (this hook) and Drawn
    /// (layers and `render_living_post`, read-only).
    ///
    /// Client-side, with the loop `context` of the client and no `Host`: it fires inside the
    /// draw. With no listener there is no event and no extra pass.
    #[hook(
        side = "client",
        contract = "listen",
        context = "loop",
        neoforge = "none (vanilla EntityModel.setupAnim RETURN; NeoForge has no event there)",
        target = "sunrise_client_lib::mobs::render::native_living_model_posed"
    )]
    fn living_model_posed(
        entity: &EntityRenderState,
        layer: &str,
        model: &mut dyn FrameModelPoseEdit,
    );
}
