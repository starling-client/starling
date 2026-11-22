use super::Game;
use sunrise_core::geometry::Vec3;

impl Game<'_> {
    pub(super) fn remote_move_entity_and_file(
        &mut self,
        id: i32,
        path: Option<super::Path>,
        rotation: Option<(f32, f32)>,
        on_ground: bool,
    ) {
        self.move_entity(id, path, rotation, on_ground, super::MovePacket::Move);
        self.entity_section_moved_id(id as u32);
    }

    pub(super) fn remote_position_sync_and_file(
        &mut self,
        id: i32,
        path: super::Path,
        yaw: f32,
        pitch: f32,
        on_ground: bool,
    ) {
        self.move_entity(
            id,
            Some(path),
            Some((yaw, pitch)),
            on_ground,
            super::MovePacket::PositionSync,
        );
        self.entity_section_moved_id(id as u32);
    }

    /// The driven vehicle put at `position` with `rotation`, its section crossing filed.
    pub(in crate::game) fn remote_move_vehicle_and_file(
        &mut self,
        position: [f64; 3],
        rotation: (f32, f32),
    ) {
        self.remote_move_vehicle(position, rotation);
        let boat = self
            .riding
            .filter(|&id| self.controls_boat(id) && self.boats.iter().any(|b| b.id == id));
        if let Some(id) = self.driven_mount().map(|m| m.id).or(boat) {
            self.entity_section_moved_id(id);
        }
    }

    pub(super) fn remote_entity_section_step(&mut self, id: u32, from: Vec3, to: Vec3) {
        self.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
    }
}

pub(super) fn player_teleport(game: &mut Game<'_>, had_player: bool, from: Vec3, to: Vec3) {
    if had_player && let Ok(id) = u32::try_from(game.own_entity_id()) {
        game.remote_entity_section_step(id, from, to);
    }
}

pub(super) fn tick_items(
    game: &mut Game<'_>,
    entered: &mut Vec<sunrise_client::entities::ItemSplash>,
) {
    let mut items = game.dropped.lift();
    let mut i = 0;
    while i < items.len() {
        let (_, discarded) =
            game.tick_item_with_block_contacts(&mut items[i], None, &[], true, &mut |splash| {
                entered.push(splash)
            });
        let from = items[i].previous;
        let to = items[i].position;
        if !discarded {
            let live = game.entity_section_step_visible(
                items[i].id,
                [from.x, from.y, from.z],
                [to.x, to.y, to.z],
            );
            if !live {
                items.remove(i);
                continue;
            }
        }
        if discarded {
            items.remove(i);
        } else {
            i += 1;
        }
    }
    game.put_back_dropped(items);
}
