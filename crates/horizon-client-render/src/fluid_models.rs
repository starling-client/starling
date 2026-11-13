//! What each mod's world fluid draws with in the world: the sprite and tint its look declares
//! (`data/<id>/horizon/fluid/`), completed by what mods register (`register_fluid_models`).
//! There is no second appearance store: a model only names sprites a look can't, and every
//! resource load reads this again, so a reload draws the new pack's textures.
use horizon_mod_package::content::{FluidLook, FluidModel, namespaced_id};
use std::sync::RwLock;

static LOOKS: RwLock<Vec<FluidLook>> = RwLock::new(Vec::new());
static REGISTERED: RwLock<Vec<FluidModel>> = RwLock::new(Vec::new());

/// The colour a mod fluid's sprites are multiplied by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FluidTint {
    /// The biome's water colour: a fluid that declares neither sprite nor tint looks like water.
    Biome,
    /// `0xRRGGBB`; a look's alpha is not drawn in the world.
    Fixed(u32),
}

/// A mod fluid's model, with every sprite named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedFluidModel {
    pub fluid: String,
    pub still: String,
    pub flowing: String,
    /// What it draws on sides against glass and leaves, as water's overlay.
    pub overlay: Option<String>,
    pub tint: FluidTint,
}

/// Remembers the world fluids' looks; every resource load calls this before it reads them.
pub fn set_world_looks<'a>(looks: impl IntoIterator<Item = &'a FluidLook>) {
    *LOOKS.write().unwrap_or_else(|e| e.into_inner()) = looks
        .into_iter()
        .filter(|look| look.world.is_some())
        .cloned()
        .collect();
}

/// Replaces the models recollected before each resource bake, including reloads.
pub fn register(models: Vec<FluidModel>) {
    *REGISTERED.write().unwrap_or_else(|e| e.into_inner()) = models;
}

/// Every world fluid's model.
pub fn resolved() -> Vec<ResolvedFluidModel> {
    resolve(
        &LOOKS.read().unwrap_or_else(|e| e.into_inner()),
        &REGISTERED.read().unwrap_or_else(|e| e.into_inner()),
    )
}

fn resolve(looks: &[FluidLook], registered: &[FluidModel]) -> Vec<ResolvedFluidModel> {
    for model in registered {
        if !looks.iter().any(|look| look.id == model.fluid) {
            log::warn!(
                "Horizon: a fluid model names {}, which is no world fluid",
                model.fluid
            );
        }
    }
    looks
        .iter()
        .map(|look| {
            let model = registered.iter().find(|model| model.fluid == look.id);
            let sprite = |name: Option<&String>| {
                name.filter(|name| {
                    let valid = namespaced_id(name);
                    if !valid {
                        log::warn!("Horizon: fluid {}: bad sprite id {name:?}", look.id);
                    }
                    valid
                })
                .cloned()
            };
            let overlay = sprite(model.and_then(|model| model.overlay.as_ref()));
            let flowing = sprite(model.and_then(|model| model.flowing.as_ref()));
            let still = sprite(model.and_then(|model| model.still.as_ref()))
                .or_else(|| sprite(look.sprite.as_ref()));
            let fixed = look.tint.map(|tint| FluidTint::Fixed(tint & 0xFF_FFFF));
            match still {
                Some(still) => ResolvedFluidModel {
                    fluid: look.id.clone(),
                    flowing: flowing.unwrap_or_else(|| still.clone()),
                    still,
                    overlay,
                    tint: fixed.unwrap_or(FluidTint::Fixed(0xFF_FFFF)),
                },
                None => ResolvedFluidModel {
                    fluid: look.id.clone(),
                    still: "minecraft:block/water_still".into(),
                    flowing: flowing.unwrap_or_else(|| "minecraft:block/water_flow".into()),
                    overlay: overlay.or_else(|| Some("minecraft:block/water_overlay".into())),
                    tint: fixed.unwrap_or(FluidTint::Biome),
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_mod_package::content::WorldFluid;

    fn look(id: &str, sprite: Option<&str>, tint: Option<u32>) -> FluidLook {
        FluidLook {
            id: id.into(),
            sprite: sprite.map(Into::into),
            tint,
            world: Some(WorldFluid::default()),
        }
    }
    fn model(fluid: &str) -> FluidModel {
        FluidModel {
            fluid: fluid.into(),
            ..FluidModel::default()
        }
    }

    #[test]
    fn a_look_alone_draws_its_sprite_and_tint_on_every_surface() {
        let drawn = resolve(
            &[look("a:oil", Some("a:block/oil"), Some(0xFF11_2233))],
            &[],
        );
        assert_eq!(
            drawn,
            [ResolvedFluidModel {
                fluid: "a:oil".into(),
                still: "a:block/oil".into(),
                flowing: "a:block/oil".into(),
                overlay: None,
                tint: FluidTint::Fixed(0x11_2233),
            }]
        );
    }

    #[test]
    fn a_sprite_without_a_tint_is_drawn_as_it_is() {
        let drawn = resolve(&[look("a:oil", Some("a:block/oil"), None)], &[]);
        assert_eq!(drawn[0].tint, FluidTint::Fixed(0xFF_FFFF));
    }

    #[test]
    fn no_sprite_draws_waters_textures_with_the_look_tint_or_the_biomes() {
        let plain = &resolve(&[look("a:plain", None, None)], &[])[0];
        assert_eq!(plain.still, "minecraft:block/water_still");
        assert_eq!(plain.flowing, "minecraft:block/water_flow");
        assert_eq!(plain.tint, FluidTint::Biome);
        let tinted = &resolve(&[look("a:tinted", None, Some(0xFF00_0010))], &[])[0];
        assert_eq!(tinted.still, "minecraft:block/water_still");
        assert_eq!(tinted.tint, FluidTint::Fixed(0x10));
    }

    #[test]
    fn a_model_names_what_the_look_cannot_and_keeps_the_looks_tint() {
        let drawn = resolve(
            &[
                look("a:oil", Some("a:block/oil"), Some(0xFF11_2233)),
                look("a:other", Some("a:block/other"), None),
            ],
            &[FluidModel {
                flowing: Some("a:block/oil_flow".into()),
                overlay: Some("a:block/oil_overlay".into()),
                ..model("a:oil")
            }],
        );
        assert_eq!(drawn[0].still, "a:block/oil");
        assert_eq!(drawn[0].flowing, "a:block/oil_flow");
        assert_eq!(drawn[0].overlay.as_deref(), Some("a:block/oil_overlay"));
        assert_eq!(drawn[0].tint, FluidTint::Fixed(0x11_2233));
        assert_eq!(drawn[1].flowing, "a:block/other", "its model is its own");
    }

    #[test]
    fn a_model_for_no_world_fluid_draws_nothing_and_a_bad_sprite_is_ignored() {
        let drawn = resolve(
            &[look("a:oil", Some("a:block/oil"), None)],
            &[
                FluidModel {
                    still: Some("no namespace".into()),
                    ..model("a:oil")
                },
                model("a:ghost"),
            ],
        );
        assert_eq!(drawn.len(), 1);
        assert_eq!(drawn[0].still, "a:block/oil");
    }
}
