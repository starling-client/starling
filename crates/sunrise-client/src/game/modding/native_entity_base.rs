//! `Entity`'s base state for every kind of entity, behind one accessor (#5082): the values vanilla
//! gives each `Entity` (`Fire`, `Invulnerable`, `NoGravity`, `Silent`, `Glowing`) whatever its
//! class. Each family keeps them where it already keeps its own state: as fields of its body,
//! in the saved compound Sunrise holds for the kinds it keeps as saved data (boats and
//! minecarts keep their saved flags there too, markers, displays and interactions all of them),
//! or, for a general entity, in the row. [`Game::read_entity_base`] and
//! [`Game::update_entity_base`] are the only places that know which family keeps which value, so
//! a new kind or a new operation touches this file alone.
use super::{Game, commands::Target, general::general_type_catalog};
use sunrise_core::nbt::{Compound, Tag};

/// Bits of the values a family keeps as fields of its own body, as opposed to in its saved
/// compound.
pub mod owned {
    pub const FIRE: u8 = 1;
    pub const INVULNERABLE: u8 = 1 << 1;
    pub const NO_GRAVITY: u8 = 1 << 2;
    pub const SILENT: u8 = 1 << 3;
    pub const GLOWING: u8 = 1 << 4;
    pub const ALL: u8 = FIRE | INVULNERABLE | NO_GRAVITY | SILENT | GLOWING;
}

/// The base state of one entity: vanilla's `Entity.remainingFireTicks`,
/// `permanentlyInvulnerable`, `isNoGravity`, `isSilent` and `hasGlowingTag`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntityBase {
    /// `Entity.getRemainingFireTicks`, saved as the short `Fire`.
    pub fire_ticks: i32,
    /// `Entity.isInvulnerable`, saved as `Invulnerable`.
    pub invulnerable: bool,
    /// `Entity.isNoGravity`, saved as `NoGravity` when set.
    pub no_gravity: bool,
    /// `Entity.isSilent`, saved as `Silent` when set.
    pub silent: bool,
    /// `Entity.hasGlowingTag`, saved as `Glowing` when set.
    pub glowing: bool,
}

impl EntityBase {
    /// The values a saved entity compound holds, as `Entity.load` reads them.
    pub fn read(tag: Option<&Tag>) -> Self {
        use sunrise_client::entity_nbt::{fire_ticks_from_nbt, flag};
        let Some(tag) = tag else {
            return Self::default();
        };
        Self {
            fire_ticks: fire_ticks_from_nbt(tag),
            invulnerable: flag(tag, "Invulnerable"),
            no_gravity: flag(tag, "NoGravity"),
            silent: flag(tag, "Silent"),
            glowing: flag(tag, "Glowing"),
        }
    }

    /// Which values differ from `other`, as [`owned`] bits.
    fn differing(&self, other: &Self) -> u8 {
        [
            (owned::FIRE, self.fire_ticks != other.fire_ticks),
            (owned::INVULNERABLE, self.invulnerable != other.invulnerable),
            (owned::NO_GRAVITY, self.no_gravity != other.no_gravity),
            (owned::SILENT, self.silent != other.silent),
            (owned::GLOWING, self.glowing != other.glowing),
        ]
        .into_iter()
        .filter(|(_, differs)| *differs)
        .fold(0, |bits, (bit, _)| bits | bit)
    }

    /// Writes the values in `mask` into `map` as `Entity.saveWithoutId` writes them: `Fire` as a
    /// short and `Invulnerable` as a boolean, always; the other flags only while set.
    pub fn write(&self, map: &mut Compound, mask: u8) {
        if mask & owned::FIRE != 0 {
            map.insert("Fire".into(), Tag::Short(self.fire_ticks as i16));
        }
        if mask & owned::INVULNERABLE != 0 {
            map.insert(
                "Invulnerable".into(),
                Tag::Byte(i8::from(self.invulnerable)),
            );
        }
        for (bit, key, value) in [
            (owned::NO_GRAVITY, "NoGravity", self.no_gravity),
            (owned::SILENT, "Silent", self.silent),
            (owned::GLOWING, "Glowing", self.glowing),
        ] {
            if mask & bit == 0 {
                continue;
            }
            let key = key.into();
            if value {
                map.insert(key, Tag::Byte(1));
            } else {
                map.remove(&key);
            }
        }
    }
}

/// An entity's base state with what the host reads beside it.
#[derive(Clone, Copy, Debug)]
pub(in crate::game) struct BaseView {
    pub base: EntityBase,
    /// `Entity.fireImmune`: the type's immunity, or the live grant the body carries.
    pub fire_immune: bool,
    /// The entity's [`BuiltinKind`], when it is one.
    pub kind: Option<BuiltinKind>,
    /// The [`owned`] values the family keeps as fields of its body.
    owned: u8,
}

/// The built-in kinds that are neither a mob (its type flag), an item (its stack), a general
/// entity (its row) nor a plain flag-less projectile: the one place the server's
/// [`Game::read_entity_base`] and a client's tracked copy both answer their fire facts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum BuiltinKind {
    Tnt,
    EndCrystal,
    EnderDragon,
    AreaEffectCloud,
    WitherSkull,
    ShulkerBullet,
}

impl BuiltinKind {
    /// The kind a client's tracked copy names by its type id, converted once at the boundary.
    pub(in crate::game) fn from_type_id(id: &str) -> Option<Self> {
        Some(match id {
            "minecraft:tnt" => Self::Tnt,
            "minecraft:end_crystal" => Self::EndCrystal,
            "minecraft:ender_dragon" => Self::EnderDragon,
            "minecraft:area_effect_cloud" => Self::AreaEffectCloud,
            "minecraft:wither_skull" => Self::WitherSkull,
            "minecraft:shulker_bullet" => Self::ShulkerBullet,
            _ => return None,
        })
    }

    /// Whether `EntityTypes` builds the kind with `.fireImmune()`: `TNT`, `END_CRYSTAL`,
    /// `ENDER_DRAGON` and `AREA_EFFECT_CLOUD`. `WITHER_SKULL` and `SHULKER_BULLET` are not.
    pub(in crate::game) fn fire_immune(self) -> bool {
        matches!(
            self,
            Self::Tnt | Self::EndCrystal | Self::EnderDragon | Self::AreaEffectCloud
        )
    }

    /// Whether the class overrides `Entity.isOnFire` to return false: `WitherSkull.isOnFire`
    /// and `ShulkerBullet.isOnFire`, whatever their fire ticks.
    pub(in crate::game) fn is_on_fire_overridden_false(self) -> bool {
        matches!(self, Self::WitherSkull | Self::ShulkerBullet)
    }
}

impl Game<'_> {
    /// The [`BuiltinKind`] of the entity `t` names.
    fn builtin_kind(&self, t: Target) -> Option<BuiltinKind> {
        use sunrise_client::projectiles::ProjectileKind;
        match t {
            Target::Tnt(_) => Some(BuiltinKind::Tnt),
            Target::Crystal(_) => Some(BuiltinKind::EndCrystal),
            Target::Dragon(_) | Target::DragonPart(_) => Some(BuiltinKind::EnderDragon),
            Target::Cloud(_) => Some(BuiltinKind::AreaEffectCloud),
            Target::Projectile(id) => {
                match self.creatures.projectiles.iter().find(|p| p.id == id)?.kind {
                    ProjectileKind::WitherSkull { .. } => Some(BuiltinKind::WitherSkull),
                    ProjectileKind::ShulkerBullet { .. } => Some(BuiltinKind::ShulkerBullet),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The base state of the entity `t` names, or None when its body is gone.
    pub(in crate::game) fn read_entity_base(&self, t: Target) -> Option<BaseView> {
        let mut base = EntityBase::read(self.saved_entity_compound(t));
        let (owned, fire_immune) = self.load_owned_base(t, &mut base)?;
        let kind = self.builtin_kind(t);
        Some(BaseView {
            base,
            fire_immune,
            kind,
            owned,
        })
    }

    /// Applies `edit` to the entity's base state and stores what changed, where its family
    /// keeps each value. False for a gone body and for a dragon's part, which has no record of
    /// its own: its eight parts are ids derived from the dragon's. A fishing bobber (never
    /// saved) and a natural leash knot (saved as `Entity.saveWithoutId` writes it, in
    /// `/data`'s tag) hold theirs in their own struct.
    pub(in crate::game) fn update_entity_base(
        &mut self,
        t: Target,
        edit: impl FnOnce(&mut EntityBase),
    ) -> bool {
        let Some(view) = self.read_entity_base(t) else {
            return false;
        };
        let mut new = view.base;
        edit(&mut new);
        let saved = new.differing(&view.base) & !view.owned;
        if saved != 0 && !self.with_saved_entity_mut(t, |tag| write_saved(&new, tag, saved)) {
            return false;
        }
        self.store_owned_base(t, &view.base, &new)
    }

    /// Runs `edit` on the saved compound of `t`, creating it when the kind keeps one and the
    /// entity has none yet. False for a kind that keeps no compound.
    fn with_saved_entity_mut(&mut self, t: Target, edit: impl FnOnce(&mut Tag)) -> bool {
        match t {
            Target::Falling(id) => {
                let uuid = self.uuid_for(id);
                let Some(i) = self.falling.iter().position(|f| f.id == id) else {
                    return false;
                };
                self.falling.update(i, |f| {
                    let saved = f
                        .saved
                        .get_or_insert_with(|| Box::new((uuid, Tag::Compound(Compound::new()))));
                    edit(&mut saved.1);
                });
                true
            }
            Target::Spawner(id) => {
                let Some(i) = self.ominous_spawners.iter().position(|s| s.id == id) else {
                    return false;
                };
                self.ominous_spawners.update(i, |spawner| {
                    let raw = spawner
                        .raw
                        .get_or_insert_with(|| Box::new(Tag::Compound(Compound::new())));
                    edit(raw);
                });
                true
            }
            Target::Hook(_) | Target::Knot(_) | Target::DragonPart(_) | Target::General(_) => false,
            t => match self.raw_mut(t) {
                Some(raw) => {
                    edit(raw);
                    true
                }
                None => false,
            },
        }
    }

    /// Reads the values the family of `t` keeps in its body into `base`, over what its saved
    /// compound gave. Returns the [`owned`] bits and the entity's `Entity.fireImmune`; None when
    /// the body is gone.
    fn load_owned_base(&self, t: Target, base: &mut EntityBase) -> Option<(u8, bool)> {
        use owned::*;
        Some(match t {
            Target::Player | Target::Guest(_) => {
                let p = self.native_player_body(t)?;
                base.fire_ticks = p.fire_ticks;
                base.invulnerable = p.health.permanently_invulnerable;
                (FIRE | INVULNERABLE, p.mod_fire_immune)
            }
            Target::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id)?;
                *base = EntityBase {
                    fire_ticks: m.fire_ticks,
                    invulnerable: m.invulnerable_flag,
                    no_gravity: m.no_gravity,
                    silent: m.silent,
                    glowing: m.glowing,
                };
                (ALL, m.fire_immune())
            }
            Target::Stand(id) => {
                let s = self.armor_stands.iter().find(|s| s.id == id)?;
                base.fire_ticks = s.fire_ticks;
                base.no_gravity = s.no_gravity;
                (FIRE | NO_GRAVITY, s.mod_fire_immune)
            }
            Target::Item(id) => {
                let i = self.dropped.iter().find(|i| i.id == id)?;
                base.fire_ticks = i.fire_ticks;
                base.no_gravity = i.no_gravity;
                // `ItemEntity.fireImmune`: a stack that fire cannot hurt.
                let immune =
                    self.stack_can_be_hurt_by(&i.stack, "minecraft:in_fire") == Some(false);
                (FIRE | NO_GRAVITY, immune)
            }
            Target::Orb(id) => {
                let o = self.orbs.iter().find(|o| o.id == id)?;
                base.fire_ticks = o.fire_ticks;
                base.no_gravity = o.no_gravity;
                (FIRE | NO_GRAVITY, false)
            }
            Target::Boat(id) => {
                let b = self.boats.iter().find(|b| b.id == id)?;
                base.fire_ticks = b.fire_ticks;
                base.no_gravity = b.no_gravity;
                (FIRE | NO_GRAVITY, false)
            }
            Target::Cart(id) => {
                let c = self.carts.iter().find(|c| c.id == id)?;
                base.no_gravity = c.no_gravity;
                (NO_GRAVITY, false)
            }
            Target::Projectile(id) => {
                let p = self.creatures.projectiles.iter().find(|p| p.id == id)?;
                base.no_gravity = p.no_gravity;
                match p.arrow.as_ref() {
                    Some(a) => {
                        base.fire_ticks = a.fire_ticks;
                        (FIRE | NO_GRAVITY, false)
                    }
                    None => {
                        base.fire_ticks = p.fire_ticks;
                        // No projectile type is built with `.fireImmune()`.
                        (FIRE | NO_GRAVITY, false)
                    }
                }
            }
            Target::Trident(id) => {
                let t = self.tridents.iter().find(|t| t.id == id)?;
                base.fire_ticks = t.fire_ticks;
                base.no_gravity = t.no_gravity;
                (FIRE | NO_GRAVITY, false)
            }
            Target::Tnt(id) => {
                let t = self.tnts.iter().find(|t| t.id == id)?;
                base.no_gravity = t.no_gravity;
                (NO_GRAVITY, BuiltinKind::Tnt.fire_immune())
            }
            Target::Falling(id) => {
                let f = self.falling.iter().find(|f| f.id == id)?;
                base.fire_ticks = f.fire_ticks;
                base.no_gravity = f.no_gravity;
                (FIRE | NO_GRAVITY, false)
            }
            Target::Crystal(id) => {
                let c = self.crystals.iter().find(|c| c.id == id)?;
                base.fire_ticks = c.fire;
                base.invulnerable = c.invulnerable;
                (FIRE | INVULNERABLE, BuiltinKind::EndCrystal.fire_immune())
            }
            Target::Bolt(id) => {
                let b = self.bolts.iter().find(|b| b.id == id)?;
                *base = EntityBase {
                    fire_ticks: b.fire_ticks,
                    invulnerable: b.invulnerable,
                    no_gravity: b.no_gravity,
                    silent: b.silent,
                    glowing: b.glowing,
                };
                (ALL, false)
            }
            Target::Rocket(id) => {
                let r = self.rockets.iter().find(|r| r.id == id)?;
                base.silent = r.silent;
                (SILENT, false)
            }
            Target::Fangs(id) => {
                let f = self.fangs.list.iter().find(|f| f.id == id)?;
                base.fire_ticks = f.fire_ticks;
                (FIRE, false)
            }
            Target::Dragon(_) => (0, BuiltinKind::EnderDragon.fire_immune()),
            Target::Cloud(_) => (0, BuiltinKind::AreaEffectCloud.fire_immune()),
            Target::General(id) => {
                let row = self.resident_entities.general.row(id)?;
                *base = row.base;
                (ALL, general_type_catalog().fire_immune(row.type_slot))
            }
            // The dragon's parts take its type.
            Target::DragonPart(_) => (ALL, BuiltinKind::EnderDragon.fire_immune()),
            // `EntityType.FISHING_BOBBER` is not fire immune; the bobber is never saved.
            Target::Hook(id) => {
                let (hook, _) = self.fishing_hook_by_id(id)?;
                *base = hook.base;
                (ALL, false)
            }
            Target::Knot(id) => {
                let k = self.resident_entities.knots.iter().find(|k| k.id == id)?;
                *base = EntityBase {
                    fire_ticks: k.fire_ticks,
                    invulnerable: k.invulnerable,
                    no_gravity: k.no_gravity,
                    silent: k.silent,
                    glowing: k.glowing,
                };
                (ALL, false)
            }
            Target::Frame(_)
            | Target::Painting(_)
            | Target::Cushion(_)
            | Target::Other(_)
            | Target::Spawner(_) => (0, false),
        })
    }

    /// Stores into the body of `t` the values of `new` its family keeps as fields. False for a
    /// gone body and for the kinds that keep no entity data.
    fn store_owned_base(&mut self, t: Target, old: &EntityBase, new: &EntityBase) -> bool {
        let changed = new.differing(old);
        let sync = changed & (owned::FIRE | owned::NO_GRAVITY | owned::GLOWING) != 0;
        match t {
            Target::Player | Target::Guest(_) => {
                let Some(p) = self.native_player_body_mut(t) else {
                    return false;
                };
                p.fire_ticks = new.fire_ticks;
                p.health.permanently_invulnerable = new.invulnerable;
            }
            Target::Mob(id) => {
                let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
                    return false;
                };
                m.fire_ticks = new.fire_ticks;
                m.invulnerable_flag = new.invulnerable;
                m.no_gravity = new.no_gravity;
                m.silent = new.silent;
                m.glowing = new.glowing;
                m.needs_sync |= sync;
            }
            Target::Stand(id) => {
                let Some(s) = self.armor_stands.iter_mut().find(|s| s.id == id) else {
                    return false;
                };
                s.fire_ticks = new.fire_ticks;
                s.no_gravity = new.no_gravity;
            }
            Target::Item(id) => {
                let Some(i) = self.dropped.iter_mut().find(|i| i.id == id) else {
                    return false;
                };
                i.fire_ticks = new.fire_ticks;
                i.no_gravity = new.no_gravity;
                i.needs_sync |= changed & owned::NO_GRAVITY != 0;
            }
            Target::Orb(id) => {
                let Some(o) = self.orbs.iter_mut().find(|o| o.id == id) else {
                    return false;
                };
                o.fire_ticks = new.fire_ticks;
                o.no_gravity = new.no_gravity;
            }
            Target::Boat(id) => {
                let Some(i) = self.boats.iter().position(|b| b.id == id) else {
                    return false;
                };
                self.boats.update(i, |b| {
                    b.fire_ticks = new.fire_ticks;
                    b.no_gravity = new.no_gravity;
                });
            }
            Target::Cart(id) => {
                let Some(i) = self.carts.iter().position(|c| c.id == id) else {
                    return false;
                };
                self.carts.update(i, |c| c.no_gravity = new.no_gravity);
            }
            Target::Projectile(id) => {
                let Some(i) = self.creatures.projectiles.iter().position(|p| p.id == id) else {
                    return false;
                };
                self.creatures.projectiles.update(i, |p| {
                    p.no_gravity = new.no_gravity;
                    if let Some(a) = p.arrow.as_mut() {
                        a.fire_ticks = new.fire_ticks;
                    } else {
                        p.fire_ticks = new.fire_ticks;
                    }
                });
            }
            Target::Trident(id) => {
                let Some(i) = self.tridents.iter().position(|t| t.id == id) else {
                    return false;
                };
                self.tridents.update(i, |t| {
                    t.fire_ticks = new.fire_ticks;
                    t.no_gravity = new.no_gravity;
                });
            }
            Target::Tnt(id) => {
                let Some(i) = self.tnts.iter().position(|t| t.id == id) else {
                    return false;
                };
                self.tnts.update(i, |t| t.no_gravity = new.no_gravity);
            }
            Target::Falling(id) => {
                let Some(i) = self.falling.iter().position(|f| f.id == id) else {
                    return false;
                };
                self.falling.update(i, |f| {
                    f.fire_ticks = new.fire_ticks;
                    f.no_gravity = new.no_gravity;
                });
            }
            Target::Crystal(id) => {
                let Some(i) = self.crystals.iter().position(|c| c.id == id) else {
                    return false;
                };
                self.crystals.update(i, |c| {
                    c.fire = new.fire_ticks;
                    c.invulnerable = new.invulnerable;
                });
            }
            Target::Bolt(id) => {
                let Some(i) = self.bolts.iter().position(|b| b.id == id) else {
                    return false;
                };
                self.bolts.update(i, |b| {
                    b.fire_ticks = new.fire_ticks;
                    b.invulnerable = new.invulnerable;
                    b.no_gravity = new.no_gravity;
                    b.silent = new.silent;
                    b.glowing = new.glowing;
                });
            }
            Target::Rocket(id) => {
                let Some(i) = self.rockets.iter().position(|r| r.id == id) else {
                    return false;
                };
                self.rockets.update(i, |r| r.silent = new.silent);
            }
            Target::Fangs(id) => {
                let Some(i) = self.fangs.list.iter().position(|f| f.id == id) else {
                    return false;
                };
                self.fangs.list.update(i, |f| f.fire_ticks = new.fire_ticks);
            }
            Target::General(id) => {
                let Some(row) = self.resident_entities.general.row_mut(id) else {
                    return false;
                };
                row.base = *new;
            }
            Target::Hook(id) => {
                let Some((_, caster)) = self.fishing_hook_by_id(id) else {
                    return false;
                };
                let Some(hook) = self.caster_hook_mut(caster) else {
                    return false;
                };
                hook.base = *new;
            }
            Target::Knot(id) => {
                let Some(i) = self.resident_entities.knots.iter().position(|k| k.id == id) else {
                    return false;
                };
                self.resident_entities.knots.update(i, |k| {
                    k.fire_ticks = new.fire_ticks;
                    k.invulnerable = new.invulnerable;
                    k.no_gravity = new.no_gravity;
                    k.silent = new.silent;
                    k.glowing = new.glowing;
                });
            }
            // A dragon's eight parts are ids derived from the dragon's, not structs: there is
            // no record to hold a part's own base state.
            Target::DragonPart(_) => return false,
            // Their values live in the saved compound, which `update_entity_base` wrote.
            Target::Dragon(_)
            | Target::Cloud(_)
            | Target::Frame(_)
            | Target::Painting(_)
            | Target::Cushion(_)
            | Target::Other(_)
            | Target::Spawner(_) => {}
        }
        true
    }
}

/// Writes the `mask` values of `base` into a saved entity compound.
fn write_saved(base: &EntityBase, tag: &mut Tag, mask: u8) {
    if let Tag::Compound(map) = tag {
        base.write(map, mask);
    }
}
