//! A block a native mod writes for a player (`player.place_block`): the level write with a
//! placement's flags, then `Block.setPlacedBy` in the player's turn.
use super::Game;
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockStateId;
use mod_api::{BlockUpdateFlags, LevelRef, LevelWriteError, PlayerRef, PlayerWorldRefusal};

impl Game<'_> {
    /// `player.place_block`: `state` written at `pos` in `level` as a placement writes it, then
    /// the block's `setPlacedBy` (a bed's head, a door's upper half) with no placer, with the
    /// player's state in place. No placement rule, trigger, sound or item use.
    pub(super) fn player_place_block(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        update_neighbour_shapes: bool,
    ) -> Result<(), PlayerWorldRefusal> {
        if !self.is_logical_server() {
            return Err(PlayerWorldRefusal::Client);
        }
        let mut placed = Err(PlayerWorldRefusal::Stale);
        let ran = self.native_as_player_handle(player, |game| {
            placed = game.place_for_acting_player(level, pos, state, update_neighbour_shapes);
        });
        if !ran {
            // The handle reads, so it was the player's death that kept it from running.
            return Err(match self.player_handle_read(player) {
                Some(_) => PlayerWorldRefusal::Dead,
                None => PlayerWorldRefusal::Stale,
            });
        }
        placed
    }

    fn place_for_acting_player(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        update_neighbour_shapes: bool,
    ) -> Result<(), PlayerWorldRefusal> {
        if !self.level_handle_current(level) {
            return Err(PlayerWorldRefusal::StaleLevel);
        }
        if self.dim.key() != level.dimension() {
            return Err(PlayerWorldRefusal::OtherLevel);
        }
        if !self.world.shape.contains_y(pos[1]) {
            return Err(PlayerWorldRefusal::OutOfWorld { pos });
        }
        if !self.world.loaded(pos[0], pos[2]) {
            return Err(PlayerWorldRefusal::NotLoaded { pos });
        }
        // BlockItem.placeBlock's flags: 11, and the neighbours' shapes kept as they are when
        // the mod has set them itself.
        let mut flags = BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS;
        if !update_neighbour_shapes {
            flags = flags | BlockUpdateFlags::KNOWN_SHAPE;
        }
        match self.level_set_block(level, pos, state, flags) {
            Ok(true) => {}
            Ok(false) => return Err(PlayerWorldRefusal::Unchanged),
            Err(LevelWriteError::Stale) => return Err(PlayerWorldRefusal::StaleLevel),
            Err(LevelWriteError::InvalidState(_)) => return Err(PlayerWorldRefusal::UnknownState),
            Err(_) => return Err(PlayerWorldRefusal::OtherLevel),
        }
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.set_placed_by(level, ticks, at, state));
        let _ = self.process_level_events();
        Ok(())
    }
}
