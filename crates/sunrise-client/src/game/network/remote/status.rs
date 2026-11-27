use super::*;
use sunrise_client::skins;

impl Game<'_> {
    /// `PlayerInfo.getSkin` for every player entity, including NPCs that are
    /// never in the tab list: resolve each profile's textures and give the
    /// level's player models their skin.
    pub(in crate::game) fn update_remote_skins(&mut self) {
        self.poll_own_profile();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let local = r.profile;
        let level: Vec<_> = r
            .level_players
            .iter()
            .map(|(id, uuid)| (*id as u32, *uuid))
            .collect();
        let listed = skin_entries(
            &r.level_players,
            &r.skin_textures,
            &mut r.level_player_textures,
        );
        let (online_mode, names): (bool, Vec<(u128, String)>) = (
            r.online_mode,
            r.players.iter().map(|(u, n)| (*u, n.clone())).collect(),
        );
        // Unpacked and verified on the skin downloader's threads: the game
        // thread reads answers and starts the rest (`SkinManager.getOrLoad`).
        let entries: Vec<(u128, Arc<skins::Textures>)> = {
            let keys = r.services_keys.lock().unwrap_or_else(|e| e.into_inner());
            let mut downloads = self.skin_downloads.borrow_mut();
            listed
                .iter()
                .filter_map(|(uuid, property)| {
                    property
                        .unpack(&mut downloads, &keys, *uuid != local)
                        .map(|textures| (*uuid, textures))
                })
                .collect()
        };
        // Minecraft.profileFuture: the signed-in account's active skin and cape
        // when the server sent none (single player), or only a skin without
        // the equipped cape: the account's skin, with its model, where the server
        // sent none, and its cape where the server sent none.
        let own = match &r.own_profile {
            OwnProfile::Done(Some(own)) => match entries.iter().find(|(u, _)| *u == local) {
                // Each profile's textures stay behind their `Arc`: only the
                // signed-in account's merge is copied.
                Some((_, textures)) => {
                    let mut merged = textures.as_ref().clone();
                    if merged.skin.is_none() {
                        merged.skin.clone_from(&own.skin);
                        if own.skin.is_some() {
                            merged.slim = own.slim;
                        }
                    }
                    if merged.cape.is_none() {
                        merged.cape.clone_from(&own.cape);
                    }
                    Some(merged)
                }
                None => (own.skin.is_some() || own.cape.is_some()).then(|| own.clone()),
            },
            _ => None,
        };
        // Starling, Skin Customization's Ely.by Skins: on a server that sends no skin for a player
        // (offline mode), the one their name has on Ely.by, as offline launchers show it; and an
        // Ely.by account's own skin where the server sent none. Ely.by's are never signed.
        let mut elyby: Vec<(u128, skins::Textures)> = Vec::new();
        let has_skin = |uuid: u128| entries.iter().any(|(u, t)| *u == uuid && t.skin.is_some());
        if !has_skin(local)
            && own.as_ref().is_none_or(|t| t.skin.is_none())
            && let Some(textures) = self.own_elyby_textures()
        {
            elyby.push((local, textures));
        }
        if !online_mode {
            for (uuid, name) in &names {
                if has_skin(*uuid) || elyby.iter().any(|(u, _)| u == uuid) {
                    continue;
                }
                if let Some(textures) = self.elyby_textures(name) {
                    elyby.push((*uuid, textures));
                }
            }
        }
        let mut skins = HashMap::new();
        {
            // PlayerInfo.createSkinLookup: only the local player's own skin
            // may be unsigned; Ely.by's never are.
            let mut downloads = self.skin_downloads.borrow_mut();
            let mut resolve = |uuid, textures: &skins::Textures| {
                if let skins::SkinLookup::Ready(skin) =
                    skins::lookup(textures, uuid != local && !elyby.iter().any(|(u, _)| *u == uuid), |kind, url| {
                        downloads.state(kind, url)
                    })
                    && skin != skins::ResolvedSkin::default()
                {
                    skins.insert(uuid, skin);
                }
            };
            for (uuid, textures) in entries.iter().filter(|(u, _)| own.is_none() || *u != local) {
                resolve(*uuid, textures);
            }
            if let Some(textures) = &own {
                resolve(local, textures);
            }
            // Last, so a name's Ely.by skin takes the place of a server's entry without one.
            for (uuid, textures) in &elyby {
                resolve(*uuid, textures);
            }
        }
        // Starling, Options > Skin Customization: the player's own skin, in this game only (others
        // get what the server sends of the profile).
        if let Some((own, own_slim)) = self.own_skin_sprite() {
            resolved.retain(|(u, _)| *u != local);
            resolved.push((local, own));
            if own_slim {
                slim.insert(local);
            } else {
                slim.remove(&local);
            }
        }
        // An offline name's skin from the launcher, where neither the server
        // nor an account gives the local player textures.
        if !skins.contains_key(&local)
            && let Some((rgba, is_slim)) = skins::offline_skin()
        {
            const OFFLINE: &str = "sunrise/offline_skin";
            if !self.skin_slots.contains(OFFLINE) || !self.gui.atlas.sprites.contains_key(OFFLINE) {
                self.store_skin(OFFLINE.into(), rgba.clone());
            }
            skins.entry(local).or_default().custom_body = Some((OFFLINE.into(), *is_slim));
        }
        // An offline name's cape from the launcher, where the profile has none.
        if skins.get(&local).is_none_or(|s| s.cape.is_none())
            && let Some(rgba) = skins::offline_cape()
        {
            const OFFLINE: &str = "capes/offline";
            if !self.skin_slots.contains(OFFLINE) || !self.gui.atlas.sprites.contains_key(OFFLINE) {
                // Kept for `again`, which finds no download or disk file for this cape.
                self.skin_downloads
                    .borrow_mut()
                    .remember_image(OFFLINE, rgba);
                self.store_skin(OFFLINE.into(), rgba.to_vec());
            }
            skins.entry(local).or_default().cape = Some(OFFLINE.into());
        }
        // Only the skins drawn hold atlas slots: the level's players, not
        // every entry of a busy server's player list.
        // Build once: resolving each player must not scan every level entity.
        let players: HashMap<_, _> = self
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| &*m.kind.id == "minecraft:player")
            .map(|(i, m)| (m.id, i))
            .collect();
        for (id, uuid) in level {
            let Some(&i) = players.get(&id) else {
                continue;
            };
            let (skin, is_slim, cape) =
                skins::drawable_skin(uuid, skins.get(&uuid), true, |sprite| {
                    self.again_skin(sprite)
                });
            let mob = &mut self.mobs[i];
            mob.skin = Some(skin.to_owned());
            mob.slim = is_slim;
            mob.cape = cape.map(str::to_owned);
        }
        if self.player_model.is_some() {
            // Avatar.isModelPartShown(CAPE): the Skin Customization option.
            let show_cape = self.settings.file.bool("modelPart_cape");
            let (skin, is_slim, cape) =
                skins::drawable_skin(local, skins.get(&local), show_cape, |sprite| {
                    self.again_skin(sprite)
                });
            // Starling: the wardrobe's cape (Android), in place of the account's.
            #[cfg(target_os = "android")]
            let cape = if self.settings.file.bool("modelPart_cape") {
                self.wardrobe_cape(cape)
            } else {
                None
            };
            if let Some(mob) = self.player_model.as_mut() {
                mob.skin = Some(skin.to_owned());
                mob.slim = is_slim;
                mob.cape = cape.map(str::to_owned);
            }
        }
        if let Some(r) = self.remote.as_mut()
            && r.skins != skins
        {
            r.skins_revision = r.skins_revision.wrapping_add(1);
            r.skins = skins;
        }
        // ClientMannequin.updateSkin (PlayerSkinRenderCache.lookup): a
        // mannequin's DATA_PROFILE `textures` property resolves like a
        // player's, never required to be signed.
        let mannequins: Vec<(usize, sunrise_client::skins::Textures)> = self
            .mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| &*m.kind.id == "minecraft:mannequin" && m.profile.is_some())
            .map(|(i, m)| {
                let profile = m.profile.as_ref();
                let t = profile.and_then(|p| p.properties.iter().find(|p| p.name == "textures"));
                let textures = if let Some(t) = t {
                    skins::unpack_insecure(&t.value)
                } else {
                    // `ResolvableProfile.resolveProfile`: no `textures` and
                    // only a name or an id resolves through the services.
                    let key = profile.and_then(|p| match (&p.name, p.uuid) {
                        (Some(name), None) => {
                            Some(sunrise_client::skins::NameOrId::Name(name.clone()))
                        }
                        (None, Some(id)) => Some(sunrise_client::skins::NameOrId::Id(id)),
                        _ => None,
                    });
                    key.and_then(|key| match self.skin_profiles.borrow_mut().state(&key) {
                        sunrise_client::skins::Resolve::Done(Some(profile)) => {
                            profile.textures().map(skins::unpack_insecure)
                        }
                        _ => None,
                    })
                    .unwrap_or_default()
                };
                (i, textures)
            })
            .collect();
        let mut resolved: Vec<(usize, Option<String>, bool)> = Vec::new();
        for (id, textures) in mannequins {
            let mut downloads = self.skin_downloads.borrow_mut();
            let (skin, slim) =
                match skins::lookup(&textures, false, |kind, url| downloads.state(kind, url)) {
                    skins::SkinLookup::Ready(skin) => skin.custom_body.unzip(),
                    _ => (None, None),
                };
            resolved.push((id, skin, slim.unwrap_or(false)));
        }
        for (i, skin, slim) in resolved {
            let skin = skin.filter(|s| !s.starts_with("skins/") || self.again_skin(s));
            // Resolution and atlas uploads do not insert or remove mobs.
            let mob = &mut self.mobs[i];
            mob.slim = slim && skin.is_some();
            mob.skin = skin;
        }
    }
    pub(in crate::game) fn remote_status(&self) -> Option<serde_json::Value> {
        let r = self.remote.as_ref()?;
        Some(serde_json::json!({
            "address": sunrise_client::net::shown_address(&r.session.address),
            "name": r.session.name,
            "joined": r.joined,
            // ServerboundPlayerLoadedPacket sent: the level around the
            // player is ready, and the client ticks and sends its moves.
            "loaded": r.loaded_sent,
            "status": r.status,
            "entity_id": r.entity_id,
            "dimension": r.dimension.name,
            "game_mode": r.game_mode,
            "chunks": self.world.chunks.len(),
            "entities": r.entities.values().fold(std::collections::BTreeMap::<&str, u32>::new(), |mut m, t| {
                *m.entry(t.kind.as_str()).or_default() += 1;
                m
            }),
            "players": r.players.values().collect::<Vec<_>>(),
            // Where the other players' entities are (harnesses check movement).
            // What the other players hold and wear, as this client draws them.
            "player_equipment": self
                .mobs
                .iter()
                .filter(|m| r.level_players.contains_key(&(m.id as i32)))
                .map(|m| {
                    let e = &m.extra;
                    let armor = sunrise_items::equipment::ARMOR.map(|slot| e.slot_id(slot));
                    serde_json::json!({
                        "id": m.id,
                        "swinging": m.ai.swinging,
                        "using": m.extra.using_item,
                        "armor": armor,
                        "main_hand": e.main_hand_id(),
                        "off_hand": e.off_hand_id(),
                    })
                })
                .collect::<Vec<_>>(),
            "player_positions": r
                .entities
                .values()
                .filter(|t| t.kind == "minecraft:player")
                .map(|t| t.position)
                .collect::<Vec<_>>(),
            // The other players this client draws dead (lying down, after EntityEvent DEATH).
            "dead_players": self
                .mobs
                .iter()
                .filter(|m| r.level_players.contains_key(&(m.id as i32)) && !m.alive())
                .map(|m| m.id)
                .collect::<Vec<_>>(),
            // The other players this client draws lying in a bed (SLEEPING_POS).
            "lying_players": self
                .mobs
                .iter()
                .filter(|m| r.level_players.contains_key(&(m.id as i32)) && m.extra.ints[1] != 0)
                .map(|m| m.id)
                .collect::<Vec<_>>(),
            // The player's own model exists (third person and the inventory draw it).
            "player_model": self.player_model.is_some(),
            // The world view draws its body now (third person, or lying in a bed;
            // not while invisible).
            "own_body": self.player_model_shown() && self.own_body_visible(),
            "text_display_vertices": self.text_display_vertices,
            "text_displays": self.text_displays.len(),
            "health": r.health.0,
            "food": r.health.1,
            "decode_errors": r.decode_errors,
            "chat": r.chat.iter().rev().take(10).rev().map(|(t, _)| t.clone()).collect::<Vec<_>>(),
        }))
    }
}
