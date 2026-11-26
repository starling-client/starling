//! The HUD's hotbar, bars and mount hearts follow `Hud.getCameraPlayer`: the local player, another
//! player the server attached the camera to, or nothing when the camera is not a player.
use super::Game;
use crate::game::hotbar_items::HotbarSource;
use crate::game::remote_camera::CameraPlayer;
use mod_api::client::hud_layout::HeartKind;
use sunrise_client::hud_layout::{AirInputs, FoodInputs, HealthInputs, HotbarInputs};
use sunrise_client::mobs::finalize::{self, Attr};
use sunrise_items::equipment::OFFHAND;

/// `FoodData`'s defaults, which a client keeps for every player but itself.
const REMOTE_FOOD: i32 = 20;
const REMOTE_SATURATION: f32 = 5.0;
/// `Entity.getMaxAirSupply`.
const MAX_AIR: i32 = 300;

impl Game<'_> {
    /// `Hud.extractItemHotbar`'s inputs and where its stacks come from; None draws nothing.
    pub(in crate::game) fn camera_hotbar(
        &self,
        camera: CameraPlayer,
    ) -> Option<(HotbarInputs, HotbarSource)> {
        match camera {
            CameraPlayer::Absent => None,
            CameraPlayer::Own => {
                let partial = self.clock.fraction() as f32;
                let inputs = HotbarInputs {
                    selected: self.selected as i32,
                    right_handed: !self.settings.main_hand_left,
                    offhand: self.offhand.is_some(),
                    slots: std::array::from_fn(|i| {
                        self.inventory[i]
                            .as_ref()
                            .map(|_| (f32::from(self.hotbar_pop[i]) - partial).max(0.0))
                    }),
                    offhand_pop: (f32::from(self.hotbar_pop[9]) - partial).max(0.0),
                };
                Some((inputs, HotbarSource::Own))
            }
            CameraPlayer::Other(i) => {
                let m = &self.mobs[i];
                // A client never syncs another player's selected slot: the main hand is slot 0.
                let inputs = HotbarInputs {
                    selected: 0,
                    right_handed: !m.extra.main_hand_left,
                    offhand: m.extra.slot_stack(OFFHAND).is_some(),
                    slots: std::array::from_fn(|s| {
                        (s == 0 && m.extra.main_hand_stack().is_some()).then_some(0.0)
                    }),
                    offhand_pop: 0.0,
                };
                Some((inputs, HotbarSource::Mob(i)))
            }
            CameraPlayer::Detached => {
                let hud = self.detached_camera_hud()?;
                let inputs = HotbarInputs {
                    selected: 0,
                    right_handed: !hud.main_hand_left,
                    offhand: hud.offhand.is_some(),
                    slots: std::array::from_fn(|s| {
                        (s == 0 && hud.main_hand.is_some()).then_some(0.0)
                    }),
                    offhand_pop: 0.0,
                };
                Some((inputs, HotbarSource::DetachedCamera))
            }
        }
    }

    /// `Hud.getPlayerVehicleWithHealth`: the camera player's vehicle as its max health and health.
    pub(in crate::game) fn camera_vehicle(&mut self, camera: CameraPlayer) -> Option<(f32, f32)> {
        let id = match camera {
            CameraPlayer::Absent | CameraPlayer::Detached => return None,
            CameraPlayer::Own => self.riding?,
            CameraPlayer::Other(i) => {
                let rider = self.mobs[i].id as i32;
                self.remote
                    .as_ref()
                    .and_then(|r| r.entities.get(&rider)?.vehicle)
                    .or(self.mobs[i].extra.mount)?
            }
        };
        // Another player can ride the local player, whose attributes and health
        // are kept on PlayerMotion rather than in the tracked entity table.
        if self
            .remote
            .as_ref()
            .is_some_and(|r| r.entity_id as u32 == id)
        {
            let p = self.player.as_mut()?;
            let max_health = p.attribute("minecraft:max_health").ok()? as f32;
            return Some((max_health, p.health.health()));
        }
        let (default_max, health) =
            if let Some(dragon) = self.dragon.as_ref().filter(|d| d.id == id) {
                (super::dragon::MAX_HEALTH, dragon.health)
            } else if let Some(m) = self.mob_index.get(&self.mobs, id) {
                (m.attrs.max_health, m.health)
            } else {
                let stand = self.armor_stands.iter().find(|s| s.id == id)?;
                (20.0, stand.health)
            };
        let max_health = self
            .motion
            .attributes
            .id("minecraft:max_health")
            .and_then(|attr| self.remote_attribute(id as i32, attr))
            .map_or(default_max, |v| v as f32);
        Some((max_health, health))
    }

    /// `Hud.extractVehicleHealth`: `(max_hearts, health)` of `camera_vehicle`, when positive.
    pub(in crate::game) fn camera_vehicle_health(
        &mut self,
        camera: CameraPlayer,
    ) -> Option<(i32, f32)> {
        let (max_health, health) = self.camera_vehicle(camera)?;
        let hearts = sunrise_client::hud_layout::vehicle_max_hearts(max_health);
        (hearts > 0).then_some((hearts, health))
    }

    /// `Hud.extractPlayerHealth` inputs for `VanillaGuiLayer::PlayerHealth`; None draws nothing.
    pub(in crate::game) fn camera_health_inputs(
        &mut self,
        camera: CameraPlayer,
    ) -> Option<(HealthInputs, HeartKind)> {
        let (tick, millis) = (self.visual_ticks() as i32, self.visual_millis());
        match camera {
            CameraPlayer::Absent => None,
            CameraPlayer::Own => {
                let p = self.player.as_mut()?;
                let blink = self.health_hud.update_health(
                    p.health.health(),
                    tick,
                    millis,
                    p.health.hurt.cooldown > 0,
                );
                let inputs = HealthInputs {
                    tick,
                    max_health: p.attribute("minecraft:max_health").unwrap_or(20.0) as f32,
                    absorption: p.health.absorption(),
                    regeneration: self.potions.effects.get("regeneration").is_some(),
                    hardcore: self.level.hardcore,
                    blink,
                };
                let kind = self.player_heart_kind()?;
                Some((inputs, kind))
            }
            CameraPlayer::Other(i) => {
                let (
                    mob_id,
                    health,
                    default_max,
                    absorption,
                    frozen,
                    invulnerable,
                    regeneration,
                    poison,
                    wither,
                ) = {
                    let m = &self.mobs[i];
                    let fx = &m.extra.client_effects;
                    (
                        m.id as i32,
                        m.health,
                        m.attrs.max_health,
                        m.absorption,
                        m.is_fully_frozen(),
                        m.invulnerable > 0,
                        fx.get("regeneration").is_some(),
                        fx.get("poison").is_some(),
                        fx.get("wither").is_some(),
                    )
                };
                let max_health = self
                    .motion
                    .attributes
                    .id("minecraft:max_health")
                    .and_then(|attr| self.remote_attribute(mob_id, attr))
                    .map_or(default_max, |v| v as f32);
                let blink = self
                    .health_hud
                    .update_health(health, tick, millis, invulnerable);
                let inputs = HealthInputs {
                    tick,
                    max_health,
                    absorption,
                    regeneration,
                    hardcore: self.level.hardcore,
                    blink,
                };
                let mut kind = if poison {
                    HeartKind::Poisoned
                } else if wither {
                    HeartKind::Withered
                } else if frozen {
                    HeartKind::Frozen
                } else {
                    HeartKind::Normal
                };
                self.native_player_heart_type(&mut kind);
                Some((inputs, kind))
            }
            CameraPlayer::Detached => {
                let (
                    health,
                    max_health,
                    absorption,
                    frozen,
                    invulnerable,
                    regeneration,
                    poison,
                    wither,
                ) = {
                    let hud = self.detached_camera_hud()?;
                    (
                        hud.health,
                        hud.max_health,
                        hud.absorption,
                        hud.frozen,
                        hud.invulnerable,
                        hud.regeneration,
                        hud.poison,
                        hud.wither,
                    )
                };
                let blink = self
                    .health_hud
                    .update_health(health, tick, millis, invulnerable);
                let inputs = HealthInputs {
                    tick,
                    max_health,
                    absorption,
                    regeneration,
                    hardcore: self.level.hardcore,
                    blink,
                };
                let mut kind = if poison {
                    HeartKind::Poisoned
                } else if wither {
                    HeartKind::Withered
                } else if frozen {
                    HeartKind::Frozen
                } else {
                    HeartKind::Normal
                };
                self.native_player_heart_type(&mut kind);
                Some((inputs, kind))
            }
        }
    }

    /// `Hud.extractArmor` value for `VanillaGuiLayer::ArmorLevel`; None draws nothing.
    pub(in crate::game) fn camera_armor(&mut self, camera: CameraPlayer) -> Option<i32> {
        match camera {
            CameraPlayer::Absent => None,
            CameraPlayer::Own => {
                let p = self.player.as_mut()?;
                Some(
                    p.attribute("minecraft:armor")
                        .or_else(|_| p.attribute("armor"))
                        .unwrap_or(0.0) as i32,
                )
            }
            CameraPlayer::Other(i) => {
                let (mob_id, fallback) = {
                    let m = &self.mobs[i];
                    (m.id as i32, finalize::value(m, Attr::Armor) as i32)
                };
                Some(
                    self.motion
                        .attributes
                        .id("minecraft:armor")
                        .and_then(|attr| self.remote_attribute(mob_id, attr))
                        .map_or(fallback, |v| v.floor() as i32),
                )
            }
            CameraPlayer::Detached => self.detached_camera_hud().map(|hud| hud.armor),
        }
    }

    /// `Hud.extractFood` inputs for `VanillaGuiLayer::FoodLevel`; None draws nothing.
    pub(in crate::game) fn camera_food_inputs(&self, camera: CameraPlayer) -> Option<FoodInputs> {
        let tick = self.visual_ticks() as i32;
        match camera {
            CameraPlayer::Absent => None,
            CameraPlayer::Own => {
                let p = self.player.as_ref()?;
                Some(FoodInputs {
                    tick,
                    food: p.health.food.food,
                    saturation: p.health.food.saturation,
                    hunger: self.potions.effects.get("hunger").is_some(),
                })
            }
            CameraPlayer::Other(i) => Some(FoodInputs {
                tick,
                food: REMOTE_FOOD,
                saturation: REMOTE_SATURATION,
                hunger: self.mobs[i].extra.client_effects.get("hunger").is_some(),
            }),
            CameraPlayer::Detached => Some(FoodInputs {
                tick,
                food: REMOTE_FOOD,
                saturation: REMOTE_SATURATION,
                hunger: self.detached_camera_hud()?.hunger,
            }),
        }
    }

    /// `Hud.extractAirBubbles` inputs for `VanillaGuiLayer::AirLevel`; only the local player plays pop sounds.
    pub(in crate::game) fn camera_air_inputs(
        &self,
        camera: CameraPlayer,
    ) -> Option<(AirInputs, Option<[f64; 3]>)> {
        let tick = self.visual_ticks() as i32;
        match camera {
            CameraPlayer::Absent => None,
            CameraPlayer::Own => {
                let p = self.player.as_ref()?;
                Some((
                    AirInputs {
                        tick,
                        air: p.air,
                        max_air: MAX_AIR,
                        // Hud.extractAirBubbles checks the fluid at the eyes, not the block.
                        under_water: p.is_eye_in_fluid(sunrise_client::motion::WATER),
                    },
                    Some([p.position.x, p.position.y, p.position.z]),
                ))
            }
            CameraPlayer::Other(i) => {
                let m = &self.mobs[i];
                Some((
                    AirInputs {
                        tick,
                        air: m.air,
                        max_air: MAX_AIR,
                        under_water: m.fluids.eye_in_water,
                    },
                    None,
                ))
            }
            CameraPlayer::Detached => {
                let hud = self.detached_camera_hud()?;
                Some((
                    AirInputs {
                        tick,
                        air: hud.air,
                        max_air: MAX_AIR,
                        under_water: hud.under_water,
                    },
                    None,
                ))
            }
        }
    }
}
