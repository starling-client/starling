//! World helpers: block explosion resistance, fluid interactions and portal destinations.

pub mod explosion_resistance {
    use mod_api::{BlockPart, ExplosionResistanceEvent, LevelHost};

    pub struct Reinforced;

    // ANCHOR: explosion_resistance
    #[mod_api::sunrise_part]
    impl BlockPart for Reinforced {
        fn explosion_resistance(
            &self,
            _: &dyn LevelHost,
            _: &mut ExplosionResistanceEvent<'_>,
        ) -> Option<f32> {
            Some(5000.0)
        }
        fn explosion_resistance_is_pure(&self) -> bool {
            true
        }
    }
    // ANCHOR_END: explosion_resistance
}

pub mod fluid_interactions {
    use mod_api::{BlockState, FluidState, ServerHost};

    // ANCHOR: makes_stone
    // Does lava here, over `below`, become something beside any of these neighbours?
    pub fn makes_stone(
        host: &dyn ServerHost,
        lava: FluidState,
        around: [BlockState; 5],
        below: BlockState,
    ) -> Option<BlockState> {
        around
            .into_iter()
            .find_map(|neighbour| host.fluid_interaction_result(lava, neighbour, below))
    }
    // ANCHOR_END: makes_stone
}

pub mod portal_destination {
    use mod_api::{LevelRef, PortalError, ServerHost};

    pub fn resolve_exit(level: LevelRef, host: &mut dyn ServerHost, portal_pos: [i32; 3]) {
        // ANCHOR: find_portal_exit
        let size = [0.98, 0.98];
        match level.find_or_create_portal_exit(host, portal_pos, size) {
            Ok(exit) => {
                // `exit.level` is the destination level, `exit.pos` the bottom centre of the box there and
                // `exit.rotation` what happens to the entity's yaw and pitch (see below).
                let _ = (exit.level, exit.pos, exit.rotation);
            }
            Err(PortalError::ChunksNotLoaded) => { /* retry on a later tick */ }
            Err(_error) => { /* the other refusals are final for this call */ }
        }
        // ANCHOR_END: find_portal_exit
    }
}
