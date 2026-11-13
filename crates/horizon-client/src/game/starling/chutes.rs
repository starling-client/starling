//! Create's chute where this game simulates the world (Starling; ported from Create 6.0.10, MIT,
//! licenses/create-MIT.txt): ChuteBlockEntity and ChuteBlock without fans' air in them. An item
//! falls through a chute a fifth of a block a tick; an empty one takes up to 16 of an item from
//! the inventory above (not from a chute: that one passes its own down) and the items landing on
//! its top. At its bottom it gives its item to the inventory below, the chute it leads into
//! (below, or below and ahead for a diagonal one), else drops it below unless something solid is
//! there; while it can do none, the item waits half way.
use super::Game;
use super::machine_entities::Kind;
use horizon_client::items::ItemStack;
use horizon_core::geometry::Direction;

/// The item's height in the chute (`itemPosition`, 0 to 1), kept in thousandths.
const TOP: i32 = 1000;
const MIDDLE: i32 = 500;
/// `getItemMotion` with no air in it: gravity's 4 blocks a second.
const FALL: i32 = 200;
/// `getExtractionAmount`.
const EXTRACT: u32 = 16;
/// ItemEntity.setDefaultPickUpDelay.
const PICKUP_DELAY: u32 = 10;

impl Game<'_> {
    /// A chute's facing (`down`, or the way a diagonal one leads) and whether it is open at the
    /// top (`isOpen`: normal or window).
    fn chute(&self, pos: [i32; 3]) -> Option<([i32; 3], bool)> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let s = self.pack.assets.registry.state(state)?;
        if !s.name.ends_with("chute") {
            return None;
        }
        let facing = horizon_client::kinetics::direction(s.properties.get("facing")?)?;
        let open = matches!(
            s.properties.get("shape").map(String::as_str),
            Some("normal" | "window") | None
        );
        Some((facing, open))
    }

    /// The chute's inventory (`ChuteItemHandler.insertItem`): a stack in when it is empty, at its
    /// top. What did not go in.
    pub(super) fn chute_insert(&mut self, pos: [i32; 3], stack: ItemStack) -> Option<ItemStack> {
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return Some(stack);
        };
        if e.items[0].is_some() {
            return Some(stack);
        }
        e.items[0] = Some(stack);
        e.timer = TOP;
        e.since = self.world.game_time;
        self.set_machine_entity(pos, &e);
        None
    }

    /// `ChuteBlockEntity.tick` for the chute at `pos`.
    pub(super) fn tick_chute(&mut self, pos: [i32; 3]) {
        let Some((facing, open)) = self.chute(pos) else {
            return;
        };
        let above = [pos[0], pos[1] + 1, pos[2]];
        // canDirectlyInsert: open, and no chute above leading into it.
        let fed = self.chute(above).is_some_and(|(f, _)| f == [0, -1, 0]);
        if open && !fed {
            self.catch_on_top(pos, Kind::Chute, 1.0);
        }
        let Some((_, mut e)) = self.machine_entity(pos) else {
            return;
        };
        let Some(item) = e.items[0].clone() else {
            // handleInputFromAbove: the inventory above, unless it is a chute.
            if self.chute(above).is_none()
                && let Some(Some(stack)) = self.inventory_extract(above, Direction::Down, EXTRACT)
            {
                e.items[0] = Some(stack);
                e.timer = TOP;
                e.since = self.world.game_time;
                self.set_machine_entity(pos, &e);
            }
            return;
        };
        let mut next = e.timer - FALL;
        // Past half way only where it can go; out at the bottom.
        if next < MIDDLE && !self.chute_has_way_down(pos, facing) {
            next = MIDDLE;
        } else if next < 0 {
            match self.chute_output(pos, facing, item.clone()) {
                None => {
                    e.items[0] = None;
                    e.timer = 0;
                    self.set_machine_entity(pos, &e);
                    return;
                }
                Some(rest) => {
                    // Some or none taken: the rest waits half way.
                    let changed = rest != item;
                    e.items[0] = Some(rest);
                    e.timer = MIDDLE;
                    if changed {
                        self.set_machine_entity(pos, &e);
                    } else {
                        self.machines.timers.insert(pos, e.timer);
                    }
                    return;
                }
            }
        }
        e.timer = next;
        self.machines.timers.insert(pos, e.timer);
    }

    /// The chute an item leaves the one at `pos` into (`getTargetChute`): below, or below and
    /// ahead for a diagonal one.
    fn chute_target(&self, pos: [i32; 3], facing: [i32; 3]) -> Option<[i32; 3]> {
        let mut at = [pos[0], pos[1] - 1, pos[2]];
        if facing[1] == 0 {
            at = [at[0] - facing[0], at[1], at[2] - facing[2]];
        }
        self.chute(at).map(|_| at)
    }

    /// Whether anything at the chute's bottom could take its item (`handleDownwardOutput`'s
    /// simulation, short of the inventory's room): an inventory below, a chute it leads into,
    /// or room to drop it.
    fn chute_has_way_down(&self, pos: [i32; 3], facing: [i32; 3]) -> bool {
        let below = [pos[0], pos[1] - 1, pos[2]];
        if self.inventory_at(below) || self.chute_target(pos, facing).is_some() {
            return true;
        }
        if facing[1] == 0 {
            return false;
        }
        let state = self.world.block(below[0], below[1], below[2]);
        let funnel_down = self.pack.assets.registry.state(state).is_some_and(|s| {
            s.name.ends_with("_funnel")
                && s.properties.get("facing").map(String::as_str) == Some("down")
        });
        !funnel_down && !self.res.facts().get(state).full_collision
    }

    /// `handleDownwardOutput`: the item into the inventory below, else the chute it leads into,
    /// else dropped below. What is left of it.
    fn chute_output(
        &mut self,
        pos: [i32; 3],
        facing: [i32; 3],
        item: ItemStack,
    ) -> Option<ItemStack> {
        let below = [pos[0], pos[1] - 1, pos[2]];
        // A full one below stops a straight chute; a diagonal one tries its chute.
        if self.chute(below).is_none()
            && let Some(rest) = self.inventory_insert(below, Direction::Up, item.clone())
            && (rest.as_ref() != Some(&item) || facing == [0, -1, 0])
        {
            return rest;
        }
        if let Some(target) = self.chute_target(pos, facing) {
            return self.chute_insert(target, item);
        }
        if facing[1] == 0 || !self.chute_has_way_down(pos, facing) {
            return Some(item);
        }
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5 - 12.0 / 16.0,
            f64::from(pos[2]) + 0.5,
        ];
        self.spawn_item(item, at, [0.0, -0.25, 0.0], PICKUP_DELAY);
        None
    }

    /// Whether something with items is at `pos` (a container or a machine, not a chute).
    fn inventory_at(&self, pos: [i32; 3]) -> bool {
        if self.chute(pos).is_some() {
            return false;
        }
        match self.machine_kind(pos) {
            Some(Kind::Depot | Kind::Millstone | Kind::Basin | Kind::Saw) => true,
            Some(_) => false,
            None => self.has_container(pos),
        }
    }
}
