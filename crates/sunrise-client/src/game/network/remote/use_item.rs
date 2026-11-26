use super::*;

impl Game<'_> {
    /// MultiPlayerGameMode.useItem for `hand`, if it holds something: whether
    /// the click ends there, as far as this client can tell (`use_ends_click`).
    pub(in crate::game) fn remote_use_item(&mut self, hand: Hand) -> bool {
        // MultiPlayerGameMode.useItem: spectators pass before sending or predicting.
        if self.spectator() {
            return false;
        }
        let Some(held) = self.with_hand(hand, |g| g.held().cloned()) else {
            return false;
        };
        let seq = self.next_sequence();
        self.send(S::UseItem(play::UseItem {
            hand: hand_id(hand),
            sequence: seq,
            yaw: self.yaw,
            pitch: self.pitch,
        }));
        let Some(info) = self.items().get(&held.item).cloned() else {
            return false;
        };
        // MultiPlayerGameMode.useItem's prediction: an item cooling down
        // passes (its packet still goes).
        let group = sunrise_client::cooldowns::group(&held, info.use_cooldown.as_ref());
        if self.cooldowns.is_on_cooldown(&group) {
            return false;
        }
        if let Some(handled) = self.client_item_interaction(hand, &held) {
            return handled;
        }
        // a mod's item used over time starts here too, as the server starts it.
        if self.with_hand(hand, |g| g.start_mod_use(&held)) {
            return true;
        }
        // Item.use: Equippable.swapWithEquipmentSlot runs on this client too, so
        // the armor slot and the hand change at once (the server confirms).
        if info.equip_slot.is_some() && self.with_hand(hand, |g| g.equip_from_hand(&held)) {
            // Equippable.swapWithEquipmentSlot's SUCCESS (SwingSource.CLIENT).
            self.with_hand(hand, Game::swing_arm);
            return true;
        }
        // `ItemStack.use` runs on this client too: a bucket's own ray and its
        // pick/empty contents decide whether the click succeeds (a failed
        // fill or pickup leaves the hand alone), and the blocks it sets are
        // this use's prediction.
        if &*held.item == "minecraft:bucket"
            || sunrise_client::items::bucket_contents(&held.item).is_some()
        {
            let (succeeded, changes, sounds) = self.with_hand(hand, |g| g.remote_use_bucket(&held));
            if let Some(changes) = changes {
                self.predict_blocks(&changes, seq);
            }
            // `BucketItem.emptyContents` / evaporate: `playSound(player, …)` on
            // this client; the server leaves the actor out (#4684).
            self.play_predicted_sounds(&sounds);
            // BucketItem.use's SUCCESS (SwingSource.CLIENT) on this client.
            if succeeded {
                self.with_hand(hand, Game::swing_arm);
            }
            return succeeded;
        }
        // SpawnEggItem.use on the client succeeds after a SOURCE_ONLY block
        // hit; the server alone tests LiquidBlock and the placement permission.
        if held.item.ends_with("_spawn_egg") {
            let succeeded = self.bucket_hit().is_some()
                && sunrise_client::block_entity_item::item_entity_data(&held, self.res.items())
                    .is_some_and(|data| self.spawn_egg_allowed(&data.kind));
            if succeeded {
                self.with_hand(hand, Game::swing_arm);
            }
            return succeeded;
        }
        // KnowledgeBookItem.use on this client: the book is used up either
        // way. With recipes it is SUCCESS (swings, ends the click); with none
        // it is FAIL, and the other hand gets its turn. The recipes go as
        // their NBT on the wire, and the item's own default is an empty list,
        // so a stack whose patch sets them has some.
        if &*held.item == "minecraft:knowledge_book" {
            /// `DataComponents.RECIPES`' registry id (26.3).
            const RECIPES: i32 = 68;
            let has_recipes = held
                .wire_patch()
                .is_some_and(|p| sunrise_protocol::item::WirePatch::adds(&p, RECIPES))
                || held
                    .data()
                    .and_then(|d| d.other("minecraft:recipes"))
                    .is_some_and(|t| matches!(t, Tag::List(l) if !l.is_empty()));
            self.with_hand(hand, Game::consume_held);
            if has_recipes {
                self.apply_use_cooldown(&held);
                self.with_hand(hand, Game::swing_arm);
            }
            return has_recipes;
        }
        // Food starts being eaten (Consumable.startConsuming). A food that
        // can't be eaten (full and not always-edible) fails the use
        // (InteractionResult.FAIL): startUseItem tries the other hand, and
        // no cooldown or use animation starts.
        let eating = self.with_hand(hand, |g| g.start_eating(&info));
        if info.food.is_some() && !eating {
            return false;
        }
        // A bucket, a glass bottle or a boat with nothing to fill, empty or
        // place PASSes or FAILs: startUseItem goes on to the off hand (a
        // Riptide trident there charges).
        if self.with_hand(hand, |g| g.use_finds_no_target(&held)) {
            return false;
        }
        // It runs the use here too: ItemStack.use starts the `use_cooldown`
        // of an item with no use time, and the server's
        // ClientboundCooldownPacket confirms it; one held in use starts it at
        // finishUsingItem (remote_complete_using).
        if !info.consumable.as_ref().is_some_and(|c| c.ticks() > 0) {
            self.apply_use_cooldown(&held);
        }
        if self.with_hand(hand, |g| g.remote_use_book(&held)) {
            // WritableBookItem.use's SUCCESS (SwingSource.CLIENT).
            self.with_hand(hand, Game::swing_arm);
            return true;
        }
        // BowItem.use and CrossbowItem.use FAIL with nothing to shoot: no
        // itemUsed (the hand doesn't dip) and the other hand's turn comes.
        if !self.with_hand(hand, |g| g.remote_start_held_use(&held)) {
            return false;
        }
        let gliding = self.player.as_ref().is_some_and(|p| p.fall_flying);
        // FireworkRocketItem.use while gliding: SUCCESS (SwingSource.PREDICTED),
        // so startUseItem swings this hand itself; a vanilla server swings it
        // for the other players only.
        if gliding && &*held.item == "minecraft:firework_rocket" {
            self.with_hand(hand, Game::swing_arm);
        }
        // A stack's own `blocks_attacks` raises it as a shield's does; a
        // use the server's patch gives it is CONSUME too (no swing).
        let blocks = self.blocks_attacks(&held).is_some() || self.patched_held_use(&held);
        let ends = use_ends_click(&info, gliding) || blocks;
        // MultiPlayerGameMode.useItem: `ItemStack.use`'s SUCCESS swings
        // (SwingSource.CLIENT); CONSUME does not — a bow, crossbow, trident
        // or spear starting its charge, and `ItemUtils.startUsingInstantly`'s
        // food, drink, spyglass, shield and goat horn.
        if ends && !blocks && remote_use_swings(&held, info.consumable.is_some()) {
            self.with_hand(hand, Game::swing_arm);
        }
        ends
    }
    /// `LivingEntity.updatingUsingItem` on a server client: an item held in
    /// use (`held_until_released`), and a consumable's eating animation and
    /// its crumbs until the server's USE_ITEM_COMPLETE. The server sends the
    /// stack, its hunger and the sounds.
    pub(in crate::game) fn remote_tick_using(&mut self) {
        self.tick_client_item_use();
        let Some((slot, ticks)) = self.using else {
            return;
        };
        // The may-still-use check (`ItemStack.isSameItem`, then a part's
        // `canContinueUsing`) reads the active hand, even when that hand is now
        // holding a different hotbar slot.
        let offhand = slot == crate::game::shield::OFFHAND_SLOT;
        let (hand_slot, hand) = if offhand {
            (slot, Hand::Off)
        } else {
            (self.selected, Hand::Main)
        };
        let held = self.stack_in(hand_slot).cloned();
        let began = self.use_start.item.clone();
        let goes_on = began
            .as_ref()
            .zip(held.as_ref())
            .is_some_and(|(began, held)| {
                self.can_continue_using(began, held, Some(self.own_entity_id()), hand)
            });
        let Some(stack) = held.filter(|_| goes_on) else {
            self.using = None;
            self.use_start = Default::default();
            // Another item stops the use (`stopUsingItem`); an emptied hand is used up.
            if let Some(began) = began.filter(|_| self.stack_in(hand_slot).is_some()) {
                let time_left = ticks_left(self.use_duration, ticks);
                self.stopped_using(&began, Some(self.own_entity_id()), hand, time_left);
            }
            return;
        };
        self.use_start.item = Some(stack.clone());
        self.using = Some((hand_slot, ticks));
        // A client cancel skips this tick's effects; the countdown and release continue.
        if !self.use_item_ticked(&stack, ticks) {
            self.using = Some((hand_slot, ticks.saturating_add(1)));
            return;
        }
        // BrushItem.onUseTick: with no block aimed the use ends (the server
        // ends it too); a stroke shows its dust. Its sounds are the server's.
        if &*stack.item == "minecraft:brush" {
            let elapsed = 201i32.wrapping_sub(ticks_left(self.use_duration, ticks));
            let hit = self.pick();
            if let Some(hit) = &hit
                && elapsed % 10 == 5
            {
                self.brush_dust(hit, offhand);
            }
            if hit.is_none() {
                self.send_action(6);
            }
            self.using = hit.and(Some((hand_slot, ticks + 1)));
            return;
        }
        // A bundle's 200-tick use ends with the server's entity event 9, not here.
        if self.items().get(&stack.item).is_some_and(|i| i.bundle) {
            self.using = Some((hand_slot, ticks + 1));
            return;
        }
        // A drawn bow, a charging crossbow, a raised shield...: held until
        // the button lets go.
        if self.held_until_released(&stack) {
            if self.mod_use(&stack) || self.blocks_attacks(&stack).is_some() {
                self.item_on_use_tick(&stack, ticks);
            }
            self.using = Some((hand_slot, ticks + 1));
            return;
        }
        // A goat horn is held up for its instrument's use duration.
        if self.items().instrument(&stack).is_some() {
            self.using = Some((hand_slot, ticks + 1));
            return;
        }
        let Some(consumable) = self.consumable(&stack) else {
            self.using = None;
            return;
        };
        // Consumable.startConsuming eats one with no use time at once, without
        // a use (nor a USE_ITEM_COMPLETE).
        if consumable.ticks() <= 0 {
            self.using = None;
            return;
        }
        // Consumable.shouldEmitParticlesAndSounds, as in tick_used_item. The
        // countdown goes on past zero (LivingEntity.updateUsingItem completes
        // only on the server) until the server's USE_ITEM_COMPLETE.
        // getUseItemRemainingTicks: the duration this use started with.
        let remaining = self.use_duration - ticks as i32;
        if remaining > 0 && consumable.should_emit(remaining) {
            // `emit_consume` plays the eat sound and spawns the bites (when the
            // item has consume particles), as `tick_used_item` does on the
            // player's own server.
            self.emit_consume(&consumable, &stack, 5);
        }
        self.item_on_use_tick(&stack, ticks);
        self.using = Some((hand_slot, ticks + 1));
    }
}
