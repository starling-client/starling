//! Game modes beyond survival and creative (`GameType`): adventure players
//! cannot build, spectators fly through blocks and only watch.
use super::{Game, Screen};
use sunrise_client::{collisions::SceneCollisions, command::args::GameMode, items::ItemStack};
use sunrise_world::world_movement::in_range_of_ground;

pub(super) use sunrise_gameplay::game_modes::{
    loads_chunks, mode_from_id, projectile_predicate_supported,
};

/// `InventoryScreen.containerTick` / `CreativeModeInventoryScreen.containerTick`:
/// reconcile an open player inventory with `Player.hasInfiniteMaterials`.
fn inventory_after_infinite_materials(
    screen: Screen,
    player_inventory: bool,
    infinite_materials: bool,
) -> Screen {
    match (screen, player_inventory, infinite_materials) {
        (Screen::Inventory, true, true) => Screen::Creative,
        (Screen::Creative, _, false) => Screen::Inventory,
        _ => screen,
    }
}

impl Game<'_> {
    /// Whether `Screen::Inventory` currently shows the player's own `InventoryScreen`
    /// rather than a container, crafting table, workstation, mount, or server/mod menu.
    pub(in crate::game) fn is_player_inventory_screen(&self) -> bool {
        self.screen == Screen::Inventory
            && self.container.is_none()
            && self.station.is_none()
            && self.craft_size == 2
            && !self.mount_active()
            && self.mod_screen.is_none()
            && self.remote_menu_type().is_none()
    }

    /// Reconcile an open player inventory on its screen tick (`InventoryScreen.containerTick`
    /// and `CreativeModeInventoryScreen.containerTick`) against live `instabuild`, keeping
    /// the shared cursor (`carried`) stack across the switch.
    pub(in crate::game) fn tick_inventory_screen(&mut self) {
        if self.player.is_none() {
            return;
        }
        let next = inventory_after_infinite_materials(
            self.screen,
            self.is_player_inventory_screen(),
            self.instabuild(),
        );
        if next == self.screen {
            return;
        }
        match next {
            Screen::Creative => {
                self.set_screen(Screen::Creative);
                if self.screen == Screen::Creative {
                    self.refresh_creative_tabs();
                    self.creative_screen.reopen();
                    self.refresh_saved_hotbars();
                }
            }
            Screen::Inventory => {
                self.set_screen(Screen::Inventory);
            }
            _ => {}
        }
    }

    /// `MultiPlayerGameMode.isSpectator()` on a client; `ServerPlayer.isSpectator()` on a server.
    pub(in crate::game) fn spectator(&self) -> bool {
        self.mode == GameMode::Spectator
    }
    /// `MultiPlayerGameMode.canHurtPlayer()` and `hasExperience()` (`localPlayerMode.isSurvival()`).
    pub(in crate::game) fn can_hurt_player(&self) -> bool {
        matches!(self.mode, GameMode::Survival | GameMode::Adventure)
    }
    /// `MultiPlayerGameMode.hasMissTime()` (`!localPlayerMode.isCreative()`).
    pub(in crate::game) fn has_miss_time(&self) -> bool {
        self.mode != GameMode::Creative
    }
    /// `AbstractClientPlayer.gameMode()` on a remote client (`PlayerInfo.getGameMode()` for our UUID), else `Some(self.mode)`.
    pub(in crate::game) fn player_info_game_mode(&self) -> Option<GameMode> {
        match self.remote.as_ref() {
            Some(r) => r.own_player_info_game_mode().map(mode_from_id),
            None => Some(self.mode),
        }
    }
    /// `Player.isSpectator()`: reads `PlayerInfo` on a remote client and `self.mode` locally.
    pub(in crate::game) fn own_player_is_spectator(&self) -> bool {
        self.player_info_game_mode() == Some(GameMode::Spectator)
    }
    /// `Player.isCreative()`: reads `PlayerInfo` on a remote client and `self.mode` locally.
    pub(in crate::game) fn own_player_is_creative(&self) -> bool {
        self.player_info_game_mode() == Some(GameMode::Creative)
    }
    /// `Abilities.mayBuild`.
    pub(super) fn may_build(&self) -> bool {
        if let Some(a) = self.remote_abilities() {
            return a.may_build;
        }
        self.acting_abilities.map_or(
            matches!(self.mode, GameMode::Survival | GameMode::Creative),
            |a| a.may_build,
        )
    }
    /// `Abilities.instabuild`. A client reads its live `Remote::abilities`.
    pub(in crate::game) fn instabuild(&self) -> bool {
        if let Some(a) = self.remote_abilities() {
            return a.instabuild;
        }
        self.acting_abilities
            .map_or(self.creative, |a| a.instabuild)
    }
    pub(super) fn mode_id(&self) -> i32 {
        self.mode.id()
    }
    pub(super) fn set_mode_id(&mut self, id: i32) {
        self.set_mode(mode_from_id(id));
    }
    /// `MultiPlayerGameMode.setLocalMode(GameType)`: updates only the local mode and `Abilities`.
    pub(in crate::game) fn set_local_mode(&mut self, mode: GameMode) {
        if mode != self.mode {
            self.previous_mode = Some(self.mode);
        }
        self.mode = mode;
        self.update_world_options_personal_mode(mode.id());
        if let (Some(r), Some(p)) = (self.remote.as_mut(), self.player.as_ref()) {
            r.sync_body_flight(p.flying, p.flying_speed);
        }
        self.finish_local_mode_update(mode);
    }
    /// `MultiPlayerGameMode.setLocalMode(GameType, @Nullable GameType)` for `Login` and `Respawn`.
    pub(in crate::game) fn set_local_mode_with_previous(
        &mut self,
        mode: GameMode,
        previous: Option<GameMode>,
    ) {
        self.mode = mode;
        self.previous_mode = previous;
        self.finish_local_mode_update(mode);
    }
    fn finish_local_mode_update(&mut self, mode: GameMode) {
        let r = self
            .remote
            .as_mut()
            .expect("set_local_mode requires a remote client");
        self.creative = r.update_local_mode_abilities(mode.id());
        self.apply_remote_abilities();
        let local_spectator = self.spectator();
        if let Some(player) = self.player.as_mut() {
            player.local_spectator = local_spectator;
        }
    }
    /// `ServerPlayer.setGameMode` on a local/server world.
    pub(super) fn set_mode(&mut self, mode: GameMode) {
        debug_assert!(
            self.remote.is_none(),
            "set_mode is server-only; remote clients use set_local_mode"
        );
        let changed = mode != self.mode;
        if changed {
            self.previous_mode = Some(self.mode);
        }
        self.mode = mode;
        self.creative = mode == GameMode::Creative;
        // GameType.updatePlayerAbilities applies at once.
        if let Some(a) = self.acting_abilities.as_mut() {
            a.update_for_mode(mode.id());
        }
        if let Some(p) = self.player.as_mut() {
            p.invulnerable = mode == GameMode::Creative || mode == GameMode::Spectator;
            p.may_fly = p.invulnerable;
        }
        if mode == GameMode::Spectator {
            self.mining = None;
            self.stop_active_item_use();
            self.set_mapping_held("key.attack", false);
            self.dismount_for_spectator();
            if let Some(p) = self.player.as_mut() {
                p.flying = true;
                p.spectator = true;
                p.local_spectator = true;
            }
        } else if let Some(p) = self.player.as_mut() {
            p.spectator = false;
            p.local_spectator = false;
            if !self.creative {
                p.flying = false;
            }
        }
        // ServerPlayerGameMode.changeGameModeForPlayer: a player left flying
        // by the change lands when the ground is within a block.
        let flying = self
            .player
            .as_ref()
            .filter(|p| p.flying)
            .map(|p| p.bounds());
        if changed
            && mode != GameMode::Spectator
            && let Some(bounds) = flying
        {
            let mut scene = SceneCollisions {
                world: &self.world,
                binding: self.collision,
                subject: None,
            };
            if in_range_of_ground(&mut scene, bounds)
                && let Some(p) = self.player.as_mut()
            {
                p.flying = false;
                if let Some(a) = self.acting_abilities.as_mut() {
                    a.flying = false;
                }
            }
        }
    }
}
impl Game<'_> {
    /// `ItemStack.canBreakBlockInAdventureMode` / `canPlaceOnBlockInAdventureMode`
    /// for the main-hand stack: some predicate of its `can_break` or
    /// `can_place_on` component (`AdventureModePredicate`: one block predicate
    /// or a list) matches the block at `pos`, as `BlockPredicate.matches(BlockInWorld)`
    /// tests it: the state, then `nbt` against the block entity's
    /// `saveWithFullMetadata` (no block entity fails). `components` is not
    /// looked at on this path.
    fn adventure_predicate(&self, component: &str, pos: [i32; 3]) -> bool {
        self.held()
            .is_some_and(|stack| self.stack_adventure_predicate(stack, component, pos))
    }
    /// `adventure_predicate` for any `stack`, not only the main hand's.
    pub(in crate::game) fn stack_adventure_predicate(
        &self,
        stack: &ItemStack,
        component: &str,
        pos: [i32; 3],
    ) -> bool {
        let Some(tag) = stack.data().and_then(|d| d.other(component)) else {
            return false;
        };
        self.adventure_predicate_at(tag, pos)
    }
    /// The existing supported block/state portion of `AdventureModePredicate.test`.
    /// The held-item wrapper and the projectile caller supply their own live value.
    pub(super) fn adventure_predicate_at(
        &self,
        tag: &sunrise_core::nbt::Tag,
        pos: [i32; 3],
    ) -> bool {
        use sunrise_core::nbt::Tag;
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let name = &self.res.facts().get(state).name;
        let properties = self
            .pack
            .assets
            .registry
            .state(state)
            .map(|s| &s.properties);
        let matches = |p: &Tag| {
            let block_ok = match p.get("blocks") {
                None => true,
                Some(Tag::String(id)) => self.block_in_holder_set(&id.to_string_lossy(), name),
                Some(Tag::List(ids)) => ids.iter().any(|id| match id {
                    Tag::String(id) => self.block_in_holder_set(&id.to_string_lossy(), name),
                    _ => false,
                }),
                Some(_) => false,
            };
            // StatePropertiesPredicate: every property exact, or in a min/max range.
            let state_ok = match p.get("state") {
                None => true,
                Some(Tag::Compound(want)) => want.iter().all(|(key, matcher)| {
                    let Some(have) = properties.and_then(|p| p.get(&key.to_string_lossy())) else {
                        return false;
                    };
                    match matcher {
                        Tag::String(value) => *have == value.to_string_lossy(),
                        Tag::Compound(_) => {
                            let bound = |k: &str| match matcher.get(k) {
                                Some(Tag::String(v)) => Some(v.to_string_lossy()),
                                _ => None,
                            };
                            let in_range = |have: &str, v: Option<String>, above: bool| {
                                v.is_none_or(|v| match (have.parse::<i64>(), v.parse::<i64>()) {
                                    (Ok(h), Ok(b)) if above => h >= b,
                                    (Ok(h), Ok(b)) => h <= b,
                                    _ => false,
                                })
                            };
                            in_range(have, bound("min"), true)
                                && in_range(have, bound("max"), false)
                        }
                        _ => false,
                    }
                }),
                Some(_) => false,
            };
            // NbtPredicate (TagParser.LENIENT_CODEC: SNBT or a compound),
            // NbtUtils.compareNbt(pattern, data, true).
            let nbt_ok = || match p.get("nbt") {
                None => true,
                Some(Tag::String(snbt)) => sunrise_core::nbt::parse_snbt(&snbt.to_string_lossy())
                    .is_ok_and(|(pattern, _)| {
                        sunrise_client::nbt_path::compare_nbt(
                            &pattern,
                            self.block_entity_nbt(pos).as_ref(),
                        )
                    }),
                Some(pattern @ Tag::Compound(_)) => sunrise_client::nbt_path::compare_nbt(
                    pattern,
                    self.block_entity_nbt(pos).as_ref(),
                ),
                Some(_) => false,
            };
            block_ok && state_ok && nbt_ok()
        };
        match tag {
            Tag::List(list) => list.iter().any(matches),
            single => matches(single),
        }
    }
    /// A `HolderSet<Block>` entry: a block id or a `#tag`.
    fn block_in_holder_set(&self, entry: &str, block: &str) -> bool {
        let namespaced = |id: &str| {
            if id.contains(':') {
                id.to_owned()
            } else {
                format!("minecraft:{id}")
            }
        };
        match entry.strip_prefix('#') {
            Some(tag) => self.res.items().block_in_tag(&namespaced(tag), block),
            None => namespaced(entry) == block,
        }
    }
    /// `Player.blockActionRestricted` for the block at `pos`: spectators never
    /// act on blocks, adventure players only with a main-hand item whose
    /// `can_break` matches it.
    pub(super) fn block_action_restricted(&self, pos: [i32; 3]) -> bool {
        if self.spectator() {
            return true;
        }
        !self.may_build() && !self.can_break_in_adventure(pos)
    }
    /// `ItemStack.canBreakBlockInAdventureMode`: an adventure player may break,
    /// or prime TNT at, only a block their held stack's `can_break` matches.
    pub(super) fn can_break_in_adventure(&self, pos: [i32; 3]) -> bool {
        self.adventure_predicate("minecraft:can_break", pos)
    }
    /// `ItemStack.useOn` in adventure mode: the used item's `can_place_on`
    /// matches the clicked block (`Player.mayUseItemAt`).
    pub(super) fn can_place_on_in_adventure(&self, clicked: [i32; 3]) -> bool {
        self.adventure_predicate("minecraft:can_place_on", clicked)
    }
    /// `Player.mayUseItemAt(pos, face, stack)` for a player who `may_build`
    /// or not: builders always may, anyone else only when `stack`'s
    /// `can_place_on` matches the block clicked, the one `pos` lies against.
    pub(super) fn may_use_item_at(
        &self,
        may_build: bool,
        pos: [i32; 3],
        face: sunrise_core::geometry::Direction,
        stack: &ItemStack,
    ) -> bool {
        may_build
            || self.stack_adventure_predicate(
                stack,
                "minecraft:can_place_on",
                relative(pos, face.opposite()),
            )
    }
    /// `ItemStack.canPlaceOnBlockInAdventureMode` of the stack a use names.
    pub(super) fn stack_can_place_on_in_adventure(
        &self,
        stack: &ItemStack,
        clicked: [i32; 3],
    ) -> bool {
        self.stack_adventure_predicate(stack, "minecraft:can_place_on", clicked)
    }
    /// `ItemStack.useOn`'s gate before any item's own `useOn`: a player who
    /// may build, or an adventure player whose held item's `can_place_on`
    /// matches the clicked block.
    pub(super) fn may_use_on(&self, clicked: [i32; 3]) -> bool {
        self.may_build() || self.can_place_on_in_adventure(clicked)
    }
    /// `ServerPlayer.setGameMode(SPECTATOR)`: `stopRiding`.
    fn dismount_for_spectator(&mut self) {
        let Some(id) = self.riding else {
            return;
        };
        if let Some(i) = self.boats.iter().position(|b| b.id == id) {
            self.dismount(i);
        } else if let Some(i) = self.carts.iter().position(|c| c.id == id) {
            self.dismount_cart(i);
        } else {
            self.riding = None;
        }
    }
}

// BlockPos.relative uses Java's wrapping integer arithmetic, including for mod input.
fn relative(pos: [i32; 3], face: sunrise_core::geometry::Direction) -> [i32; 3] {
    let [dx, dy, dz] = face.normal();
    [
        pos[0].wrapping_add(dx),
        pos[1].wrapping_add(dy),
        pos[2].wrapping_add(dz),
    ]
}

#[cfg(test)]
#[path = "tests/adventure_gate_tests.rs"]
mod adventure_gate_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn face_offsets_wrap_at_integer_boundaries() {
        use sunrise_core::geometry::Direction::*;
        for (face, axis, positive) in [
            (East, 0, true),
            (West, 0, false),
            (Up, 1, true),
            (Down, 1, false),
            (South, 2, true),
            (North, 2, false),
        ] {
            for boundary in [i32::MIN, i32::MAX] {
                let mut pos = [5; 3];
                pos[axis] = boundary;
                let mut expected = pos;
                expected[axis] = match (positive, boundary) {
                    (true, i32::MAX) => i32::MIN,
                    (false, i32::MIN) => i32::MAX,
                    (true, _) => i32::MIN + 1,
                    (false, _) => i32::MAX - 1,
                };
                assert_eq!(
                    super::relative(pos, face),
                    expected,
                    "{face:?} at {boundary}"
                );
            }
        }
    }

    use super::{
        Screen, inventory_after_infinite_materials, loads_chunks, projectile_predicate_supported,
    };

    // Added after the frozen old Game proof: unsupported conjuncts must not
    // become permission from their otherwise matching blocks portion.
    #[test]
    fn projectile_predicates_require_a_complete_supported_match() {
        for extra in [
            "nbt:{marker:1}",
            "components:{}",
            "predicates:{}",
            "state:{age:{min:1}}",
        ] {
            let text = format!("{{blocks:\"minecraft:chorus_flower\",{extra}}}");
            let (tag, _) = sunrise_core::nbt::parse_snbt(&text).unwrap();
            assert!(!projectile_predicate_supported(&tag), "{text}");
        }
        for text in ["7", "[]", "{blocks:[7]}"] {
            let (tag, _) = sunrise_core::nbt::parse_snbt(text).unwrap();
            assert!(!projectile_predicate_supported(&tag), "{text}");
        }
        let (tag, _) = sunrise_core::nbt::parse_snbt(
            "{blocks:\"minecraft:chorus_flower\",state:{age:{min:\"0\",max:\"5\"}}}",
        )
        .unwrap();
        assert!(projectile_predicate_supported(&tag));
    }

    /// `InventoryScreen.containerTick` / `CreativeModeInventoryScreen.containerTick`:
    /// both directions follow `hasInfiniteMaterials` only for player-inventory screens,
    /// leaving container menus and other screens alone.
    #[test]
    fn open_inventory_reconciles_both_directions_with_infinite_materials() {
        assert_eq!(
            inventory_after_infinite_materials(Screen::Inventory, true, true),
            Screen::Creative
        );
        assert_eq!(
            inventory_after_infinite_materials(Screen::Inventory, true, false),
            Screen::Inventory
        );
        assert_eq!(
            inventory_after_infinite_materials(Screen::Inventory, false, true),
            Screen::Inventory
        );
        assert_eq!(
            inventory_after_infinite_materials(Screen::Inventory, false, false),
            Screen::Inventory
        );
        assert_eq!(
            inventory_after_infinite_materials(Screen::Creative, false, false),
            Screen::Inventory
        );
        assert_eq!(
            inventory_after_infinite_materials(Screen::Creative, false, true),
            Screen::Creative
        );
        for screen in [
            Screen::Playing,
            Screen::Paused,
            Screen::Stats,
            Screen::Advancements,
        ] {
            assert_eq!(
                inventory_after_infinite_materials(screen, false, true),
                screen
            );
            assert_eq!(
                inventory_after_infinite_materials(screen, false, false),
                screen
            );
        }
    }

    /// `ChunkMap.skipPlayer`: only a spectator with `spectators_generate_chunks`
    /// off stops loading chunks around itself.
    #[test]
    fn a_skipped_spectator_loads_no_chunks() {
        assert!(!loads_chunks(false, true), "spectator, rule off");
        assert!(loads_chunks(true, true), "spectator, rule on");
        assert!(
            loads_chunks(false, false),
            "survival, creative or adventure"
        );
        assert!(
            loads_chunks(true, false),
            "survival, creative or adventure, rule on"
        );
    }
}
