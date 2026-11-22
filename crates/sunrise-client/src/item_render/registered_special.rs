//! Baking special model nodes at load and writing their layers atomically at draw time.
use super::{
    ItemDrawEnv, ItemRenderer, LayerGeometry, LayerRender, LayerTints, corners, glam_matrix,
    registered_providers::{
        ItemProviderTables, decode_item_model, decode_range_property, decode_select_property,
        decode_tint_source,
    },
    sprite_name,
};
use glam::{Mat4, Vec3};
use mod_api::{
    __private::{BakedSpecialModelRenderer, SpecialModelSubmission, game::CallbackOwner},
    ItemDisplayContext, ItemStack, SpecialModelBakeContext, SpecialModelCollector,
    SpecialModelConfig, SpecialModelFactory, SpecialModelGeometry, SpecialModelQuad,
    SpecialModelStack, SpecialModelTints,
};
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    ops::ControlFlow,
    sync::{Arc, OnceLock},
};
use sunrise_assets::{
    DecodeError,
    bake::{ModelQuad, facing},
    item_bake::{BakedItemModel, MISSING_SPRITE, SpriteLookup, bake_resolved},
    item_model::{
        ClientItem, Context, DisplayContext, FoilType, ItemModelDecoders, ItemModelLoader,
        LayerTransform, RangePropertyLoader, RegisteredSpecialModel, SelectPropertyLoader,
        SpecialModelLoader, StackView, TintSourceLoader, identifier,
    },
    math::Mat4 as AssetMatrix,
    model::{Direction, Material},
};

pub type SpecialModelFactories = BTreeMap<mod_api::Identifier, OwnedSpecialModelFactory>;

/// One winning type, with the owner of its factory and retained renderer callbacks.
pub struct OwnedSpecialModelFactory {
    factory: SpecialModelFactory,
    bake_owner: CallbackOwner,
    draw_owner: CallbackOwner,
}

impl OwnedSpecialModelFactory {
    pub fn new(id: &'static str, factory: SpecialModelFactory) -> Self {
        Self {
            factory,
            bake_owner: CallbackOwner::for_mod(id),
            draw_owner: CallbackOwner::for_mod(id),
        }
    }
}

pub(super) static FACTORIES: OnceLock<SpecialModelFactories> = OnceLock::new();

/// Install the winning factories before the first client resource load. A type may replace a
/// builtin.
pub fn install_special_model_factories(factories: SpecialModelFactories) -> bool {
    FACTORIES.set(factories).is_ok()
}

/// One baked `minecraft:special` node. The draw owner is shared by all bakes of the same
/// winning registration, including across resource reloads.
pub struct BakedSpecialModel {
    pub kind: Box<str>,
    renderer: BakedSpecialModelRenderer,
    base: SpecialLayerBase,
    owner: CallbackOwner,
}

/// The item geometry and extents retained by an API lease from this bake context.
pub struct BakedSpecialGeometry {
    pub model: Arc<BakedItemModel>,
    pub extents: Arc<Vec<Vec3>>,
    generation: Arc<()>,
}

impl BakedSpecialGeometry {
    fn lease(model: BakedItemModel, generation: &Arc<()>) -> SpecialModelGeometry {
        SpecialModelGeometry::new(Self {
            extents: Arc::new(corners(&model)),
            model: Arc::new(model),
            generation: Arc::clone(generation),
        })
    }
}

struct SpecialBaker<'a>(&'a ItemRenderer);

impl SpecialModelBakeContext for SpecialBaker<'_> {
    fn bake_model(&self, model: &str) -> Result<SpecialModelGeometry, String> {
        let model = self
            .0
            .library
            .resolve_required(model)
            .map_err(|error| error.to_string())?;
        let model = bake_resolved(&model, &self.0.sprites).map_err(|error| error.to_string())?;
        Ok(BakedSpecialGeometry::lease(
            model,
            &self.0.special_generation,
        ))
    }

    fn bake_geometry(&self, quads: &[SpecialModelQuad]) -> Result<SpecialModelGeometry, String> {
        let mut baked = Vec::with_capacity(quads.len());
        for (index, quad) in quads.iter().enumerate() {
            if quad.positions.iter().flatten().any(|v| !v.is_finite()) {
                return Err(format!("quad {index}: non-finite position"));
            }
            let facing = facing(&quad.positions).unwrap_or(Direction::Up);
            if quad.uv.iter().flatten().any(|v| !v.is_finite()) {
                return Err(format!("quad {index}: non-finite UV"));
            }
            if quad.light_emission > 15 {
                return Err(format!("quad {index}: light emission is over 15"));
            }
            let tint_index = quad
                .tint_index
                .map_or(Ok(-1), i32::try_from)
                .map_err(|_| format!("quad {index}: tint index is out of range"))?;
            let sprite =
                identifier(&quad.sprite).map_err(|error| format!("quad {index}: {error}"))?;
            let sprite = if sprite != MISSING_SPRITE && self.0.sprites.mask(&sprite).is_none() {
                log::warn!(
                    "Sunrise: special model quad {index} references missing sprite {sprite}; using missing texture"
                );
                MISSING_SPRITE.to_owned()
            } else {
                sprite
            };
            baked.push(ModelQuad {
                positions: quad.positions,
                uv: quad.uv,
                facing,
                cull: None,
                material: Material {
                    sprite,
                    force_translucent: quad.force_translucent,
                },
                tint_index,
                shade_direction_override: None,
                light_emission: quad.light_emission,
                light: [quad.light_emission, 0],
                color: None,
                ambient_occlusion: None,
                render_type: None,
                shade: true,
            });
        }
        let model = BakedItemModel {
            quads: baked,
            uses_block_light: false,
            particle: Material {
                sprite: MISSING_SPRITE.to_owned(),
                force_translucent: false,
            },
            transforms: Default::default(),
        };
        Ok(BakedSpecialGeometry::lease(
            model,
            &self.0.special_generation,
        ))
    }
}

impl ItemRenderer {
    /// Decode an item definition, binding its registered tint sources, item model types,
    /// range/select properties, and special model renderers against this renderer's selected
    /// models and sprites. Any failed decode or bake refuses the enclosing definition.
    pub fn decode_definition(
        &self,
        text: &str,
        providers: &ItemProviderTables<'_>,
    ) -> Result<ClientItem, DecodeError> {
        let special = |kind: &mod_api::Identifier, base: &str, fields: &Map<String, Value>| {
            let Some(factory) = providers.special.and_then(|factories| factories.get(kind)) else {
                return Ok(None);
            };
            // A disabled winner still owns this type. Only a missing registration
            // permits the built-in decoder to read the node.
            let renderer = factory
                .bake_owner
                .call("special model factory", "special_model_bake", || {
                    factory
                        .factory
                        .bake(SpecialModelConfig::new(fields), &SpecialBaker(self))
                })
                .and_then(|result| {
                    result.map_err(|error| format!("mod {}: {error}", factory.bake_owner.id()))
                })
                .map_err(|error| format!("special model {kind}: {error}"))?;
            let base = self.bake(base);
            let particle = Arc::from(sprite_name(&base.particle.sprite));
            Ok(Some(RegisteredSpecialModel(Arc::new(BakedSpecialModel {
                kind: kind.to_string().into(),
                renderer,
                base: SpecialLayerBase {
                    model: base,
                    particle,
                },
                owner: factory.draw_owner.clone(),
            }))))
        };
        let tint = |kind: &str, fields: &Map<String, Value>| {
            decode_tint_source(providers.tint, kind, fields)
        };
        let item_model =
            |kind: &str,
             fields: &Map<String, Value>,
             decode_child: &dyn Fn(
                &Value,
            ) -> Result<
                sunrise_assets::item_model::ItemModelNode,
                DecodeError,
            >| { decode_item_model(providers.item_model, kind, fields, decode_child) };
        let range_property = |kind: &str, fields: &Map<String, Value>| {
            decode_range_property(providers.range_property, kind, fields)
        };
        let select_property = |kind: &str, fields: &Map<String, Value>| {
            decode_select_property(providers.select_property, kind, fields)
        };
        let special_ref: &SpecialModelLoader<'_> = &special;
        let tint_ref: &TintSourceLoader<'_> = &tint;
        let item_model_ref: &ItemModelLoader<'_> = &item_model;
        let range_property_ref: &RangePropertyLoader<'_> = &range_property;
        let select_property_ref: &SelectPropertyLoader<'_> = &select_property;
        let decoders = ItemModelDecoders {
            special: providers.special.is_some().then_some(special_ref),
            tint: providers.tint.is_some().then_some(tint_ref),
            item_model: providers.item_model.is_some().then_some(item_model_ref),
            range_property: providers
                .range_property
                .is_some()
                .then_some(range_property_ref),
            select_property: providers
                .select_property
                .is_some()
                .then_some(select_property_ref),
        };
        ClientItem::from_json_with_decoders(text, &decoders)
    }

    /// Append one registered layer, with no committed writes until its callback succeeds.
    pub(super) fn append_registered_special(
        &self,
        layers: &mut Vec<LayerRender>,
        model: &RegisteredSpecialModel,
        (foil, outer_transform, display_transform): (FoilType, AssetMatrix, LayerTransform),
        ctx: &Context<'_>,
        env: &dyn ItemDrawEnv,
        ancestor: &StackAncestor<'_>,
    ) -> ControlFlow<()> {
        let Some(model) = model.0.downcast_ref::<BakedSpecialModel>() else {
            return ControlFlow::Break(());
        };
        if !model.owner.is_enabled() {
            return ControlFlow::Break(());
        }
        let report_refusal = |error: &str| {
            log::warn!(
                "mod {}: special model draw of {} using {} was refused: {error}",
                model.owner.id(),
                ancestor.stack.item,
                model.kind,
            );
        };
        let Some(input) = env.special_model_input(ancestor.stack, ctx) else {
            report_refusal("the special model input is unavailable");
            return ControlFlow::Break(());
        };
        let components = self.models.components_view(ancestor.stack);
        let read_component = |id: &str| components.get(id).map(crate::scoreboard::json_to_tag);
        let stack = SpecialModelStack::new(input.item, input.count, &read_component);
        let mut writer = LayerWriter {
            checkpoint: layers.len(),
            layers,
            renderer: self,
            base: &model.base,
            ctx,
            env,
            ancestor,
            foil,
            outer_transform,
            display_transform,
            failure: None,
            committed: false,
        };
        let result = model
            .owner
            .call("special model renderer", "special_model_draw", || {
                let mut pose = PoseWriter {
                    layer: &mut writer,
                    local_pose: AssetMatrix::IDENTITY,
                };
                model.renderer.submit(
                    input.host,
                    &stack,
                    &input.context,
                    &mut SpecialModelCollector::new(&mut pose),
                )
            });
        let Ok(result) = result else {
            // The owner already reported a panic or disabled state.
            return ControlFlow::Break(());
        };
        if let Err(error) = result
            && writer.failure.is_none()
        {
            writer.failure = Some(SubmissionFailure::Fault(error));
        }
        match writer.failure.as_ref() {
            Some(SubmissionFailure::Fault(error)) => {
                report_refusal(error);
                ControlFlow::Break(())
            }
            Some(SubmissionFailure::Nested) => ControlFlow::Break(()),
            None => {
                writer.committed = true;
                ControlFlow::Continue(())
            }
        }
    }
}

/// The base draw properties shared by all submissions for one baked node.
struct SpecialLayerBase {
    model: Arc<BakedItemModel>,
    particle: Arc<str>,
}

/// The active `(stack, display)` path, borrowed from the resolvers and their callers, with no
/// stack copies.
pub(super) struct StackAncestor<'a> {
    pub(super) stack: &'a StackView,
    pub(super) display: DisplayContext,
    pub(super) parent: Option<&'a StackAncestor<'a>>,
}

impl StackAncestor<'_> {
    fn contains(&self, stack: &StackView, display: DisplayContext) -> bool {
        std::iter::successors(Some(self), |entry| entry.parent)
            .any(|entry| entry.display == display && *entry.stack == *stack)
    }
}

enum SubmissionFailure {
    Nested,
    Fault(String),
}

/// Writes one special layer into the item state. Dropping without a commit rolls back every
/// write, including the geometry of any children already submitted.
struct LayerWriter<'out, 'draw> {
    layers: &'out mut Vec<LayerRender>,
    checkpoint: usize,
    renderer: &'draw ItemRenderer,
    base: &'draw SpecialLayerBase,
    ctx: &'draw Context<'draw>,
    env: &'draw dyn ItemDrawEnv,
    ancestor: &'draw StackAncestor<'draw>,
    foil: FoilType,
    outer_transform: AssetMatrix,
    display_transform: LayerTransform,
    failure: Option<SubmissionFailure>,
    committed: bool,
}

impl Drop for LayerWriter<'_, '_> {
    fn drop(&mut self) {
        if !self.committed {
            self.layers.truncate(self.checkpoint);
        }
    }
}

/// The same layer writer borrowed under one pose. Child scopes can't mutate their parent's pose.
struct PoseWriter<'scope, 'out, 'draw> {
    layer: &'scope mut LayerWriter<'out, 'draw>,
    local_pose: AssetMatrix,
}

impl PoseWriter<'_, '_, '_> {
    fn matrix(&self) -> Mat4 {
        // Under the display transform's half-block offset: poses are about the model's corner.
        let layer = &self.layer;
        glam_matrix(&layer.base.model.layer_matrix(
            layer.display_transform,
            layer.ctx.display,
            &layer.outer_transform.mul(&self.local_pose),
        ))
    }
}

impl SpecialModelSubmission for PoseWriter<'_, '_, '_> {
    fn with_pose(
        &mut self,
        [a, b, c, d, e, f, g, h, i, j, k, l]: [f32; 12],
        draw: &mut dyn FnMut(&mut SpecialModelCollector<'_>) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.layer.failure.is_some() {
            return Err("the special layer was already refused".into());
        }
        let pose =
            AssetMatrix::from_row_major([a, b, c, d, e, f, g, h, i, j, k, l, 0.0, 0.0, 0.0, 1.0]);
        let local_pose = self.local_pose.mul(&pose);
        if !local_pose.is_finite() {
            let error = "the composed special model pose is not finite";
            self.layer.failure = Some(SubmissionFailure::Fault(error.into()));
            return Err(error.into());
        }
        let mut child = PoseWriter {
            layer: &mut *self.layer,
            local_pose,
        };
        let result = draw(&mut SpecialModelCollector::new(&mut child));
        if let Err(error) = &result
            && self.layer.failure.is_none()
        {
            self.layer.failure = Some(SubmissionFailure::Fault(error.clone()));
        }
        result
    }

    fn submit_geometry(
        &mut self,
        geometry: &SpecialModelGeometry,
        tints: SpecialModelTints,
    ) -> Result<(), String> {
        if self.layer.failure.is_some() {
            return Err("the special layer was already refused".into());
        }
        let Some(geometry) = geometry
            .downcast_ref::<BakedSpecialGeometry>()
            .filter(|geometry| {
                Arc::ptr_eq(
                    &geometry.generation,
                    &self.layer.renderer.special_generation,
                )
            })
        else {
            let error = "the special model geometry was not baked for this resource generation";
            self.layer.failure = Some(SubmissionFailure::Fault(error.into()));
            return Err(error.into());
        };
        let matrix = self.matrix();
        self.layer.layers.push(LayerRender {
            geometry: LayerGeometry::Quads(geometry.model.clone()),
            tints: LayerTints::Special(tints),
            foil: self.layer.foil,
            matrix,
            uses_block_light: self.layer.base.model.uses_block_light,
            particle: self.layer.base.particle.clone(),
            extents: geometry.extents.clone(),
            display: self.layer.ctx.display,
        });
        Ok(())
    }

    fn submit_stack(
        &mut self,
        stack: &ItemStack,
        display: ItemDisplayContext,
    ) -> Result<(), String> {
        let layer = &mut *self.layer;
        if layer.failure.is_some() {
            return Err("the special layer was already refused".into());
        }
        // Leave room to resolve one child and unwind its renderer. A cyclic renderer can
        // make every key distinct, so the active path check alone cannot protect Rust's stack.
        if stacker::remaining_stack().is_some_and(|bytes| bytes < 64 * 1024) {
            let error = "too little native stack space to resolve a nested item model";
            layer.failure = Some(SubmissionFailure::Fault(error.into()));
            return Err(error.into());
        }
        let Some((stack, display)) = layer.env.special_model_stack(stack, display) else {
            layer.failure = Some(SubmissionFailure::Nested);
            return Err("the nested special model stack or display context is unreadable".into());
        };
        if layer.ancestor.contains(&stack, display) {
            log::warn!(
                "Sunrise: a special model renderer's nested stack of {} in {:?} is a cycle; the special layer draws the missing model",
                stack.item,
                display,
            );
            layer.failure = Some(SubmissionFailure::Nested);
            return Err("the nested special model stack is a cycle".into());
        }
        let ancestor = StackAncestor {
            stack: &stack,
            display,
            parent: Some(layer.ancestor),
        };
        let ctx = Context {
            display,
            ..*layer.ctx
        };
        let start = layer.layers.len();
        if layer
            .renderer
            .append_render_layers(layer.layers, &ancestor, &ctx, layer.env, None)
            .is_break()
        {
            layer.failure = Some(SubmissionFailure::Nested);
            return Err("the nested special model refused its draw".into());
        }
        // Compose after resolving the child: preserve parent * (child * grandchild) order
        // as well as the child's own display, foil and tint state.
        let matrix = self.matrix();
        for layer in &mut self.layer.layers[start..] {
            layer.matrix = matrix * layer.matrix;
        }
        Ok(())
    }
}
