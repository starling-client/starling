//! A client block notification is not a save mutation.
use super::Game;
use sunrise_client::world::World;
use sunrise_core::position::BlockPos;

impl Game<'_> {
    pub(super) fn notify_block_clients(&mut self, pos: (i32, i32, i32)) {
        notify_block_clients(&mut self.world, pos);
    }
}

/// `ServerLevel.sendBlockUpdated`: mark the chunk for client updates and invalidate the path type.
pub(super) fn notify_block_clients(world: &mut World, pos: (i32, i32, i32)) {
    world.path_type_block_updated(BlockPos::new(pos.0, pos.1, pos.2));
    if let Some(log) = world.block_log.as_mut()
        && !log.contains(&pos)
    {
        log.push(pos);
    }
}
