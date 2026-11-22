//! Reload-owned item providers and baked humanoid equipment trees.
use super::*;
use mod_api::{
    __private::game::discard_registration, ArmorModelRef, ArmorTextureRef, ClientItemExtensions,
    ClientItemExtensionsRegistrar, HookId, ItemKey, ModelPart,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use sunrise_client::{
    fast_hash::FxHashMap,
    mobs::{Part, Polygon},
};

pub(super) struct ItemExtensionsPart {
    pub attached: mod_api::Attached<dyn ClientItemExtensions>,
    pub failed: AtomicBool,
    pub reported_stale: AtomicBool,
}

pub(super) struct ArmorModel {
    pub name: String,
    pub slots: [Arc<Part>; 4],
}

pub(super) struct ArmorExtensions {
    pub generation: u64,
    pub models: Vec<ArmorModel>,
    pub textures: Vec<String>,
    pub texture_keys: FxHashMap<String, u32>,
    pub items: FxHashMap<ItemKey, ItemExtensionsPart>,
}
impl Default for ArmorExtensions {
    fn default() -> Self {
        static GENERATION: AtomicU64 = AtomicU64::new(1);
        Self::empty(GENERATION.fetch_add(1, Ordering::Relaxed))
    }
}
impl ArmorExtensions {
    fn empty(generation: u64) -> Self {
        Self {
            generation,
            models: Vec::new(),
            textures: Vec::new(),
            texture_keys: FxHashMap::default(),
            items: FxHashMap::default(),
        }
    }
}

pub(in crate::game) struct Registration<'a> {
    state: ArmorExtensions,
    published: Option<&'a ArmorExtensions>,
    sprite_exists: &'a dyn Fn(&str) -> bool,
    refused: Option<String>,
}
impl<'a> Registration<'a> {
    fn new(sprite_exists: &'a dyn Fn(&str) -> bool) -> Self {
        Self {
            state: ArmorExtensions::default(),
            published: None,
            sprite_exists,
            refused: None,
        }
    }

    fn record_refusal(&mut self, error: &str) {
        if let Some(owner) = mod_api::content::crash::current_native_mod() {
            log::warn!("mod {owner} hook register_client_item_extensions: {error}");
        }
        self.refused = Some(error.to_owned());
    }

    pub(in crate::game) fn register_listener(
        &mut self,
        listen: &mut dyn FnMut(&mut dyn ClientItemExtensionsRegistrar) -> bool,
    ) {
        let mut pending = Registration {
            state: ArmorExtensions::empty(self.state.generation),
            published: Some(&self.state),
            sprite_exists: self.sprite_exists,
            refused: None,
        };
        if listen(&mut pending) {
            let Registration { state, refused, .. } = pending;
            self.state.models.extend(state.models);
            self.state.textures.extend(state.textures);
            self.state.texture_keys.extend(state.texture_keys);
            self.state.items.extend(state.items);
            if let Some(error) = refused {
                self.refused = Some(error);
            }
        } else {
            for part in pending.state.items.into_values() {
                let owner = part.attached.mod_id();
                discard_registration(
                    (
                        HookId::RegisterClientItemExtensions,
                        "register_client_item_extensions",
                    ),
                    Some(owner),
                    part,
                );
            }
        }
    }
}
impl ClientItemExtensionsRegistrar for Registration<'_> {
    fn register_model(
        &mut self,
        name: &str,
        model: &ModelPart,
    ) -> std::result::Result<ArmorModelRef, String> {
        let result = (|| {
            let owner = mod_api::content::crash::current_native_mod()
                .ok_or_else(|| format!("armour model {name}: no registering provider"))?;
            let id = mod_api::Identifier::parse(name)
                .map_err(|why| format!("armour model {name}: {why}"))?;
            if id.namespace() == "minecraft"
                || !mod_api::__private::owns_namespace(owner, id.namespace())
            {
                return Err(format!("armour model {name}: expected namespace {owner}"));
            }
            if self
                .published
                .is_some_and(|state| state.models.iter().any(|model| model.name == name))
                || self.state.models.iter().any(|model| model.name == name)
            {
                return Err(format!("duplicate armour model {name}"));
            }
            let part =
                bake_model("root", model).map_err(|why| format!("armour model {name}: {why}"))?;
            let prefix = self.published.map_or(0, |state| state.models.len());
            let index = u32::try_from(prefix + self.state.models.len())
                .map_err(|_| format!("armour model {name}: model table exhausted"))?;
            let slots = [
                super::equipment_piece::ArmorSlot::Head,
                super::equipment_piece::ArmorSlot::Chest,
                super::equipment_piece::ArmorSlot::Legs,
                super::equipment_piece::ArmorSlot::Feet,
            ]
            .map(|slot| {
                let mut selected = part.clone();
                hide_other_slots(&mut selected, slot);
                Arc::new(selected)
            });
            self.state.models.push(ArmorModel {
                name: name.to_owned(),
                slots,
            });
            Ok(ArmorModelRef::__new(self.state.generation, index))
        })();
        if let Err(error) = &result {
            self.record_refusal(error);
        }
        result
    }
    fn register_texture(&mut self, name: &str) -> std::result::Result<ArmorTextureRef, String> {
        let result = if !(self.sprite_exists)(name) {
            Err(format!("unknown armour texture {name}"))
        } else if let Some(&index) = self
            .published
            .and_then(|state| state.texture_keys.get(name))
            .or_else(|| self.state.texture_keys.get(name))
        {
            Ok(ArmorTextureRef::__new(self.state.generation, index))
        } else {
            let prefix = self.published.map_or(0, |state| state.textures.len());
            u32::try_from(prefix + self.state.textures.len())
                .map_err(|_| format!("armour texture {name}: texture table exhausted"))
                .map(|index| {
                    self.state.textures.push(name.to_owned());
                    self.state.texture_keys.insert(name.to_owned(), index);
                    ArmorTextureRef::__new(self.state.generation, index)
                })
        };
        if let Err(error) = &result {
            self.record_refusal(error);
        }
        result
    }
    fn client_item_extensions(
        &mut self,
        item: ItemKey,
        provider: Arc<dyn ClientItemExtensions>,
    ) -> std::result::Result<(), String> {
        let result = (|| {
            let owner = mod_api::content::crash::current_native_mod()
                .ok_or_else(|| "client item extensions: no registering provider".to_owned())?;
            let attached = mod_api::Attached::__client_item_extensions(provider)
                .ok_or_else(|| format!("unknown item extension provider {owner}"))?;
            let name = mod_api::__private::item_key_id(item)
                .ok_or_else(|| format!("unknown item for extension provider {owner}"))?;
            if let Some(previous) = self
                .published
                .and_then(|state| state.items.get(&item))
                .or_else(|| self.state.items.get(&item))
            {
                return Err(format!(
                    "duplicate client item extensions for {name}: {} and {owner}",
                    previous.attached.mod_id()
                ));
            }
            self.state.items.insert(
                item,
                ItemExtensionsPart {
                    attached,
                    failed: AtomicBool::new(false),
                    reported_stale: AtomicBool::new(false),
                },
            );
            Ok(())
        })();
        if let Err(error) = &result {
            self.record_refusal(error);
        }
        result
    }
}

// Bounds the recursive bake and subsequent clones and traversal of the baked tree.
const MAX_ARMOR_MODEL_DEPTH: usize = 32;

// All unknown geometry is validated once while baking, never during a frame.
fn bake_model(name: &str, model: &ModelPart) -> std::result::Result<Part, &'static str> {
    fn bake(
        name: &str,
        model: &ModelPart,
        depth: usize,
    ) -> std::result::Result<Part, &'static str> {
        if depth == 0 {
            return Err("model has too many nested parts");
        }
        let pose = [
            model.x,
            model.y,
            model.z,
            model.x_rot,
            model.y_rot,
            model.z_rot,
            model.x_scale,
            model.y_scale,
            model.z_scale,
        ];
        if !pose.iter().all(|x| x.is_finite()) {
            return Err("non-finite model pose");
        }
        let mut polygons = Vec::new();
        for polygon in &model.polygons {
            if !polygon.normal.iter().all(|x| x.is_finite())
                || !polygon
                    .vertices
                    .iter()
                    .all(|v| v.pos.iter().chain([&v.u, &v.v]).all(|x| x.is_finite()))
            {
                return Err("non-finite model geometry");
            }
            if !model.skip_draw && model.visible {
                polygons.push(Polygon {
                    normal: polygon.normal,
                    vertices: polygon
                        .vertices
                        .map(|v| [v.pos[0], v.pos[1], v.pos[2], v.u, v.v]),
                    cull_back: false,
                });
            }
        }
        let children = if model.visible {
            model
                .children()
                .map(|(name, child)| bake(name, child, depth - 1))
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        Ok(Part {
            name: name.to_owned(),
            pose,
            polygons,
            children,
        })
    }
    bake(name, model, MAX_ARMOR_MODEL_DEPTH)
}

impl Game<'_> {
    pub(super) fn prepare_client_item_extensions(
        gui: &UiResources,
    ) -> std::result::Result<ArmorExtensions, String> {
        use mod_api::__private::game::RegistrationContextNew;
        let sprite_exists = |name: &str| gui.atlas.sprites.contains_key(name);
        let mut registration = Registration::new(&sprite_exists);
        // Prebind every default texture once, so frame callbacks receive only integer references.
        for name in gui.atlas.sprites.keys() {
            let _ = registration.register_texture(name);
        }
        Self::native_register_client_item_extensions(
            &mod_api::RegistrationContext::new(),
            &mut registration,
        );
        if let Some(why) = registration.refused {
            log::warn!("Sunrise: client item extensions refused: {why}");
            return Err(why);
        }
        Ok(registration.state)
    }
}

fn hide_other_slots(part: &mut Part, slot: super::equipment_piece::ArmorSlot) {
    if !slot.shows(&part.name) {
        part.polygons.clear();
        part.children.clear();
        return;
    }
    for child in &mut part.children {
        hide_other_slots(child, slot);
    }
}

impl Game<'_> {
    pub(super) fn extended_equipment_materials<'a>(
        &'a self,
        piece: &super::equipment_piece::EquipmentPiece<'_>,
        stack: &sunrise_client::items::ItemStack,
        trim: Option<super::equipment_piece::MaterialBinding<'a>>,
        mut texture: impl FnMut(&str) -> Option<super::equipment_piece::MaterialBinding<'a>>,
        mut emissive: impl FnMut(&str) -> Option<super::equipment_piece::MaterialBinding<'a>>,
    ) -> Option<Vec<super::equipment_piece::EquipmentMaterial<'a>>> {
        use super::api_values::TryIntoApi;
        let state = &self.armor_stand_assets.extensions;
        if state.items.is_empty() {
            return None;
        }
        let item = mod_api::__private::item_key(&stack.item)?;
        let part = state.items.get(&item)?;
        if !part.attached.enabled() || part.failed.load(Ordering::Relaxed) {
            return None;
        }
        let refused = |why: &str| {
            if !part.failed.swap(true, Ordering::Relaxed) {
                log::warn!(
                    "Sunrise: client item extensions for {} refused: {why}",
                    part.attached.mod_id()
                );
            }
        };
        let stack = match stack.try_into_api() {
            Ok(stack) => stack,
            Err(_) => {
                refused("invalid equipped stack");
                return None;
            }
        };
        let bind = |name: &str| {
            self.gui.atlas.sprites.get_key_value(name).map(|(name, r)| {
                super::equipment_piece::MaterialBinding {
                    name,
                    uv: [r.u0, r.v0, r.u1, r.v1],
                }
            })
        };
        let materials = dispatch_equipment(
            self,
            state,
            part,
            &stack,
            piece,
            EquipmentBindings {
                trim,
                default: &mut texture,
                binding: &bind,
                emissive: &mut emissive,
            },
        );
        // A refusal discards the whole piece and selects the vanilla draw this frame.
        (!part.failed.load(Ordering::Relaxed)).then_some(materials)
    }
}

type Binding<'a> = super::equipment_piece::MaterialBinding<'a>;
struct EquipmentBindings<'a, 'call> {
    trim: Option<Binding<'a>>,
    default: &'call mut dyn FnMut(&str) -> Option<Binding<'a>>,
    binding: &'call dyn Fn(&str) -> Option<Binding<'a>>,
    emissive: &'call mut dyn FnMut(&str) -> Option<Binding<'a>>,
}
fn dispatch_equipment<'a>(
    host: &dyn mod_api::ClientHost,
    state: &'a ArmorExtensions,
    part: &ItemExtensionsPart,
    stack: &mod_api::ItemStack,
    piece: &super::equipment_piece::EquipmentPiece<'_>,
    lookups: EquipmentBindings<'a, '_>,
) -> Vec<super::equipment_piece::EquipmentMaterial<'a>> {
    if part.failed.load(Ordering::Relaxed) {
        return Vec::new();
    }
    use super::equipment_piece::ResolvedEquipmentLayer;
    use mod_api::{EquipmentLayer, EquipmentLayerType};
    let EquipmentBindings {
        trim,
        default: texture,
        binding: bind,
        emissive,
    } = lookups;
    let refused = |why: &str| {
        if !part.failed.swap(true, Ordering::Relaxed) {
            log::warn!(
                "Sunrise: client item extensions for {} refused: {why}",
                part.attached.mod_id()
            );
        }
    };
    let provider = &**part.attached.part();
    let report_stale = || {
        if !part.reported_stale.swap(true, Ordering::Relaxed) {
            log::warn!(
                "Sunrise: stale armour model or texture reference from {}; using the vanilla binding",
                part.attached.mod_id()
            );
        }
    };
    let _guard = mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
        id: part.attached.mod_id(),
        hook: "client_item_extensions",
        part: Some("ClientItemExtensions"),
    });
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let dye = provider.default_dye_color(host, stack);
        let layer_type = match piece.kind {
            "humanoid_baby" => EquipmentLayerType::HumanoidBaby,
            "humanoid_leggings" => EquipmentLayerType::HumanoidLeggings,
            _ => EquipmentLayerType::Humanoid,
        };
        piece.resolved_materials(
            trim,
            |index, layer| {
                let original_ref = state
                    .texture_keys
                    .get(&layer.texture)
                    .map_or(ArmorTextureRef::__new(0, 0), |&index| {
                        ArmorTextureRef::__new(state.generation, index)
                    });
                let snapshot = EquipmentLayer::__new(
                    original_ref,
                    layer.dyeable,
                    layer.color_when_undyed,
                    layer.use_player_texture,
                );
                let color =
                    provider.armor_layer_tint_color(host, stack, &snapshot, index as u32, dye);
                if color == 0 {
                    return None;
                }
                let default = texture(&layer.texture);
                let default_ref = default
                    .and_then(|b| state.texture_keys.get(b.name))
                    .map_or(ArmorTextureRef::__new(0, 0), |&index| {
                        ArmorTextureRef::__new(state.generation, index)
                    });
                let selected =
                    provider.armor_texture(host, stack, layer_type, &snapshot, default_ref);
                let mut selected_texture_final = selected != default_ref;
                let binding = if selected == default_ref {
                    default
                } else {
                    let (generation, index) = selected.__parts();
                    let name = (generation == state.generation)
                        .then(|| state.textures.get(index as usize))
                        .flatten();
                    if let Some(name) = name {
                        bind(name)
                    } else {
                        report_stale();
                        selected_texture_final = false;
                        default
                    }
                }?;
                let selected =
                    provider.armor_layer_model(host, stack, layer_type, &snapshot, index as u32);
                let model = if selected == ArmorModelRef::VANILLA {
                    None
                } else {
                    let (generation, index) = selected.__parts();
                    let model = (generation == state.generation)
                        .then(|| state.models.get(index as usize))
                        .flatten();
                    if let Some(model) = model {
                        Some(&model.slots[piece.slot.index()])
                    } else {
                        report_stale();
                        None
                    }
                };
                let color = (color & 0xFF00_FF00)
                    | ((color & 0x00FF_0000) >> 16)
                    | ((color & 0x0000_00FF) << 16);
                Some(ResolvedEquipmentLayer {
                    binding,
                    color,
                    model,
                    final_texture: selected_texture_final,
                })
            },
            emissive,
        )
    }));
    match result {
        Ok(materials) => materials,
        Err(_) => {
            refused("callback failed");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::equipment_piece::{ArmorSlot, EquipmentAssetLayer, EquipmentPiece};
    use super::*;
    use mod_api::{
        ClientHost, EquipmentLayer, EquipmentLayerType, ModelPolygon, ModelVertex, PartPose,
    };
    struct Empty;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Empty {}
    fn owner(id: &'static str) -> mod_api::content::crash::ModGuard {
        mod_api::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
            id,
            hook: "register_client_item_extensions",
            part: None,
        })
    }
    fn tree(x: f32) -> ModelPart {
        let face = ModelPolygon::new(
            [
                ModelVertex::new([x, 0.0, 0.0], 0.0, 0.0),
                ModelVertex::new([x + 1.0, 0.0, 0.0], 1.0, 0.0),
                ModelVertex::new([x + 1.0, 1.0, 0.0], 1.0, 1.0),
                ModelVertex::new([x, 1.0, 0.0], 0.0, 1.0),
            ],
            [0.0, 0.0, 1.0],
        );
        ModelPart::new(
            Vec::new(),
            vec![(
                "body".into(),
                ModelPart::new(vec![face], Vec::new(), PartPose::ZERO),
            )],
            PartPose::ZERO,
        )
    }
    #[test]
    fn an_armour_model_tree_too_deep_to_bake_is_refused() {
        let mut model = tree(0.0);
        for _ in 0..MAX_ARMOR_MODEL_DEPTH {
            model = ModelPart::new(Vec::new(), vec![("child".into(), model)], PartPose::ZERO);
        }
        assert_eq!(
            bake_model("root", &model).err(),
            Some("model has too many nested parts")
        );
    }

    #[test]
    fn a_panicking_equipment_registrar_leaves_the_next_batch_usable() {
        use mod_api::__private::game::RegistrationContextNew;
        use mod_api::{Hooks, ModEntry, RegisterClientItemExtensionsEvent, RegistrationContext};
        use std::sync::Mutex;

        struct Provider;
        impl ClientItemExtensions for Provider {}

        type Refs = (ArmorModelRef, ArmorTextureRef, ArmorTextureRef);
        struct Registrant {
            model: &'static str,
            refs: Arc<Mutex<Vec<Refs>>>,
            panics: bool,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Registrant {
            fn register_client_item_extensions(
                &self,
                _: &RegistrationContext,
                event: &mut RegisterClientItemExtensionsEvent<'_>,
            ) {
                let extensions = &mut *event.extensions;
                if self.panics {
                    extensions.register_texture("orphan").unwrap();
                }
                let shared = extensions.register_texture("shared").unwrap();
                assert_eq!(extensions.register_texture("shared").unwrap(), shared);
                let other = extensions
                    .register_texture(if self.panics { "failed" } else { "clear" })
                    .unwrap();
                let model = extensions.register_model(self.model, &tree(4.0)).unwrap();
                let item = mod_api::__private::item_key("test:gear").unwrap();
                extensions
                    .client_item_extensions(item, Arc::new(Provider))
                    .unwrap();
                if self.panics {
                    assert_eq!(
                        extensions.register_texture("missing"),
                        Err("unknown armour texture missing".into())
                    );
                }
                self.refs.lock().unwrap().push((model, shared, other));
                if self.panics {
                    panic!("after staging an equipment model, textures and provider");
                }
            }
        }

        let failed_refs = Arc::new(Mutex::new(Vec::new()));
        let healthy_refs = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![
                ModEntry::new(
                    "failed",
                    Box::new(Registrant {
                        model: "failed:shape",
                        refs: failed_refs.clone(),
                        panics: true,
                    }),
                ),
                ModEntry::new(
                    "healthy",
                    Box::new(Registrant {
                        model: "healthy:shape",
                        refs: healthy_refs.clone(),
                        panics: false,
                    }),
                ),
            ]
            .into(),
            || {
                let mut host = mod_api::testing::FakeHost::new();
                host.registries
                    .resolve_items([mod_api::Identifier::parse("test:gear").unwrap()])
                    .unwrap();
                let mut registration = Registration::new(&|name| name != "missing");
                let generation = registration.state.generation;
                assert_eq!(
                    registration.register_texture("original").unwrap().__parts(),
                    (generation, 0)
                );
                {
                    let _owner = owner("prefix");
                    assert_eq!(
                        registration
                            .register_model("prefix:shape", &tree(0.0))
                            .unwrap()
                            .__parts(),
                        (generation, 0)
                    );
                }
                Game::native_register_client_item_extensions(
                    &RegistrationContext::new(),
                    &mut registration,
                );
                assert!(
                    failed_refs.lock().unwrap().len() == 1,
                    "the failed listener completed all staged edits before panicking"
                );
                assert!(registration.refused.is_none());
                let [(model, shared, clear)] = healthy_refs.lock().unwrap()[..] else {
                    panic!("the healthy listener registered its bindings exactly once");
                };
                assert_eq!(model.__parts(), (generation, 1));
                assert_eq!(shared.__parts(), (generation, 1));
                assert_eq!(clear.__parts(), (generation, 2));
                let state = registration.state;
                assert_eq!(
                    state
                        .models
                        .iter()
                        .map(|model| model.name.as_str())
                        .collect::<Vec<_>>(),
                    ["prefix:shape", "healthy:shape"]
                );
                assert_eq!(state.textures, ["original", "shared", "clear"]);
                assert_eq!(state.texture_keys.len(), 3);
                assert_eq!(state.texture_keys["shared"], 1);
                assert_eq!(state.texture_keys["clear"], 2);
                assert_eq!(state.items.len(), 1);
                let item = mod_api::__private::item_key("test:gear").unwrap();
                assert_eq!(state.items[&item].attached.mod_id(), "healthy");
                assert_eq!(
                    mod_api::__private::take_hook_panics(),
                    [mod_api::__private::HookPanicReport {
                        mod_id: "failed",
                        hook: "register_client_item_extensions",
                    }]
                );
                let mut next = Registration::new(&|name| name != "missing");
                Game::native_register_client_item_extensions(
                    &RegistrationContext::new(),
                    &mut next,
                );
                assert_eq!(failed_refs.lock().unwrap().len(), 1);
                assert_eq!(healthy_refs.lock().unwrap().len(), 2);
                assert_eq!(next.state.items[&item].attached.mod_id(), "healthy");
                assert!(mod_api::__private::take_hook_panics().is_empty());
            },
        );
    }

    struct TwoShapes {
        models: [ArmorModelRef; 2],
        textures: [ArmorTextureRef; 2],
        original_first: ArmorTextureRef,
        calls: Arc<std::sync::Mutex<Vec<String>>>,
    }
    impl ClientItemExtensions for TwoShapes {
        fn default_dye_color(&self, _: &dyn ClientHost, _: &mod_api::ItemStack) -> u32 {
            self.calls.lock().unwrap().push("dye".into());
            0
        }
        fn armor_layer_tint_color(
            &self,
            _: &dyn ClientHost,
            _: &mod_api::ItemStack,
            _: &EquipmentLayer,
            index: u32,
            _: u32,
        ) -> u32 {
            self.calls.lock().unwrap().push(format!("tint{index}"));
            match index {
                0 => 0xFF12_3456,
                1 => 0xFFFF_FFFF,
                _ => 0,
            }
        }
        fn armor_texture(
            &self,
            _: &dyn ClientHost,
            _: &mod_api::ItemStack,
            _: EquipmentLayerType,
            layer: &EquipmentLayer,
            _: ArmorTextureRef,
        ) -> ArmorTextureRef {
            let index = usize::from(layer.texture != self.original_first);
            self.calls.lock().unwrap().push(format!("texture{index}"));
            self.textures[index]
        }
        fn armor_layer_model(
            &self,
            _: &dyn ClientHost,
            _: &mod_api::ItemStack,
            _: EquipmentLayerType,
            _: &EquipmentLayer,
            index: u32,
        ) -> ArmorModelRef {
            self.calls.lock().unwrap().push(format!("model{index}"));
            self.models[index as usize]
        }
    }
    #[test]
    fn two_shapes_two_textures_tint_skip_and_one_trim_use_the_real_dispatch_boundary() {
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("armor_test", Box::new(Empty))].into(),
            || {
                use mod_api::prelude::ItemsHost;
                let mut host = mod_api::testing::FakeHost::new();
                let id = mod_api::Identifier::parse("armor_test:chest").unwrap();
                host.registries.resolve_items([id.clone()]).unwrap();
                let item = host.item_key(&id).unwrap();
                let stack = mod_api::ItemStack::of(item);
                let mut registry = Registration::new(&|_| true);
                let _owner = owner("armor_test");
                let models = [
                    registry
                        .register_model("armor_test:inner", &tree(0.0))
                        .unwrap(),
                    registry
                        .register_model("armor_test:outer", &tree(16.0))
                        .unwrap(),
                ];
                let original_first = registry.register_texture("inner").unwrap();
                registry.register_texture("outer").unwrap();
                let textures = [
                    registry.register_texture("custom_inner").unwrap(),
                    registry.register_texture("custom_outer").unwrap(),
                ];
                let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
                registry
                    .client_item_extensions(
                        item,
                        Arc::new(TwoShapes {
                            models,
                            textures,
                            original_first,
                            calls: calls.clone(),
                        }),
                    )
                    .unwrap();
                let layers = [
                    EquipmentAssetLayer {
                        texture: "inner".into(),
                        dyeable: false,
                        use_player_texture: false,
                        color_when_undyed: None,
                    },
                    EquipmentAssetLayer {
                        texture: "outer".into(),
                        dyeable: false,
                        use_player_texture: false,
                        color_when_undyed: None,
                    },
                    EquipmentAssetLayer {
                        texture: "skipped".into(),
                        dyeable: false,
                        use_player_texture: false,
                        color_when_undyed: None,
                    },
                ];
                let piece = EquipmentPiece {
                    asset: "armor_test:chest".into(),
                    slot: ArmorSlot::Chest,
                    kind: "humanoid",
                    layers: &layers,
                    overlay: 36,
                };
                let bind = |name: &str| match name {
                    "inner" => Some(Binding {
                        name: "inner",
                        uv: [0.75, 0.75, 1.0, 1.0],
                    }),
                    "outer" => Some(Binding {
                        name: "outer",
                        uv: [0.75, 0.75, 1.0, 1.0],
                    }),
                    "custom_inner" => Some(Binding {
                        name: "custom_inner",
                        uv: [0.0, 0.0, 0.25, 0.25],
                    }),
                    "custom_outer" => Some(Binding {
                        name: "custom_outer",
                        uv: [0.25, 0.25, 0.5, 0.5],
                    }),
                    _ => None,
                };
                let mut default = bind;
                let mut glow = |_: &str| None;
                let materials = dispatch_equipment(
                    &host,
                    &registry.state,
                    &registry.state.items[&item],
                    &stack,
                    &piece,
                    EquipmentBindings {
                        trim: Some(Binding {
                            name: "trim",
                            uv: [0.5, 0.5, 1.0, 1.0],
                        }),
                        default: &mut default,
                        binding: &bind,
                        emissive: &mut glow,
                    },
                );
                assert_eq!(
                    *calls.lock().unwrap(),
                    [
                        "dye", "tint0", "texture0", "model0", "tint1", "texture1", "model1",
                        "tint2"
                    ]
                );
                assert_eq!(
                    materials
                        .iter()
                        .map(|m| (m.binding.name, m.overlay))
                        .collect::<Vec<_>>(),
                    [("custom_inner", 0), ("custom_outer", 0), ("trim", 36)]
                );
                assert_eq!(
                    materials[0].color, 0xFF56_3412,
                    "ARGB becomes renderer ABGR"
                );
                assert!(materials[0].final_texture && materials[1].final_texture);
                let selected = materials
                    .iter()
                    .map(|m| m.model.unwrap())
                    .collect::<Vec<_>>();
                assert!(Arc::ptr_eq(selected[0], &registry.state.models[0].slots[1]));
                assert!(Arc::ptr_eq(selected[1], &registry.state.models[1].slots[1]));
                assert!(
                    Arc::ptr_eq(selected[2], selected[1]),
                    "one trim on the last material's tree"
                );
                let mut vertices = Vec::new();
                for m in materials {
                    let model = m.model.unwrap();
                    sunrise_client::mobs::render::draw(
                        model,
                        glam::Mat4::IDENTITY,
                        &sunrise_client::mobs::anim::Poses::new(model),
                        m.binding.uv,
                        m.color,
                        0xF0F0,
                        m.overlay,
                        &mut vertices,
                    );
                }
                assert_eq!(
                    vertices.len(),
                    12,
                    "one face per material and one trim, no double painting"
                );
                assert_eq!(vertices[0].pos[0], 0.0);
                assert_eq!(vertices[4].pos[0], 1.0);
                assert_eq!(vertices[8].pos[0], 1.0);
                assert_eq!(vertices[0].uv, [0.0, 0.0]);
                assert_eq!(vertices[4].uv, [0.25, 0.25]);
                assert_eq!(vertices[8].uv, [0.5, 0.5]);
            },
        );
    }
    #[test]
    fn unknown_bindings_and_duplicate_providers_are_refused_by_name() {
        mod_platform::testing::with_mods(
            vec![
                mod_api::ModEntry::new("armor_test", Box::new(Empty)),
                mod_api::ModEntry::new("other", Box::new(Empty)),
            ]
            .into(),
            || {
                use mod_api::prelude::ItemsHost;
                struct DefaultProvider;
                impl ClientItemExtensions for DefaultProvider {}
                let mut host = mod_api::testing::FakeHost::new();
                let id = mod_api::Identifier::parse("armor_test:chest").unwrap();
                host.registries.resolve_items([id.clone()]).unwrap();
                let item = host.item_key(&id).unwrap();
                let mut registry = Registration::new(&|name| name == "known");
                let missing = registry.register_texture("armor_test:missing").unwrap_err();
                assert!(missing.contains("armor_test:missing"));
                {
                    let _owner = owner("armor_test");
                    let mut invalid = tree(0.0);
                    invalid.x = f32::NAN;
                    let invalid = registry
                        .register_model("armor_test:invalid", &invalid)
                        .unwrap_err();
                    assert!(invalid.contains("armor_test:invalid"));
                    registry
                        .client_item_extensions(item, Arc::new(DefaultProvider))
                        .unwrap();
                }
                let _owner = owner("other");
                let duplicate = registry
                    .client_item_extensions(item, Arc::new(DefaultProvider))
                    .unwrap_err();
                assert!(duplicate.contains("armor_test") && duplicate.contains("other"));
            },
        );
    }

    #[test]
    fn stale_refs_fall_back_to_vanilla_bindings_once_without_panicking() {
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("armor_test", Box::new(Empty))].into(),
            || {
                use mod_api::prelude::ItemsHost;
                struct Stale(u64);
                impl ClientItemExtensions for Stale {
                    fn armor_texture(
                        &self,
                        _: &dyn ClientHost,
                        _: &mod_api::ItemStack,
                        _: EquipmentLayerType,
                        _: &EquipmentLayer,
                        _: ArmorTextureRef,
                    ) -> ArmorTextureRef {
                        ArmorTextureRef::__new(self.0, 0)
                    }
                    fn armor_layer_model(
                        &self,
                        _: &dyn ClientHost,
                        _: &mod_api::ItemStack,
                        _: EquipmentLayerType,
                        _: &EquipmentLayer,
                        _: u32,
                    ) -> ArmorModelRef {
                        ArmorModelRef::__new(self.0, 0)
                    }
                }
                let mut host = mod_api::testing::FakeHost::new();
                let id = mod_api::Identifier::parse("armor_test:chest").unwrap();
                host.registries.resolve_items([id.clone()]).unwrap();
                let item = host.item_key(&id).unwrap();
                let old = ArmorExtensions::default();
                let mut registry = Registration::new(&|_| true);
                let _owner = owner("armor_test");
                registry.register_texture("original").unwrap();
                registry
                    .client_item_extensions(item, Arc::new(Stale(old.generation)))
                    .unwrap();
                let layers = [EquipmentAssetLayer {
                    texture: "original".into(),
                    dyeable: false,
                    use_player_texture: false,
                    color_when_undyed: None,
                }];
                let piece = EquipmentPiece {
                    asset: "armor_test:chest".into(),
                    slot: ArmorSlot::Chest,
                    kind: "humanoid",
                    layers: &layers,
                    overlay: 0,
                };
                let binding = Binding {
                    name: "original",
                    uv: [0.0, 0.0, 1.0, 1.0],
                };
                let part = &registry.state.items[&item];
                for _ in 0..2 {
                    let mut default = |_: &str| Some(binding);
                    let bind = |_: &str| Some(binding);
                    let mut glow = |_: &str| None;
                    let out = dispatch_equipment(
                        &host,
                        &registry.state,
                        part,
                        &mod_api::ItemStack::of(item),
                        &piece,
                        EquipmentBindings {
                            trim: None,
                            default: &mut default,
                            binding: &bind,
                            emissive: &mut glow,
                        },
                    );
                    assert_eq!(out.len(), 1);
                    assert_eq!(out[0].binding.name, "original");
                    assert!(out[0].model.is_none());
                    assert!(!out[0].final_texture);
                }
                assert!(part.reported_stale.load(Ordering::Relaxed));
                assert!(!part.failed.load(Ordering::Relaxed));
            },
        );
    }
    #[test]
    fn callback_failure_discards_earlier_layers_and_disables_until_reload() {
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("armor_test", Box::new(Empty))].into(),
            || {
                use mod_api::prelude::ItemsHost;
                struct FailsOnSecond(Arc<std::sync::atomic::AtomicUsize>);
                impl ClientItemExtensions for FailsOnSecond {
                    fn armor_layer_tint_color(
                        &self,
                        _: &dyn ClientHost,
                        _: &mod_api::ItemStack,
                        _: &EquipmentLayer,
                        index: u32,
                        _: u32,
                    ) -> u32 {
                        self.0.fetch_add(1, Ordering::Relaxed);
                        assert_ne!(index, 1, "callback refused");
                        0xFFFFFFFF
                    }
                }
                let mut host = mod_api::testing::FakeHost::new();
                let id = mod_api::Identifier::parse("armor_test:chest").unwrap();
                host.registries.resolve_items([id.clone()]).unwrap();
                let item = host.item_key(&id).unwrap();
                let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let layers = (0..2)
                    .map(|_| EquipmentAssetLayer {
                        texture: "original".into(),
                        dyeable: false,
                        use_player_texture: false,
                        color_when_undyed: None,
                    })
                    .collect::<Vec<_>>();
                let piece = EquipmentPiece {
                    asset: "armor_test:chest".into(),
                    slot: ArmorSlot::Chest,
                    kind: "humanoid",
                    layers: &layers,
                    overlay: 0,
                };
                let binding = Binding {
                    name: "original",
                    uv: [0.0, 0.0, 1.0, 1.0],
                };
                // Reload replaces the failed Part rather than reviving it in place.
                for reload in 0..2 {
                    let mut registry = Registration::new(&|_| true);
                    let _owner = owner("armor_test");
                    registry.register_texture("original").unwrap();
                    registry
                        .client_item_extensions(item, Arc::new(FailsOnSecond(calls.clone())))
                        .unwrap();
                    let part = &registry.state.items[&item];
                    for _ in 0..2 {
                        let out = dispatch_equipment(
                            &host,
                            &registry.state,
                            part,
                            &mod_api::ItemStack::of(item),
                            &piece,
                            EquipmentBindings {
                                trim: None,
                                default: &mut |_| Some(binding),
                                binding: &|_| Some(binding),
                                emissive: &mut |_| None,
                            },
                        );
                        assert!(out.is_empty(), "no earlier layer escapes the failed piece");
                        assert!(
                            part.failed.load(Ordering::Relaxed),
                            "caller selects vanilla"
                        );
                        assert_eq!(calls.load(Ordering::Relaxed), (reload + 1) * 2);
                    }
                }
            },
        );
    }
    #[test]
    fn served_by_armor_model_delegates_to_serving_host_only() {
        let packages = vec![
            mod_api::PackageInfo::new(
                "host".into(),
                "1".into(),
                mod_api::PackageKind::Code,
                mod_api::Side::Both,
                vec!["host".into()],
                vec![],
            ),
            mod_api::PackageInfo::new(
                "other".into(),
                "1".into(),
                mod_api::PackageKind::Code,
                mod_api::Side::Both,
                vec!["other".into()],
                vec![],
            ),
            mod_api::PackageInfo::new(
                "runes".into(),
                "1".into(),
                mod_api::PackageKind::Content,
                mod_api::Side::Both,
                vec!["runes".into()],
                vec![],
            )
            .with_served_by(Some("host".into())),
        ];
        mod_platform::__private::with_test_package_inventory(packages, || {
            let mut registry = Registration {
                state: Default::default(),
                published: None,
                sprite_exists: &|_| true,
                refused: None,
            };
            {
                let _owner = owner("host");
                registry.register_model("runes:helm", &tree(0.0)).unwrap();
                assert!(
                    registry
                        .register_model("minecraft:helm", &tree(0.0))
                        .is_err()
                );
            }
            {
                let _owner = owner("other");
                assert!(registry.register_model("runes:other", &tree(0.0)).is_err());
            }
            assert_eq!(registry.state.models.len(), 1);
            assert_eq!(registry.state.models[0].name, "runes:helm");
        });
    }
}
