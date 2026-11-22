//! Resolve the mob's travel branch and fire `living_travel` at its entry.
//! The same route names the mode and runs travel, including species overrides.
use super::super::{Species, allay, aquatic, frog, guardian, nautilus, turtle, undead, vex};
use super::{DamageLevel, Fluid, Mob, MobEvent, MobWorld, TravelPause, Vec3};
use std::ops::ControlFlow;
use sunrise_world::travel::TravelMode;

enum TravelRoute {
    Air,
    AirEndingGlide,
    Fluid { water: bool },
    Glide,
    Flying,
    Allay,
    Vex,
    Squid,
    Fish,
    Drowned,
    Frog,
    Guardian,
    Nautilus,
    Turtle,
}

impl TravelRoute {
    fn mode(&self) -> TravelMode {
        match self {
            Self::Air | Self::AirEndingGlide => TravelMode::Air,
            Self::Fluid { water: true } => TravelMode::Water,
            Self::Fluid { water: false } => TravelMode::Lava,
            Self::Glide => TravelMode::Glide,
            Self::Flying | Self::Allay | Self::Vex => TravelMode::Flying,
            Self::Squid => TravelMode::VelocityOnly,
            Self::Fish
            | Self::Drowned
            | Self::Frog
            | Self::Guardian
            | Self::Nautilus
            | Self::Turtle => TravelMode::Water,
        }
    }
}

impl Mob {
    pub(in crate::mobs) fn travel_until_pause(
        &mut self,
        w: &MobWorld,
        level: &mut DamageLevel<'_>,
        mut input: Vec3,
        events: &mut Vec<MobEvent>,
        pause: bool,
    ) -> ControlFlow<TravelPause> {
        let Some(route) = self.travel_route(w) else {
            return ControlFlow::Continue(());
        };
        let mut velocity = self.velocity;
        if !crate::travel_gate::decide(
            w.loop_context,
            self.id,
            &self.kind.id,
            || route.mode(),
            &mut input,
            &mut velocity,
        ) {
            return ControlFlow::Continue(());
        }
        self.velocity = velocity;
        // Camel.travel: on the ground, a camel that sits or changes pose keeps
        // the vertical part of its motion and drops the horizontal input.
        if super::super::camel::is_camel(self.species())
            && self.ground.on_ground
            && super::super::camel::refuse_to_move(self)
        {
            let vertical = Vec3::new(0.0, 1.0, 0.0);
            self.velocity = self.velocity.multiply(vertical);
            input = input.multiply(vertical);
        }
        match route {
            TravelRoute::Air => self.travel_in_air(w, level, input, events, pause, false),
            TravelRoute::AirEndingGlide => self.travel_in_air(w, level, input, events, pause, true),
            TravelRoute::Fluid { water } => {
                self.travel_in_fluid(w, level, input, events, pause, water)
            }
            TravelRoute::Glide => self.travel_fall_flying(w, level, events, pause),
            TravelRoute::Flying => self.travel_flying(w, level, input, events, pause),
            TravelRoute::Allay => allay::travel(self, w, level, input, events, pause),
            TravelRoute::Vex => {
                vex::travel(self, w);
                ControlFlow::Continue(())
            }
            // Squid.travel moves by its current velocity alone, even out of water.
            TravelRoute::Squid => self.travel_move(w, level, events, pause, |_, _, _, _| {}),
            TravelRoute::Fish => aquatic::travel_in_water(self, w, level, input, events, pause),
            TravelRoute::Drowned => undead::drowned_travel(self, w, level, input, events, pause),
            TravelRoute::Frog => frog::travel(self, w, level, input, events, pause),
            TravelRoute::Guardian => guardian::travel(self, w, level, input, events, pause),
            TravelRoute::Nautilus => nautilus::travel(self, w, level, input, events, pause),
            TravelRoute::Turtle => turtle::travel(self, w, level, input, events, pause),
        }
    }

    // Decide once, before mod input or velocity can change. Species overrides
    // take precedence over the generic branches; Shulkers do not travel.
    fn travel_route(&self, w: &MobWorld) -> Option<TravelRoute> {
        use Species as S;
        let water = self.fluids.in_water;
        Some(match self.species() {
            S::Shulker => return None,
            S::Allay => TravelRoute::Allay,
            S::Vex => TravelRoute::Vex,
            S::Squid | S::GlowSquid => TravelRoute::Squid,
            S::Axolotl
            | S::Cod
            | S::Salmon
            | S::TropicalFish
            | S::Pufferfish
            | S::Tadpole
            | S::Dolphin
                if water =>
            {
                TravelRoute::Fish
            }
            S::Drowned
                if water && self.was_eye_in_water && undead::drowned_wants_to_swim(self, w) =>
            {
                TravelRoute::Drowned
            }
            S::Frog if water => TravelRoute::Frog,
            S::Guardian | S::ElderGuardian if water => TravelRoute::Guardian,
            S::Nautilus | S::ZombieNautilus if water => TravelRoute::Nautilus,
            S::Turtle if water => TravelRoute::Turtle,
            _ if self.kind.flags.flying => TravelRoute::Flying,
            _ => {
                // shouldTravelInFluid: not where the fluid at its feet holds it up.
                let (x, y, z) = (
                    self.position.x.floor() as i32,
                    self.position.y.floor() as i32,
                    self.position.z.floor() as i32,
                );
                let stands_on_fluid =
                    self.kind.flags.lava_walker && w.fluid(x, y, z) == Fluid::Lava;
                if self.is_in_liquid() && self.affected_by_fluids() && !stands_on_fluid {
                    TravelRoute::Fluid { water }
                } else if self.extra.fall_flying {
                    if self.on_climbable(w) {
                        TravelRoute::AirEndingGlide
                    } else {
                        TravelRoute::Glide
                    }
                } else {
                    TravelRoute::Air
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{Flow, Hooks, LivingTravelEvent, LoopContext, TravelMode as Mode};
    use std::sync::{Arc, Mutex};

    #[test]
    #[ignore = "daily"]
    fn custom_mob_travel_reports_the_route_it_runs() {
        struct Observe(Arc<Mutex<Vec<(u32, Mode)>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Observe {
            fn living_travel(
                &self,
                _: &dyn LoopContext,
                event: &mut LivingTravelEvent<'_>,
            ) -> Flow {
                self.0.lock().unwrap().push((event.entity, event.mode));
                Flow::Continue
            }
        }

        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "observe",
            Box::new(Observe(Arc::clone(&heard))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            assert!(
                crate::mobs::world::with_test_world(&[], |w, types| {
                    let at = Vec3::new(0.5, 80.0, 0.5);
                    let mut test_level = crate::health::TestLevel::new(0);
                    for (id, name, dx, vx) in [
                        (1, "minecraft:allay", 1.0, f64::from(0.91_f32)),
                        (2, "minecraft:vex", 1.0, f64::from(0.91_f32)),
                        (3, "minecraft:squid", 1.0, 1.0),
                        (4, "minecraft:shulker", 0.0, 1.0),
                    ] {
                        let mut mob = Mob::new(id, Arc::new(types.types[name].clone()), at, 0.0);
                        mob.velocity = Vec3::new(1.0, 0.0, 0.0);
                        mob.travel(w, &mut test_level.level(), Vec3::ZERO, &mut Vec::new());
                        assert_eq!(mob.position.x, at.x + dx, "{name} displacement");
                        assert_eq!(mob.velocity.x, vx, "{name} drag");
                        if name == "minecraft:allay" {
                            assert_eq!(mob.velocity.y, 0.0, "allay flight does not apply gravity");
                        }
                    }
                }),
                "prepared resources are required"
            );
        });
        assert_eq!(
            *heard.lock().unwrap(),
            vec![
                (1, Mode::Flying),
                (2, Mode::Flying),
                (3, Mode::VelocityOnly)
            ]
        );
    }
}
