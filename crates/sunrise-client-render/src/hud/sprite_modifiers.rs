//! Polytone screen sprite modifiers (`polytone/overlay_modifiers`, wiki "Screen Sprite Modifiers"):
//! a GUI sprite drawn moved, resized, recoloured, with other atlas UVs or another texture, and
//! extra sprites drawn over it. Measured on the real mod (Polytone 26.3-6.8.6): the size grows
//! as the sprite is drawn (a hotbar stretches), `u0`..`v1` are atlas UVs, `color` is the
//! vertex colour, and an overlay goes through its own texture's modifier.

use crate::resource_source::ResourceSource;
use std::collections::HashMap;

use sunrise_polytone::sprites::{SpriteChange, SpriteModifier, sprite_id};

use super::{HudAtlas, Rect as SpriteRect};

/// The packs' modifiers, by the GUI id of the sprite each one changes (`hud/hotbar`, a mod's
/// `mymod:panel`).
#[derive(Debug, Default)]
pub struct SpriteModifiers {
    by_sprite: HashMap<String, (SpriteChange, Vec<SpriteChange>)>,
}

/// One sprite to draw in place of a modified one: where, which atlas rect and its colour (ABGR).
pub(crate) struct SpriteDraw {
    pub key: String,
    pub rect: SpriteRect,
    pub at: [f32; 4],
    pub color: u32,
}

/// `minecraft:hud/hotbar` as the GUI atlas names it (`hud/hotbar`; another namespace's keeps its
/// namespace).
fn gui_id(id: &str) -> String {
    id.strip_prefix("minecraft:").unwrap_or(id).to_owned()
}

impl SpriteModifiers {
    /// The modifiers of the packs merged at `root` for the GUI sprites among `ids` (the GUI
    /// atlas's): for each sprite, the last file that targets it.
    pub fn read<'a>(root: &ResourceSource, ids: impl Iterator<Item = &'a String>) -> Self {
        let mut modifiers = Vec::new();
        for path in root
            .list("assets/")
            .into_iter()
            .filter(|path| path.split_once("/polytone/overlay_modifiers/").is_some())
        {
            let parsed = root
                .read(&path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|e| e.to_string())
                })
                .and_then(|json| SpriteModifier::parse(&json));
            match parsed {
                Ok(m) => modifiers.push(m),
                Err(e) => log::warn!("{path}: {e}"),
            }
        }
        Self::from_modifiers(modifiers, ids)
    }

    /// `modifiers` (in load order) for the GUI sprites among `ids`: for each sprite, the last that
    /// targets it.
    fn from_modifiers<'a>(
        modifiers: Vec<SpriteModifier>,
        ids: impl Iterator<Item = &'a String>,
    ) -> Self {
        let mut by_sprite = HashMap::new();
        if !modifiers.is_empty() {
            for id in ids {
                if let Some(m) = modifiers
                    .iter()
                    .rev()
                    .find(|m| m.targets.matches(&sprite_id(id), |_| false))
                {
                    by_sprite.insert(id.clone(), (m.change.clone(), m.overlays.clone()));
                }
            }
        }
        Self { by_sprite }
    }

    pub fn is_empty(&self) -> bool {
        self.by_sprite.is_empty()
    }
}

/// `change` on a sprite at `key` drawn over `at` in `color`: its other texture, atlas UVs,
/// rectangle and colour.
fn apply(
    atlas: &HudAtlas,
    key: &str,
    rect: Option<SpriteRect>,
    change: &SpriteChange,
    at: [f32; 4],
    color: u32,
) -> Option<SpriteDraw> {
    // The caller's own cut of the sprite, unless the modifier draws another texture.
    let (key, mut rect) = match (change.texture.as_deref(), rect) {
        (None, Some(rect)) => (key.to_owned(), rect),
        (texture, _) => {
            let key = texture.map_or_else(|| key.to_owned(), gui_id);
            let rect = *atlas.gui_rect(&key)?;
            (key, rect)
        }
    };
    let [u0, v0, u1, v1] = change.uv;
    rect.u0 = u0.unwrap_or(rect.u0);
    rect.v0 = v0.unwrap_or(rect.v0);
    rect.u1 = u1.unwrap_or(rect.u1);
    rect.v1 = v1.unwrap_or(rect.v1);
    let [dx, dy, dw, dh] = change.offset.map(|v| v as f32);
    Some(SpriteDraw {
        key,
        rect,
        at: [at[0] + dx, at[1] + dy, at[2] + dw, at[3] + dh],
        color: change.color.map_or(color, crate::hud_layout::abgr),
    })
}

/// What drawing the sprite `key` (whose atlas `rect` the caller already cut, as a bar's fill) over
/// `at` in `color` (ABGR) draws under the packs' modifiers: the
/// sprite as its modifier changes it, then its overlays (each as its own texture's modifier also
/// changes it). `None` when no modifier targets the sprite.
pub(crate) fn draws(
    atlas: &HudAtlas,
    key: &str,
    rect: SpriteRect,
    at: [f32; 4],
    color: u32,
) -> Option<Vec<SpriteDraw>> {
    atlas.sprite_modifiers.draws(atlas, key, rect, at, color)
}

impl SpriteModifiers {
    fn draws(
        &self,
        atlas: &HudAtlas,
        key: &str,
        rect: SpriteRect,
        at: [f32; 4],
        color: u32,
    ) -> Option<Vec<SpriteDraw>> {
        let key = key.strip_prefix("minecraft:").unwrap_or(key);
        let (change, overlays) = self.by_sprite.get(key)?;
        let base = apply(atlas, key, Some(rect), change, at, color);
        let base_at = base.as_ref().map_or(at, |b| b.at);
        let mut out: Vec<SpriteDraw> = base.into_iter().collect();
        for overlay in overlays {
            let Some(texture) = overlay.texture.as_deref() else {
                continue;
            };
            let key = gui_id(texture);
            // The overlay's texture as its own modifier draws it, then the overlay's own changes.
            let own = self
                .by_sprite
                .get(&key)
                .map(|(own, _)| SpriteChange {
                    texture: None,
                    ..own.clone()
                })
                .unwrap_or_default();
            let Some(mut d) = apply(atlas, &key, None, &own, base_at, color) else {
                continue;
            };
            let [dx, dy, dw, dh] = overlay.offset.map(|v| v as f32);
            d.at = [d.at[0] + dx, d.at[1] + dy, d.at[2] + dw, d.at[3] + dh];
            let uvs = [
                &mut d.rect.u0,
                &mut d.rect.v0,
                &mut d.rect.u1,
                &mut d.rect.v1,
            ];
            for (slot, uv) in uvs.into_iter().zip(overlay.uv) {
                if let Some(uv) = uv {
                    *slot = uv;
                }
            }
            if let Some(c) = overlay.color {
                d.color = crate::hud_layout::abgr(c);
            }
            out.push(d);
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(u0: f32) -> SpriteRect {
        SpriteRect {
            u0,
            v0: 0.0,
            u1: u0 + 0.1,
            v1: 0.1,
            w: 9,
            h: 9,
        }
    }

    #[test]
    fn a_modified_sprite_draws_moved_resized_recoloured_and_then_its_overlays() {
        let mut atlas = HudAtlas::fonts(&ResourceSource::indexed(Default::default())).unwrap();
        for (id, rect) in [
            ("hud/hotbar", rect(0.0)),
            ("hud/hotbar_selection", rect(0.2)),
            ("hud/heart/full", rect(0.4)),
            ("hud/food_full", rect(0.6)),
        ] {
            atlas.sprites.insert(id.to_owned(), rect);
        }

        let modifiers: Vec<SpriteModifier> = [
            serde_json::json!({"texture": "minecraft:hud/hotbar", "x_inc": -20, "width_inc": 40}),
            serde_json::json!({"texture": "minecraft:hud/heart/full", "u0": 0, "u1": 0.5}),
            serde_json::json!({"texture": "minecraft:hud/food_full", "new_texture": "minecraft:hud/heart/full", "color": -65536}),
            serde_json::json!({"texture": "minecraft:hud/hotbar_selection", "overlays": [{"texture": "minecraft:hud/heart/full", "y_inc": -30}]}),
        ]
        .iter()
        .map(|j| SpriteModifier::parse(j).unwrap())
        .collect();
        let mods = SpriteModifiers::from_modifiers(modifiers, atlas.sprites.keys());
        let white = 0xFFFF_FFFF;
        // Moved left 20 and 40 wider, the rest as it was.
        let hotbar = &mods
            .draws(
                &atlas,
                "hud/hotbar",
                rect(0.0),
                [100.0, 200.0, 182.0, 22.0],
                white,
            )
            .unwrap()[0];
        assert_eq!(hotbar.at, [80.0, 200.0, 222.0, 22.0]);
        assert_eq!((hotbar.rect.u0, hotbar.color), (0.0, white));
        // The namespaced spelling of the vanilla key is the same sprite.
        let spelled = &mods
            .draws(
                &atlas,
                "minecraft:hud/hotbar",
                rect(0.0),
                [100.0, 200.0, 182.0, 22.0],
                white,
            )
            .unwrap()[0];
        assert_eq!(spelled.at, hotbar.at);
        // The heart's atlas UVs: u0 and u1 replaced, v kept.
        let heart = &mods
            .draws(&atlas, "hud/heart/full", rect(0.4), [0.0; 4], white)
            .unwrap()[0];
        assert_eq!(
            [heart.rect.u0, heart.rect.u1, heart.rect.v1],
            [0.0, 0.5, 0.1]
        );
        // Food drawn with the heart's texture (its own rect, not the heart modifier's UVs), red.
        let food = &mods
            .draws(&atlas, "hud/food_full", rect(0.6), [0.0; 4], white)
            .unwrap()[0];
        assert_eq!((food.key.as_str(), food.rect.u0), ("hud/heart/full", 0.4));
        assert_eq!(food.color, crate::hud_layout::abgr(0xFFFF_0000));
        // The selection, then the heart over it 30 higher, with the heart's own UVs.
        let selection = mods
            .draws(
                &atlas,
                "hud/hotbar_selection",
                rect(0.2),
                [10.0, 50.0, 24.0, 23.0],
                white,
            )
            .unwrap();
        assert_eq!(selection.len(), 2);
        assert_eq!(selection[0].at, [10.0, 50.0, 24.0, 23.0]);
        assert_eq!(selection[1].at, [10.0, 20.0, 24.0, 23.0]);
        assert_eq!((selection[1].rect.u0, selection[1].rect.u1), (0.0, 0.5));
        // A sprite no modifier aims at draws as it is.
        assert!(
            mods.draws(&atlas, "hud/crosshair", rect(0.9), [0.0; 4], white)
                .is_none()
        );
    }

    #[test]
    fn a_namespaced_texture_names_its_own_gui_id() {
        assert_eq!(gui_id("minecraft:hud/hotbar"), "hud/hotbar");
        assert_eq!(gui_id("hud/hotbar"), "hud/hotbar");
        assert_eq!(gui_id("mypack:hud/bar"), "mypack:hud/bar");
    }
}
