use super::*;

impl Game<'_> {
    /// `Minecraft.startUseItem`'s entity turn for `hand`: Interact goes to
    /// the server, and whether the click ends there is what this client's
    /// copy of the entity answers (`MultiPlayerGameMode.interact`).
    pub(in crate::game) fn remote_use_entity(&mut self, hand: Hand) -> bool {
        // A menu this use opens (a chest minecart's) is no clicked block's.
        if let Some(r) = self.remote.as_mut() {
            r.use_target = None;
        }
        if self.uses_stored_pick() {
            let Some(mod_api::CrosshairTarget::Entity { entity, .. }) = self.crosshair_target()
            else {
                return false;
            };
            let Some([lo, hi]) = self.entity_bounding_box(entity) else {
                return false;
            };
            let Some(eye) = self.eye_position() else {
                return false;
            };
            // Client interact range: closest box point, strict bound and no server buffer.
            // Refuse this use without choosing another target or the block behind it.
            let distance_squared = (0..3)
                .map(|i| (eye[i] - eye[i].clamp(lo[i], hi[i])).powi(2))
                .sum::<f64>();
            if distance_squared >= self.entity_reach.powi(2) {
                return false;
            }
        }
        let sneaking = self.remote_shift_down();
        // A boat, minecart, item frame, armor stand or painting nearer than
        // any mob: the server uses it (AbstractBoat.interact boards it).
        if let Some(hit) = self.remote_object_hit(false)
            && self.mob_hit().is_none_or(|(_, d, _)| hit.distance < d)
            && self.before_block(hit.distance)
        {
            // Interaction.interact answers SUCCESS when it responds and
            // CONSUME otherwise: both end the click (nothing behind the box
            // is used), but only SUCCESS swings; the packet carries the hand.
            let response = self.interactions.get(&hit.id).map(|i| i.response);
            // Objects whose interact passes (paintings, empty or fixed item frames,
            // boats sneaked at or out of control, carts that pass) leave each hand's
            // turn to its item: a crossbow aimed at it shoots.
            let is_painting = self.paintings.iter().any(|p| p.id as i32 == hit.id);
            let held = self.with_hand(hand, |g| g.held().cloned());
            let frame_passes = self
                .item_frames
                .iter()
                .find(|f| f.id as i32 == hit.id)
                .is_some_and(|f| {
                    item_frame_interact_passes(f.fixed, f.item.is_some(), held.is_some())
                });
            let boat_passes = self
                .boats
                .iter()
                .enumerate()
                .find(|(_, b)| b.id as i32 == hit.id)
                .is_some_and(|(i, b)| {
                    boat_interact_passes(
                        b.chest,
                        sneaking,
                        b.out_of_control >= 60.0,
                        self.boat_has_room(i),
                    )
                });
            let (cart_passes, is_rideable_cart) = self
                .carts
                .iter()
                .find(|c| c.id as i32 == hit.id)
                .map(|c| {
                    let ridden = self.riding == Some(c.id)
                        || self.mobs.iter().any(|m| m.extra.mount == Some(c.id));
                    (
                        c.kind
                            .interact_passes(sneaking, ridden, self.can_use_game_master_blocks()),
                        c.kind == crate::game::minecarts::CartKind::Rideable,
                    )
                })
                .unwrap_or((false, false));
            // Cushion.interact on a client: PASS when sneaked at or when
            // something already sits on it, CONSUME (no swing) otherwise.
            let is_cushion = self.cushions.iter().any(|c| c.id as i32 == hit.id);
            let cushion_passes = is_cushion
                && (sneaking
                    || self.riding.is_some_and(|r| r as i32 == hit.id)
                    || self
                        .remote
                        .as_ref()
                        .is_some_and(|r| r.has_passengers(hit.id as u32)));
            // EndCrystal and EnderDragonPart keep Entity.interact's PASS.
            let crystal_passes = self.crystals.iter().any(|c| c.id as i32 == hit.id)
                || self.level_dragon_part(hit.id as u32).is_some();
            let passes = self
                .remote
                .as_ref()
                .is_some_and(|r| r.general.rows.contains_key(&hit.id))
                || is_painting
                || frame_passes
                || boat_passes
                || cart_passes
                || cushion_passes
                || crystal_passes;
            // Otherwise the main hand's turn takes it; there is no off hand turn.
            if response.is_none() && hand == Hand::Off && !passes {
                return false;
            }
            self.send(S::Interact(play::Interact {
                id: hit.id,
                hand: hand_id(hand),
                location: hit.location,
                secondary: sneaking,
            }));
            if let Some(done) = self.entity_interaction(hand, hit.id) {
                return done;
            }
            if passes {
                return false;
            }
            // Minecart.interact resets the passenger's easing baseline on
            // a successful boarding click, not on SetPassengers (commands
            // can mount a player without this interaction).
            if let Some(i) = self.carts.iter().position(|c| {
                c.id as i32 == hit.id && c.kind == crate::game::minecarts::CartKind::Rideable
            }) {
                self.carts
                    .update(i, |c| c.player_rotation_offset = c.rotation_offset);
            }
            // MinecartCommandBlock.interact on the client: the editor opens.
            if self.carts.iter().any(|c| {
                c.id as i32 == hit.id && c.kind == crate::game::minecarts::CartKind::CommandBlock
            }) {
                self.open_cart_command_block(hit.id as u32);
            }
            // Minecart.interact boards the cart and answers CONSUME on the client,
            // which ends the click without an arm swing. AbstractBoat.interact
            // answers SUCCESS there, which swings. Cushion.interact answers
            // CONSUME there too.
            if remote_object_swings(response, is_rideable_cart || is_cushion) {
                self.with_hand(hand, Game::swing_arm);
            }
            return true;
        }
        if let Some((i, distance, location)) = self.mob_hit()
            && self.before_block(distance)
        {
            let id = self.mobs[i].id as i32;
            self.send(S::Interact(play::Interact {
                id,
                hand: hand_id(hand),
                location,
                secondary: sneaking,
            }));
            if let Some(done) = self.entity_interaction(hand, id) {
                return done;
            }
            // Player.interactOn: another player passes, and so does a mob
            // the held item means nothing to: the item is used instead.
            let player = self
                .remote
                .as_ref()
                .is_some_and(|r| r.level_players.contains_key(&id));
            if !player && self.remote_mob_takes(i, hand) {
                self.remote_feed_particles(i, hand);
                // As for the objects above, only SUCCESS swings the hand: a
                // CONSUME ends the click without one.
                if self.remote_mob_swings(i, hand) {
                    self.with_hand(hand, Game::swing_arm);
                }
                return true;
            }
            if !player
                && self.with_hand(hand, |g| {
                    g.item_interact_mob(Some(crate::game::InteractTarget { id, location }))
                })
            {
                return true;
            }
        }
        false
    }
    /// MultiPlayerGameMode.useItemOn for `hand`: whether the click ends there
    /// (the block reacts, or a block item is placed).
    pub(in crate::game) fn remote_use_item_on(
        &mut self,
        hand: Hand,
        attempt: mod_api::InteractionAttempt,
        hit: Option<Hit>,
    ) -> bool {
        let held = self.with_hand(hand, |g| g.held().cloned());
        if let Some(hit) = hit.or_else(|| self.pick()) {
            use mod_api::InteractionResult as Answer;
            use mod_api::TriState;
            let spectator = self.spectator();
            // The client and server can answer RightClickBlock differently (for
            // example, a server-only listener may override sneaking). Selection
            // comes from the registration snapshot, not the client's hook answer.
            let cauldron_click = self
                .remote
                .as_ref()
                .filter(|_| !spectator && self.has_server_cauldron_handler(hit.pos, held.as_ref()))
                .map(|remote| {
                    let selected = self.selected;
                    let main = self.stack_in(selected).cloned();
                    let off = self.offhand.clone();
                    let aim = self.remote_item_use_aim();
                    let swing = remote.interaction_swing;
                    move |sequence, item_after_block| PendingCauldron {
                        hand,
                        hit,
                        attempt,
                        selected,
                        main,
                        off,
                        aim,
                        swing,
                        sequence,
                        item_after_block,
                    }
                });
            // RightClickBlock on this client, where performUseItemOn starts,
            // for a spectator too (whose use only opens menus on the server):
            // it may force the block's use or the item's useOn, or answer for
            // both.
            let (mut canceled, mut use_block, mut use_item) =
                (None, TriState::Default, TriState::Default);
            match self.right_click_block(&hit, hand, held.as_ref()) {
                crate::game::native_hooks::BlockClickAnswer::Go {
                    use_block: block,
                    use_item: item,
                } => (use_block, use_item) = (block, item),
                crate::game::native_hooks::BlockClickAnswer::Canceled(answer) => {
                    canceled = Some(answer);
                }
            }
            // The held item's onItemUseFirst on this client: the broadcast
            // event skips the item's before-block Part when canceled. Only
            // non-PASS answers end the block interaction; PASS goes
            // on to the block (a Part's answer comes from the server).
            if canceled.is_none()
                && !spectator
                && use_item.or(true)
                && let Some(stack) = held.as_ref()
            {
                canceled = self
                    .use_item_on_block(&hit, hand, Some(stack), mod_api::UsePhase::ItemBeforeBlock)
                    .filter(|answer| *answer != Answer::Pass);
            }
            // Player.isSecondaryUseActive with something in either hand
            // keeps the block from reacting by default; an explicit block
            // allow overrides that skip, and False always skips the block.
            let block_used =
                canceled.is_none() && use_block.or(!self.remote_secondary_use_skips_block());
            let item_forced = use_item == TriState::True;
            let item_used = canceled.is_none()
                && use_item.or(true)
                && (self.may_build()
                    || self.with_hand(hand, |g| g.can_place_on_in_adventure(hit.pos)))
                && held
                    .as_ref()
                    .is_none_or(|stack| self.items().get(&stack.item).is_some())
                && (item_forced
                    || held.as_ref().is_some_and(|stack| {
                        self.items().get(&stack.item).is_some_and(|info| {
                            !self
                                .cooldowns
                                .is_on_cooldown(&sunrise_client::cooldowns::group(
                                    stack,
                                    info.use_cooldown.as_ref(),
                                ))
                        })
                    }));
            let block_answer = if !spectator && block_used {
                self.use_item_on_block(&hit, hand, held.as_ref(), mod_api::UsePhase::Block)
            } else {
                None
            };
            let vanilla_block = cauldron_click.is_none() && block_used && block_answer.is_none();
            let seq = self.next_sequence();
            let packet = play::UseItemOn {
                hand: hand_id(hand),
                pos: hit.pos,
                face: i32::from(face_direction(hit.face)),
                cursor: std::array::from_fn(|axis| {
                    (hit.point[axis] - f64::from(hit.pos[axis])) as f32
                }),
                inside: hit.inside,
                world_border: false,
                sequence: seq,
            };
            // ShelfBlock.useItemOn consumes a nonempty main-hand click client-side
            // without predicting the inventory swap or swinging.
            let shelf_reacts = hand == Hand::Main
                && held.is_some()
                && vanilla_block
                && self.shelf_hit(&hit).is_some();
            let (result, changes) = if let Some(answer) = block_answer {
                if let Some(result) = crate::game::native_hooks::block_use_result(answer) {
                    (result, DryUse::default())
                } else if hand == Hand::Main {
                    self.remote_block_template(&hit, held.as_ref(), true, true)
                } else {
                    (InteractionResult::Pass, DryUse::default())
                }
            } else if hand == Hand::Main && !spectator && vanilla_block {
                self.remote_block_reacts(&hit, held.as_ref(), true)
            } else {
                // useItemOn runs for the off hand too (useWithoutItem doesn't):
                // RespawnAnchorBlock.useItemOn charges an anchor with its
                // glowstone, SUCCESS; the blocks of `client_block_answer` answer too.
                let charges = vanilla_block && self.anchor_takes(hit.pos, held.as_ref());
                let result = if charges {
                    InteractionResult::Success
                } else if let Some(answer) = (vanilla_block && !spectator)
                    .then(|| self.client_block_answer(&hit, held.as_ref()))
                    .flatten()
                {
                    answer
                } else {
                    InteractionResult::Pass
                };
                (result, DryUse::default())
            };
            if let Some(click) = cauldron_click
                && canceled.is_none_or(|answer| !answer.consumes_action() && answer != Answer::Fail)
                && !result.consumes_action()
            {
                // A terminal client answer has no continuation and takes the
                // ordinary path below. Otherwise wait before the after-block
                // hooks, which can apply item effects of their own.
                self.remote_send_carried();
                if !self.client_packet_send_canceled(&S::UseItemOn(packet.clone()))
                    && let Some(remote) = &mut self.remote
                {
                    use sunrise_protocol::buf::Codec;
                    remote.cauldron_receipt = remote.cauldron_receipt.wrapping_add(1);
                    let receipt = remote.cauldron_receipt;
                    let mut body = sunrise_core::vanilla_bytes::Writer::new();
                    body.i64(receipt);
                    play::UseItemOn::write(&packet, &mut body);
                    // The envelope carries a receipt for the *same* block-use
                    // action. Do not expose it instead of the logical UseItemOn.
                    self.send_or_hold_play(
                        S::CustomPayload(sunrise_protocol::packets::configuration::CustomPayload {
                            channel: super::super::cauldron_prediction::USE_CHANNEL.into(),
                            data: body.data,
                        }),
                        Some((receipt, click(seq, item_used))),
                    );
                }
                return true;
            }
            // AbstractCauldronBlock.useItemOn, either hand: CauldronInteractions
            // answers on the client as on the server and only the server acts,
            // so a bucket the cauldron takes is not used again (UseItem).
            // The main-hand path already asked above; only the off-hand fallback asks here.
            let result =
                if result.consumes_action() || spectator || !vanilla_block || hand == Hand::Main {
                    result
                } else {
                    match self.with_hand(hand, |game| game.cauldron_outcome(hit.pos, held.as_ref()))
                    {
                        sunrise_client::cauldron::Outcome::Success => InteractionResult::Success,
                        sunrise_client::cauldron::Outcome::Consume => InteractionResult::Consume,
                        sunrise_client::cauldron::Outcome::Pass => result,
                    }
                };
            // useItemOn on this client, unless a sneak with something in hand
            // skips it: RedStoneOreBlock's dust, and the wick smoke of a lit
            // candle an empty hand snuffs (CandleBlock.useItemOn) or a lit
            // candle cake (CandleCakeBlock.useItemOn).
            if hand == Hand::Main && !spectator && vanilla_block {
                self.redstone_ore_particles(hit.pos);
                let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                let name = self.block_name(hit.pos);
                let lit =
                    self.pack.assets.registry.state(state).is_some_and(|s| {
                        s.properties.get("lit").map(String::as_str) == Some("true")
                    });
                let snuffs = if name.ends_with("candle_cake") {
                    hit.point[1] - f64::from(hit.pos[1]) > 0.5
                } else {
                    name.ends_with("candle") && self.may_build()
                };
                if held.is_none() && lit && snuffs {
                    self.candle_smoke(hit.pos, state);
                }
            }
            let result = if !spectator
                && vanilla_block
                && !result.consumes_action()
                && self.with_hand(hand, |g| g.predict_campfire_douse(&hit, held.as_ref(), seq))
            {
                InteractionResult::Success
            } else {
                result
            };
            let item_answer = if !spectator && !result.consumes_action() && item_used {
                self.remote_item_after_block(&hit, hand, held.as_ref())
            } else {
                None
            };
            let item_used =
                item_used && !spectator && !result.consumes_action() && item_answer.is_none();
            let reacts = result.consumes_action();
            let debug_stick = item_used
                && held
                    .as_ref()
                    .is_some_and(|stack| &*stack.item == "minecraft:debug_stick");
            // ArmorStandItem.useOn checks the space in startPrediction before
            // the packet goes out (the server may spawn the stand at once).
            let stand_fits = item_used && self.remote_armor_stand_fits(&hit, held.as_ref());
            self.send(S::UseItemOn(packet));
            // MultiPlayerGameMode.performUseItemOn: a spectator's use predicts
            // nothing and ends the click (CONSUME, no swing); the server opens
            // the block's menu. A cancelled RightClickBlock answers instead,
            // as for any player (below).
            if spectator && canceled.is_none() {
                return true;
            }
            // A cancelled RightClickBlock's answer, once the packet went out
            // (startPrediction sends it whatever the use): SUCCESS swings
            // here, and PASS goes on to useItem and the other hand.
            if let Some(answer) = canceled {
                return self.finish_remote_item_on(hand, answer);
            }
            // Only a predicted placement (predict_place) marks a cell whose
            // resend plays a place sound: an empty hand or a non-block item
            // places nothing (BlockItem.place is where the sound comes from).
            if let Some(r) = self.remote.as_mut() {
                r.use_target = Some(hit.pos);
            }
            if let Some(answer) = item_answer {
                return self.finish_remote_item_on(hand, answer);
            }
            if shelf_reacts {
                return true;
            }
            if reacts {
                self.predict_use(&changes, seq);
            }
            // LeverBlock.useWithoutItem on the client: the predicted pull puffs.
            if (reacts || debug_stick)
                && (vanilla_block || block_answer == Some(Answer::TryWithEmptyHand))
            {
                if &*self.block_name(hit.pos) == "minecraft:lever" {
                    self.lever_puff(hit.pos);
                }
                self.open_command_block_if_editable((hit.pos[0], hit.pos[1], hit.pos[2]));
                self.open_block_editor_if_editable((hit.pos[0], hit.pos[1], hit.pos[2]));
            }
            if reacts {
                // MultiPlayerGameMode.useItemOn: SUCCESS SwingSource.CLIENT
                // (a chest, barrel, furnace, …). CONSUME does not swing.
                if result.should_swing() && block_answer != Some(Answer::SuccessServer) {
                    self.with_hand(hand, Game::swing_arm);
                }
                return true;
            }
            return item_used
                && self.remote_vanilla_item_on(&hit, hand, held.as_ref(), seq, stand_fits);
        }
        false
    }

    fn remote_armor_stand_fits(&self, hit: &Hit, held: Option<&ItemStack>) -> bool {
        held.filter(|stack| &*stack.item == "minecraft:armor_stand")
            .is_some_and(|stack| self.armor_stand_spawn(hit, stack).is_some())
    }

    /// `MultiPlayerGameMode.performUseItemOn`'s local `ItemStack.useOn`, after
    /// the block and the item's hook passed. Publishing the block packet is
    /// the caller's job: a deferred item turn must not send it again.
    fn remote_vanilla_item_on(
        &mut self,
        hit: &Hit,
        hand: Hand,
        held: Option<&ItemStack>,
        seq: i32,
        stand_fits: bool,
    ) -> bool {
        let Some(stack) = held else {
            return false;
        };
        match &*stack.item {
            "minecraft:debug_stick" => {
                // The client answers SUCCESS and swings; only the server changes the block.
                self.with_hand(hand, Game::swing_arm);
                return true;
            }
            "minecraft:end_crystal" => {
                // EndCrystalItem.useOn: SUCCESS where the crystal fits (the
                // server spawns it), FAIL elsewhere. Either ends the click.
                if self.end_crystal_fits(hit.pos) {
                    self.with_hand(hand, Game::swing_arm);
                }
                return true;
            }
            "minecraft:brush" => {
                // BrushItem.useOn starts brushing (CONSUME, no swing); the
                // server brushes and the dust is this client's own.
                self.with_hand(hand, Game::start_using_item);
                return true;
            }
            "minecraft:bone_meal"
                if self.with_hand(hand, |g| g.remote_bone_meal_succeeds(hit, stack)) =>
            {
                // BoneMealItem.useOn succeeds here; the server grows the plant.
                self.with_hand(hand, Game::swing_arm);
                return true;
            }
            "minecraft:armor_stand" if stand_fits => {
                // ArmorStandItem.useOn succeeds once the stand fits.
                self.with_hand(hand, Game::swing_arm);
                return true;
            }
            item if item.ends_with("_spawn_egg") => {
                // SpawnEggItem.useOn is FAIL when the mob cannot spawn, else
                // SUCCESS. No Item.use follows to spawn a second mob.
                if sunrise_client::block_entity_item::item_entity_data(stack, self.res.items())
                    .is_some_and(|data| self.spawn_egg_allowed(&data.kind))
                {
                    self.with_hand(hand, Game::swing_arm);
                }
                return true;
            }
            "minecraft:flint_and_steel" | "minecraft:fire_charge" => {
                if self.predict_fire_starter(stack, hit, hand, seq) {
                    self.with_hand(hand, Game::swing_arm);
                }
                return true;
            }
            "minecraft:potion"
                if stack.potion() == Some("minecraft:water")
                    && hit.face != [0, -1, 0]
                    && self.named_block_in_tag(
                        "minecraft:convertible_to_mud",
                        &self.block_name(hit.pos),
                    )
                    && (self.may_build()
                        || self.with_hand(hand, |g| g.can_place_on_in_adventure(hit.pos))) =>
            {
                // PotionItem.useOn pours out the bottle and turns the block to
                // mud locally; sounds and splash particles come from the server.
                if let Some(mud) = self.pack.assets.registry.default_state("minecraft:mud") {
                    let pos =
                        sunrise_core::position::BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
                    self.predict_blocks(&[(pos, mud)], seq);
                }
                self.with_hand(hand, |g| {
                    g.filled_result(ItemStack::new("minecraft:glass_bottle", 1))
                });
                self.with_hand(hand, Game::swing_arm);
                return true;
            }
            _ => {}
        }
        if self.with_hand(hand, |g| g.predict_item_on_block(stack, hit))
            || self.predict_map_on_banner(stack, hit)
        {
            self.with_hand(hand, Game::swing_arm);
            return true;
        }
        if let Some(info) = self
            .items()
            .get(&stack.item)
            .filter(|info| info.block.is_some())
        {
            let consumable = info.consumable.is_some().then(|| info.clone());
            let placed = self.remote_can_place(hit, stack);
            // A consumable that could not be placed is used (Item.use), which
            // the server does for this same packet. A full player's FAIL still
            // ends the click. Adventure leaves the placing to the server.
            if placed.is_none()
                && self.may_build()
                && let Some(info) = &consumable
            {
                self.with_hand(hand, |g| g.start_eating(info));
            }
            if let Some(placed) = placed {
                self.predict_place(&placed, stack, seq);
                self.with_hand(hand, |g| g.consume_placed(stack));
                self.with_hand(hand, Game::swing_arm);
            }
            // BlockItem.place's FAIL also ends the click.
            return true;
        }
        false
    }

    /// `Item.useOn` on this client's level for the items whose effects are
    /// the clicker's own (`Level.levelEvent(player, ...)` and
    /// `playSound(player, ...)`: the server sends them to the others only):
    /// honeycomb waxing copper, and an axe, hoe or shovel transforming a
    /// block (stripping, scraping, removing wax, tilling, making a path).
    /// A cushion succeeds where it may be placed and spawns on the server, a
    /// firework rocket wherever it may be used, and the server launches it.
    /// Whether the item's use succeeded, which ends the click.
    /// The caller has selected Item.useOn after cooldown and adventure gates.
    pub(in crate::game) fn predict_item_on_block(
        &mut self,
        stack: &ItemStack,
        hit: &crate::game::Hit,
    ) -> bool {
        // FireworkRocketItem.useOn: SUCCESS unless gliding, when Item.use
        // boosts instead. The phase's cooldown/adventure gates already ran.
        if &*stack.item == "minecraft:firework_rocket" {
            return !self.player.as_ref().is_some_and(|p| p.fall_flying);
        }
        let at = [
            f64::from(hit.pos[0]) + 0.5,
            f64::from(hit.pos[1]) + 0.5,
            f64::from(hit.pos[2]) + 0.5,
        ];
        if &*stack.item == "minecraft:honeycomb" {
            // HoneycombItem.useOn.
            let registry = self.pack.assets.registry;
            let clicked = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            let old = registry.state(clicked);
            let waxes = old
                .and_then(|old| self.res.transformers().waxed(registry, old))
                .is_some();
            if waxes {
                self.level_event_at(3003, hit.pos, 0);
                self.play_client_local_sound(
                    LevelSound::new("minecraft:item.honeycomb.wax_on", at, 1.0, 1.0)
                        .in_source(mod_api::SoundSource::Blocks),
                );
                // The other half of a double chest: its particles, and a second
                // sound from the clicked block.
                if let Some(other) =
                    old.and_then(|old| crate::game::storage::chest_connected_pos(old, hit.pos))
                {
                    self.level_event_at(3003, other, 0);
                    self.play_client_local_sound(
                        LevelSound::new("minecraft:item.honeycomb.wax_on", at, 1.0, 1.0)
                            .in_source(mod_api::SoundSource::Blocks),
                    );
                }
            }
            waxes
        } else if let Some((_, sound, _, _, particle, _)) = self.block_transform(stack, hit) {
            self.transform_particle(&particle, hit.pos);
            self.play_client_local_sound(
                LevelSound::new(&sound, at, 1.0, 1.0).in_source(mod_api::SoundSource::Blocks),
            );
            true
        } else if self
            .res
            .items()
            .get(&stack.item)
            .is_some_and(|i| i.cushion_color.is_some())
        {
            // CushionItem.useOn on a client level: SUCCESS once
            // `canBePlacedAt` holds, before the server's own checks.
            self.cushion_spawn(hit).is_some()
        } else if &*stack.item == "minecraft:painting" {
            // HangingEntityItem.useOn on a client level: SUCCESS once
            // `Painting.create` finds a surviving variant, before the
            // server spawns it.
            self.painting_fits(hit, stack)
        } else {
            false
        }
    }

    fn remote_item_use_aim(&self) -> ItemUseAim {
        ItemUseAim {
            rotation: (self.yaw, self.pitch),
            eye: self.eye_position(),
            ray: self.aim(),
            reach: self.reach(),
        }
    }

    pub(super) fn receive_cauldron_use(&mut self, data: &[u8]) -> Result<()> {
        let mut body = sunrise_protocol::buf::Reader::new(data);
        let receipt = body.i64()?;
        let block_fell_through = body.bool()?;
        let passed = body.bool()?;
        if body.remaining() != 0 {
            return Err("extra bytes in authoritative block-use result".into());
        }
        let click = self
            .remote
            .as_mut()
            .and_then(|r| r.pending_cauldrons.get_mut().remove(&receipt));
        // Minecraft.startUseItem and Item.getPlayerPOVHitResult use the same
        // player and aim in one input turn. A UseItem packet can't supply an old
        // eye position, so changed input supersedes the suspended click.
        if (block_fell_through || passed)
            && let Some(click) = click
            && self.selected == click.selected
            && self.stack_in(click.selected) == click.main.as_ref()
            && self.offhand == click.off
            && self.remote_item_use_aim() == click.aim
        {
            self.with_interaction_swing_scope(|game| {
                if let Some(remote) = &mut game.remote {
                    remote.interaction_swing = click.swing;
                }
                if block_fell_through && click.item_after_block {
                    let held = match click.hand {
                        Hand::Main => click.main.as_ref(),
                        Hand::Off => click.off.as_ref(),
                    };
                    let swings = game.swings_started;
                    let answer = game.remote_item_after_block(&click.hit, click.hand, held);
                    if let Some(remote) = &mut game.remote {
                        remote.use_target = Some(click.hit.pos);
                    }
                    let done = if let Some(answer) = answer {
                        game.finish_remote_item_on(click.hand, answer)
                    } else {
                        let stand_fits = game.remote_armor_stand_fits(&click.hit, held);
                        game.remote_vanilla_item_on(
                            &click.hit,
                            click.hand,
                            held,
                            click.sequence,
                            stand_fits,
                        )
                    };
                    game.finish_remote_use_block(click.hand, held.map(|s| s.count), swings, done);
                    if done {
                        return;
                    }
                }
                if passed && game.remote_item_use_aim() == click.aim {
                    game.remote_use_hands(click.hand, click.attempt, Some(click.hit));
                }
            });
        }
        Ok(())
    }

    fn remote_item_after_block(
        &mut self,
        hit: &Hit,
        hand: Hand,
        held: Option<&ItemStack>,
    ) -> Option<mod_api::InteractionResult> {
        self.use_item_on_block(hit, hand, held, mod_api::UsePhase::ItemAfterBlock)
            .or_else(|| held.and_then(|stack| self.item_on_interaction(hit, hand, stack)))
    }

    fn finish_remote_item_on(&mut self, hand: Hand, answer: mod_api::InteractionResult) -> bool {
        use mod_api::InteractionResult as Answer;
        if answer == Answer::Success {
            self.with_hand(hand, Game::swing_arm);
        }
        answer.consumes_action() || answer == Answer::Fail
    }
}
