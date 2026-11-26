use super::*;

use sunrise_protocol::packets::configuration::{ClientInformation, Transfer};

/// Connection already started; its eventual packets still belong to the next listener.
pub(super) struct RemoteOpening {
    session: Session,
    builtin: Arc<Builtin>,
    lang: Arc<Language>,
    info: ClientInformation,
    user: (String, u128),
    local: bool,
    list_name: Option<String>,
}

impl Game<'_> {
    pub(in crate::game) fn connect_remote_session(
        &mut self,
        address: &str,
        name: &str,
        local: Option<sunrise_protocol::connection::Pipe>,
        cookies: Option<sunrise_client::net::Cookies>,
        lan: bool,
        realm: bool,
        list_name: Option<&str>,
    ) -> Result<()> {
        let opening =
            self.open_remote_session(address, name, local, cookies, lan, true, list_name)?;
        self.install_remote_session(opening, lan, realm)
    }

    fn open_remote_session(
        &mut self,
        address: &str,
        name: &str,
        local: Option<sunrise_protocol::connection::Pipe>,
        cookies: Option<sunrise_client::net::Cookies>,
        lan: bool,
        announce: bool,
        list_name: Option<&str>,
    ) -> Result<RemoteOpening> {
        let root = &self.options.resources;
        let item_data = Arc::clone(&self.res.core.item_data);
        let builtin = builtin_registries(root).ok_or("Resources are incomplete: registries.json is missing or invalid. Click \"Prepare again\" in the launcher (or run sunrise-prepare), then try again.")?;
        let lang = Arc::new(sunrise_client::net::load_language(root));
        let info = self
            .settings
            .client_information(client_view_distance(self.settings.render_distance));
        // A signed-in account always plays under its own name on servers (an online-mode one
        // takes no other). Starling: single player's own server, the name its world opened with
        // (a name chosen in Skin Customization goes first).
        let auth = self.auth();
        let name = match (&auth, &local) {
            (Some(a), None) => a.name.as_str(),
            _ => name,
        };
        let user = (
            name.to_owned(),
            auth.as_ref()
                .map_or_else(|| sunrise_protocol::crypto::offline_uuid(name), |a| a.uuid),
        );
        let is_local = local.is_some();
        if announce && !is_local {
            self.connection_starts(address, name, lan);
        }
        let session = match (local, cookies) {
            (Some(pipe), _) => Session::connect_local(
                pipe,
                name,
                auth.clone(),
                info.clone(),
                lang.clone(),
                item_data,
                Arc::clone(&self.client_reports),
            ),
            (None, Some(cookies)) => Session::connect_transferred(
                address,
                name,
                auth.clone(),
                info.clone(),
                lang.clone(),
                cookies,
                item_data,
                Arc::clone(&self.client_reports),
            ),
            (None, None) => {
                let connect = if lan {
                    Session::connect_lan
                } else {
                    Session::connect
                };
                connect(
                    address,
                    name,
                    auth.clone(),
                    info.clone(),
                    lang.clone(),
                    item_data,
                    Arc::clone(&self.client_reports),
                )
            }
        };
        Ok(RemoteOpening {
            session,
            builtin,
            lang,
            info,
            user,
            local: is_local,
            list_name: list_name.map(str::to_owned),
        })
    }

    /// `handleTransfer`: close the old connection and start the new one now.
    /// Keep the old listener installed for the remaining handlers of this bundle.
    pub(super) fn start_remote_transfer(
        &mut self,
        transfer: Transfer,
    ) -> Result<Option<RemoteOpening>> {
        let Some(remote) = self.remote.as_ref() else {
            return Ok(None);
        };
        remote.session.close_after_pending();
        if remote.session.address == "local" {
            return Ok(None);
        }
        let address = format!("{}:{}", transfer.host, transfer.port);
        let name = remote.session.name.clone();
        let cookies = remote.session.cookies.clone();
        self.open_remote_session(&address, &name, None, Some(cookies), false, false, None)
            .map(Some)
    }

    pub(super) fn install_remote_transfer(&mut self, opening: RemoteOpening) -> Result<()> {
        self.connection_left(sunrise_hooks::connection::Left::Transferred);
        self.leave_server(None)?;
        self.connection_starts(&opening.session.address, &opening.session.name, false);
        self.install_remote_session(opening, false, false)
    }

    fn install_remote_session(
        &mut self,
        opening: RemoteOpening,
        lan: bool,
        realm: bool,
    ) -> Result<()> {
        let RemoteOpening {
            session,
            builtin,
            lang,
            info,
            user,
            local,
            list_name,
        } = opening;
        self.reset_client_item_use();
        self.realm_session = realm;
        self.lan_session = lan;
        self.reset_tutorial();
        self.perf.world_opening();
        if !local {
            // A server's own biomes come over the connection, not from the last local world.
            self.world_biomes = None;
            self.world_biomes_base = None;
            mod_api::content::crash::set_detail(
                "Server",
                sunrise_client::net::shown_address(&session.address),
            );
        }
        self.server_packs_connect(
            (!local).then_some(&session.address),
            list_name.as_deref(),
            user,
        );
        let root = self.options.resources.clone();
        let player_type = super::super::perspective::player_type(&root);
        let mut streaming = Streaming::new(
            Arc::new(NoTerrain),
            self.worker_context(),
            self.res.threads(),
        );
        streaming.remote = true;
        streaming.radius = self.settings.render_distance;
        self.streaming = streaming;
        self.reset_client_world(0);
        let group_count = sunrise_client::chunk_map::ConnectionGroupCount::default();
        self.world
            .chunks
            .track_connection_groups(group_count.clone());
        self.save = Arc::new(SaveDir::new(
            std::env::temp_dir().join("sunrise-remote-unused"),
            sunrise_client::save::LevelShape::OVERWORLD,
            None,
        ));
        self.inventory = vec![None; 36];
        self.offhand = None;
        self.armor = [None, None, None, None];
        self.equipment = Default::default();
        self.craft_grid = vec![None; 9];
        self.craft_size = 2;
        self.remove_local_player_body();
        self.spawn = None;
        self.level.world_spawn_dimension = None;
        self.mobs.clear();
        self.clear_remote_objects();
        self.clear_remote_projectiles();
        self.creatures.projectiles.clear();
        self.dropped.clear();
        self.clear_client_level_particles();
        self.falling.clear();
        self.mining = None;
        self.container = None;
        self.carried = None;
        self.using = None;
        self.clicks.miss_time = 0;
        // A new LocalPlayer has no effects: the server sends what it has
        // (ClientboundUpdateMobEffectPacket on placement).
        self.potions = Default::default();
        self.experience = Experience {
            level: 0,
            progress: 0.0,
            total: 0,
        };
        // The connected client never opens a server world (single player also
        // joins here), so load the registries its argument parsers and local
        // suggestions need before receiving the server's command tree.
        self.ui.registries = Default::default();
        self.build_registries();
        self.ui.registries.permission = 0;
        self.ui.chat.clear();
        self.set_screen(Screen::Playing);
        self.hide_title();
        self.dim = Dim::Overworld;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        self.apply_settings();
        self.clear_waypoints();
        self.boss_overlay.reset();
        // The chat's tree until the server sends its own, or forever when it
        // sends none: the game's own, plus the commands native mods own.
        self.native_client_commands.pending_commands.clear();
        self.native_client_commands.pending_chats.clear();
        let chat_tree = self.chat_tree_for(None);
        let services_keys = self.fresh_services_keys();
        self.ui.registries.selector_tokens.clear();
        self.remote = Some(Remote {
            selector_metadata_supported: true,
            professions: self.client_professions()?,
            display_address: Remote::display_address_for(&session.address),
            session,
            brand: "unknown".into(),
            datapack_vanilla_values: None,
            particle_packet_random: sunrise_client::random_source::create(),
            recipe_viewer: Default::default(),
            features: ["minecraft:vanilla".to_string()].into(),
            quiet_commands: false,
            custom_books: false,
            logged_out: false,
            attachments: HashMap::new(),
            colliders: HashMap::new(),
            collider_pass_start: None,
            carry_age: Default::default(),
            far_terrain: None,
            builtin,
            status: lang
                .get("connect.connecting")
                .cloned()
                .unwrap_or_else(|| "Connecting to the server...".into()),
            lang,
            registries: Arc::new(Dynamic::default()),
            variants: Arc::new(sunrise_client::mobs::variants::VariantTables::default()),
            tags: None,
            enchantments: None,
            joined: false,
            reconfiguring: None,
            logged_in: false,
            has_logical_player: false,
            entity_id: 0,
            local_entity_silent: false,
            camera: None,
            detached_camera: None,
            dimension: DimensionType::vanilla("minecraft:overworld"),
            level: "minecraft:overworld".into(),
            level_key: mod_api::DimensionKey::OVERWORLD,
            level_facts: None,
            level_environment: None,
            level_pack_environment: None,
            synced_type: None,
            level_environments: Default::default(),
            facts_cache: Default::default(),
            added_levels: Default::default(),
            viewed: Default::default(),
            hashed_seed: 0,
            sea_level: 63,
            chunk_format: Arc::new(ChunkFormat::new(
                &DimensionType::vanilla("minecraft:overworld"),
                Vec::new(),
                7,
            )),
            decoder: ChunkDecoder::new(),
            view_distance: self.settings.render_distance,
            simulation_distance: self.settings.simulation_distance,
            sequence: 0,
            last_teleport_sequence: -1,
            position_sender: Default::default(),
            last_input: 0,
            was_sprinting: false,
            carried: std::cell::Cell::new(None),
            was_shared_flags: 0,
            was_living_flags: 0,
            last_recorded_pose: None,
            abilities: sunrise_client::lan::abilities::PlayerAbilities::default(),
            respawn_velocity: None,
            loaded_sent: false,
            heard_level: None,
            datapacks: Default::default(),
            constructing_level: None,
            level_load: sunrise_client::level_loading::LevelLoadTracker::new(0),
            sent_info: info,
            group_count,
            predictions: HashMap::new(),
            menu: ClientMenu::default(),
            ticked_before_player: std::collections::HashSet::new(),
            craft_result: None,
            menu_title: None,
            menu_title_snapshot: None,
            state_id: 0,
            inventory_state_id: 0,
            inventory_carried: None,
            inventory_craft_grid: Default::default(),
            inventory_craft_result: None,
            use_target: None,
            cauldron_entries: None,
            cauldron_receipt: 0,
            pending_cauldrons: Default::default(),
            menu_extra: None,
            entities: HashMap::new(),
            tick_fluids: Vec::new(),
            tick_rain: Vec::new(),
            tick_standings: Vec::new(),
            tick_water_fx: Vec::new(),
            passenger_vehicles: HashMap::new(),
            lerps: HashMap::new(),
            general: Default::default(),
            cart_lerps: HashMap::new(),
            stepped: HashMap::new(),
            head_lerps: HashMap::new(),
            ghast_dismount: None,
            first_passenger: false,
            own_tick_count: 0,
            passenger_index: 0,
            pending_items: HashMap::new(),
            health: (20.0, 20, 5.0),
            flash_on_set_health: false,
            game_mode: 0,
            chat: Vec::new(),
            chat_received: 0,
            names: HashMap::new(),
            starling: Default::default(),
            chat_queue: VecDeque::new(),
            signatures: Default::default(),
            chat_previous: None,
            next_chat_index: 0,
            enforces_secure_chat: false,
            services_keys,
            chat_sessions: HashMap::new(),
            chat_validators: HashMap::new(),
            joining_narrated: None,
            chat_tree,
            server_chat_tree: None,
            secure_chat: Default::default(),
            custom_completions: Default::default(),
            players: HashMap::new(),
            tab_players: HashMap::new(),
            tab_game_modes: HashMap::new(),
            own_player_info: std::cell::OnceCell::new(),
            skin_textures: HashMap::new(),
            own_profile: OwnProfile::Idle,
            skins: HashMap::new(),
            skins_revision: 0,
            published_skins: None,
            listed: Default::default(),
            online_mode: false,
            levels: Vec::new(),
            level_players: HashMap::new(),
            level_uuids: HashMap::new(),
            level_entity_uuids: HashMap::new(),
            level_player_infos: HashMap::new(),
            level_player_names: HashMap::new(),
            level_player_textures: HashMap::new(),
            glowing: Default::default(),
            own_invisible: false,
            own_on_fire: false,
            own_arrows: 0,
            own_stingers: 0,
            own_shoulders: [None; 2],
            profile: 0,
            profile_name: String::new(),
            destroying: None,
            destroying_item: None,
            is_destroying: false,
            interaction_swing: [true; 2],
            destroy_delay: 0,
            swap_answer: [false; 2],
            stop_mining: false,
            flat: false,
            death_message: None,
            show_death_screen: true,
            reduced_debug_info: false,
            do_limited_crafting: false,
            player_type,
            synced: vec![None; CREATIVE_SYNCED],
            decode_errors: 0,
            clocks: HashMap::new(),
            default_clock: None,
            overworld_clock: None,
            clock_game_time: 0,
            chunk_center: (0, 0),
            awaiting_viewed_chunk_center: false,
            chunk_light: RemoteLight::default(),
            backlog: VecDeque::new(),
            backlog_since: None,
            server_links: Vec::new(),
            debug_query: Default::default(),
            synced_attributes: HashMap::new(),
        });
        self.set_local_mode_with_previous(sunrise_client::command::args::GameMode::Survival, None);
        // The level's TickRateManager follows the server's ticking packets.
        self.clock = sunrise_client::clock::FixedStep::client();
        // The player's advancements and statistics are the server's.
        self.adv = Default::default();
        self.stats = Default::default();
        Ok(())
    }
    /// A joined client's trust keys, fetched off the game thread. Minecraft makes its
    /// services once per run; a new set ends the skin verdicts made with the old one.
    fn fresh_services_keys(&mut self) -> Arc<std::sync::Mutex<ServicesKeys>> {
        self.skin_downloads.borrow_mut().forget_unpacked();
        ServicesKeys::fetching()
    }
    /// Prepare a client session and remote state to receive packets from a studio replay.
    /// A replayed or viewed world checks its players' signed properties as a joined
    /// client does.
    pub(in crate::game) fn init_remote_for_replay(&mut self) -> Result<()> {
        // Minecraft builds its `Services` once per client: a replay that opens a new
        // session on every seek keeps the keys the session it replaces is fetching or
        // has fetched.
        let services_keys = match &self.remote {
            Some(remote) => remote.services_keys.clone(),
            None => self.fresh_services_keys(),
        };
        self.reset_client_item_use();
        self.realm_session = false;
        self.lan_session = false;
        self.reset_tutorial();
        self.perf.world_opening();
        self.world_biomes = None;
        self.world_biomes_base = None;
        let root = self.options.resources.clone();
        let builtin = builtin_registries(&root).ok_or("Resources are incomplete")?;
        let lang = Arc::new(sunrise_client::net::load_language(&root));
        let info = self
            .settings
            .client_information(client_view_distance(self.settings.render_distance));
        let auth = self.auth();
        let name = auth.as_ref().map_or("ReplayPlayer", |a| a.name.as_str());
        let user = auth.as_ref().map_or_else(
            || {
                (
                    name.to_owned(),
                    sunrise_protocol::crypto::offline_uuid(name),
                )
            },
            |a| (a.name.clone(), a.uuid),
        );
        self.server_packs_connect(None, None, user);
        let session = Session::replay(name);
        let player_type = super::super::perspective::player_type(&root);
        let mut streaming = Streaming::new(
            Arc::new(NoTerrain),
            self.worker_context(),
            self.res.threads(),
        );
        streaming.remote = true;
        streaming.radius = self.settings.render_distance;
        self.streaming = streaming;
        self.reset_client_world(0);
        let group_count = sunrise_client::chunk_map::ConnectionGroupCount::default();
        self.world
            .chunks
            .track_connection_groups(group_count.clone());
        self.save = Arc::new(SaveDir::new(
            std::env::temp_dir().join("sunrise-replay-unused"),
            sunrise_client::save::LevelShape::OVERWORLD,
            None,
        ));
        self.inventory = vec![None; 36];
        self.offhand = None;
        self.armor = [None, None, None, None];
        self.equipment = Default::default();
        self.craft_grid = vec![None; 9];
        self.craft_size = 2;
        self.remove_local_player_body();
        self.spawn = None;
        self.level.world_spawn_dimension = None;
        self.mobs.clear();
        self.clear_remote_objects();
        self.clear_remote_projectiles();
        self.creatures.projectiles.clear();
        self.dropped.clear();
        self.clear_client_level_particles();
        self.orbs.clear();
        self.falling.clear();
        self.mining = None;
        self.container = None;
        self.carried = None;
        self.using = None;

        self.experience = Experience {
            level: 0,
            progress: 0.0,
            total: 0,
        };
        self.ui.registries = Default::default();
        self.build_registries();
        self.ui.registries.permission = 0;
        self.ui.chat.clear();
        self.set_screen(Screen::Playing);
        self.title = None;
        self.dim = Dim::Overworld;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        self.apply_settings();
        self.clear_waypoints();
        self.boss_overlay.reset();

        self.native_client_commands.pending_commands.clear();
        self.native_client_commands.pending_chats.clear();
        let chat_tree = self.chat_tree_for(None);

        self.ui.registries.selector_tokens.clear();
        self.remote = Some(Remote {
            selector_metadata_supported: true,
            professions: self.client_professions()?,
            display_address: Remote::display_address_for(&session.address),
            session,
            brand: "sunrise-studio".into(),
            datapack_vanilla_values: None,
            particle_packet_random: sunrise_client::random_source::create(),
            recipe_viewer: Default::default(),
            features: ["minecraft:vanilla".to_string()].into(),
            quiet_commands: false,
            custom_books: false,
            attachments: HashMap::new(),
            colliders: HashMap::new(),
            collider_pass_start: None,
            carry_age: Default::default(),
            far_terrain: None,
            builtin,
            status: "Playing Studio Replay".into(),
            lang,
            registries: Arc::new(Dynamic::default()),
            variants: Arc::new(sunrise_client::mobs::variants::VariantTables::default()),
            tags: None,
            enchantments: None,
            joined: false,
            reconfiguring: None,
            logged_in: true,
            has_logical_player: false,
            logged_out: false,
            local_entity_silent: false,
            entity_id: 0,
            camera: None,
            detached_camera: None,
            dimension: DimensionType::vanilla("minecraft:overworld"),
            level: "minecraft:overworld".into(),
            level_key: mod_api::DimensionKey::OVERWORLD,
            level_facts: None,
            level_environment: None,
            level_pack_environment: None,
            synced_type: None,
            level_environments: Default::default(),
            facts_cache: Default::default(),
            added_levels: Default::default(),
            viewed: Default::default(),
            hashed_seed: 0,
            sea_level: 63,
            chunk_format: Arc::new(ChunkFormat::new(
                &DimensionType::vanilla("minecraft:overworld"),
                Vec::new(),
                7,
            )),
            decoder: ChunkDecoder::new(),
            view_distance: self.settings.render_distance,
            simulation_distance: self.settings.simulation_distance,
            sequence: 0,
            last_teleport_sequence: -1,
            position_sender: Default::default(),
            last_input: 0,
            was_sprinting: false,
            was_shared_flags: 0,
            was_living_flags: 0,
            last_recorded_pose: None,
            carried: std::cell::Cell::new(None),
            abilities: sunrise_client::lan::abilities::PlayerAbilities::default(),
            respawn_velocity: None,
            loaded_sent: false,
            heard_level: None,
            datapacks: Default::default(),
            constructing_level: None,
            level_load: sunrise_client::level_loading::LevelLoadTracker::new(0),
            sent_info: info,
            group_count,
            predictions: HashMap::new(),
            menu: ClientMenu::default(),
            ticked_before_player: std::collections::HashSet::new(),
            craft_result: None,
            menu_title: None,
            menu_title_snapshot: None,
            state_id: 0,
            inventory_state_id: 0,
            inventory_carried: None,
            inventory_craft_grid: Default::default(),
            inventory_craft_result: None,
            use_target: None,
            cauldron_entries: None,
            cauldron_receipt: 0,
            pending_cauldrons: Default::default(),
            menu_extra: None,
            entities: HashMap::new(),
            tick_fluids: Vec::new(),
            tick_rain: Vec::new(),
            tick_standings: Vec::new(),
            tick_water_fx: Vec::new(),
            passenger_vehicles: HashMap::new(),
            lerps: HashMap::new(),
            general: Default::default(),
            cart_lerps: HashMap::new(),
            stepped: HashMap::new(),
            head_lerps: HashMap::new(),
            ghast_dismount: None,
            first_passenger: false,
            own_tick_count: 0,
            passenger_index: 0,
            pending_items: HashMap::new(),
            health: (20.0, 20, 5.0),
            flash_on_set_health: false,
            game_mode: 0,
            chat: Vec::new(),
            chat_received: 0,
            names: HashMap::new(),
            starling: Default::default(),
            chat_queue: VecDeque::new(),
            signatures: Default::default(),
            chat_previous: None,
            next_chat_index: 0,
            enforces_secure_chat: false,
            services_keys,
            chat_sessions: HashMap::new(),
            chat_validators: HashMap::new(),
            joining_narrated: None,
            chat_tree,
            server_chat_tree: None,
            secure_chat: Default::default(),
            custom_completions: Default::default(),
            players: HashMap::new(),
            tab_players: HashMap::new(),
            tab_game_modes: HashMap::new(),
            own_player_info: std::cell::OnceCell::new(),
            skin_textures: HashMap::new(),
            own_profile: OwnProfile::Idle,
            skins: HashMap::new(),
            skins_revision: 0,
            published_skins: None,
            listed: Default::default(),
            online_mode: false,
            levels: Vec::new(),
            level_players: HashMap::new(),
            level_uuids: HashMap::new(),
            level_entity_uuids: HashMap::new(),
            level_player_infos: HashMap::new(),
            level_player_names: HashMap::new(),
            level_player_textures: HashMap::new(),
            glowing: Default::default(),
            own_invisible: false,
            own_on_fire: false,
            own_arrows: 0,
            own_stingers: 0,
            own_shoulders: [None; 2],
            profile: 0,
            profile_name: String::new(),
            destroying: None,
            destroying_item: None,
            is_destroying: false,
            interaction_swing: [true; 2],
            destroy_delay: 0,
            swap_answer: [false; 2],
            stop_mining: false,
            flat: false,
            death_message: None,
            show_death_screen: true,
            reduced_debug_info: false,
            do_limited_crafting: false,
            player_type,
            synced: vec![None; CREATIVE_SYNCED],
            decode_errors: 0,
            clocks: HashMap::new(),
            default_clock: None,
            overworld_clock: None,
            clock_game_time: 0,
            chunk_center: (0, 0),
            awaiting_viewed_chunk_center: false,
            chunk_light: RemoteLight::default(),
            backlog: VecDeque::new(),
            backlog_since: None,
            server_links: Vec::new(),
            debug_query: Default::default(),
            synced_attributes: HashMap::new(),
        });
        self.set_local_mode_with_previous(sunrise_client::command::args::GameMode::Survival, None);
        self.clock = sunrise_client::clock::FixedStep::client();
        self.adv = Default::default();
        self.stats = Default::default();
        Ok(())
    }
    /// Refuse invalid server data with the normal connection-error screen.
    pub(in crate::game) fn refuse_remote_input(
        &mut self,
        error: impl std::fmt::Display,
    ) -> Result<()> {
        log::warn!("Sunrise: {error}");
        self.leave_server(Some(Text::translate(
            "disconnect.genericReason",
            &[Text::literal(error.to_string())],
        )))
    }

    /// Disconnect and return to the multiplayer screen (DisconnectedScreen).
    pub(in crate::game) fn leave_server(&mut self, reason: Option<Text>) -> Result<()> {
        if reason.is_none() && self.integrated.as_ref().is_some_and(|s| !s.stopped()) {
            return self.begin_closing_save(false);
        }
        self.close_server_session(reason, |_| {})
    }

    /// Close the transport before finishing any pack feedback, but keep the pack manager's
    /// prompt state until that feedback has been processed.
    pub(in crate::game) fn close_server_session(
        &mut self,
        reason: Option<Text>,
        before_pack_cleanup: impl FnOnce(&mut crate::game::server_packs::ServerPackState),
    ) -> Result<()> {
        self.client_logging_out();
        self.finish_leave_server_with(reason, before_pack_cleanup)
    }

    /// Tear down the client's server session without firing `client_logging_out`:
    /// used after `client_logging_out` has already run (`leave_server`, integrated
    /// server close), and for aborted or failed `ConnectScreen` / login-handshake
    /// attempts that never reached `ClientCommonPacketListenerImpl` or
    /// `Minecraft.disconnect`.
    pub(in crate::game) fn finish_leave_server(&mut self, reason: Option<Text>) -> Result<()> {
        self.finish_leave_server_with(reason, |_| {})
    }

    fn finish_leave_server_with(
        &mut self,
        reason: Option<Text>,
        before_pack_cleanup: impl FnOnce(&mut crate::game::server_packs::ServerPackState),
    ) -> Result<()> {
        self.finish_perf();
        // The one boundary every network session passes when it closes here: the player leaving,
        // or this client ending it with a reason (a raw disconnect closed it already, with the
        // connection's or the server's).
        match &reason {
            None => self.connection_left(sunrise_hooks::connection::Left::Disconnected),
            Some(reason) => self.connection_ended(
                &reason.plain(),
                reason.translation_key(),
                sunrise_hooks::connection::By::Client,
            ),
        }
        self.clear_waypoints();
        self.hud_disconnected();
        // Vanilla's Hud also ticks on the title and connect screens, so the
        // action bar has run out by the next world; Sunrise's ticks in one.
        self.titles.overlay_time = 0;
        // GameRenderer.resetData and narrator.clear (Minecraft.disconnect).
        self.item_activation.reset();
        // Kept mod block entity geometry belongs to the world leaving.
        self.kept_block_entities.get_mut().clear();
        self.mod_view_distances.take();
        self.clear_narrator();
        // A world is leaving either way: its icon attempt goes with it.
        self.world_icon = None;
        // Minecraft.disconnect: the client level goes.
        if self.remote.is_some() {
            self.forget_all_remote_columns();
        }
        if let Some(old) = self.remote.as_mut().and_then(|r| r.heard_level.take()) {
            self.native_client_level_unload(&old);
        }
        self.native_net.holds.borrow_mut().clear();
        self.native_client_commands.pending_commands.clear();
        self.native_client_commands.pending_chats.clear();
        if let Some(mut r) = self.remote.take() {
            r.session.disconnect();
            self.clear_spectated_entity_post_effect();
            self.reset_creative_contents();
            self.refresh_creative_search_tags();
        }
        // Minecraft.disconnect -> updateLevelInEngines(null), after logout and level
        // unload: stop even the sounds a listener started during teardown.
        self.ambience.clear(&mut self.audio);
        self.music.stop_playing(None, &mut self.audio);
        self.audio.leave_level();
        self.mod_sounds.clear();
        self.update_window_title();
        self.gui.atlas.clear_object_skins();
        before_pack_cleanup(&mut self.server_packs);
        self.server_packs_disconnect();
        self.code_of_conduct = None;
        self.reset_tutorial();
        self.close_dialogs();
        // Gui.setScreen -> KeyMapping.releaseAll: the title or disconnect
        // screen releases every held key. Its release goes to that screen,
        // so a key kept here would still be down in the next world (#3246).
        self.clear_device_keys();
        self.set_mapping_held("key.attack", false);
        self.set_mapping_held("key.use", false);
        // The shared world is closing: its invite code ends with it.
        self.friends_lan_closed();
        // Minecraft.disconnect waits for the integrated server before showing the title.
        let stopped = self
            .integrated
            .take()
            .map(|server| server.stop())
            .transpose();
        // Keep the server's registry snapshot until the thread has joined, even on error.
        super::super::lan::sync::forget_sync_data();
        let single_player = stopped?.is_some();
        mod_api::content::crash::clear_detail("Server");
        self.clock = Default::default();
        self.adv = Default::default();
        self.stats = Default::default();
        self.score.board = Default::default();
        self.streaming = Streaming::new(Arc::new(NoTerrain), self.worker_context(), 1);
        self.streaming.paused = true;
        self.reset_client_world(0);
        self.remove_local_player_body();
        self.post_effects.clear();
        self.clear_spectated_entity_post_effect();
        // The vehicle went with the level: an id kept here would seat the
        // next login on an entity that is not there.
        self.riding = None;
        // Out of the server's bed: InBedChatScreen goes with the level.
        self.sleeping = None;
        self.sleep_timer = 0;
        self.clear_entities();
        self.score.board = Default::default();
        self.tab_list = Default::default();
        self.border = Default::default();
        self.set_screen(Screen::Playing);
        self.container = None;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        let saves = std::env::current_dir()?.join("saves");
        let mut title = self
            .new_title(saves.clone())
            .with_user(|| self.options.player_name());
        title.last_server = self.settings.file.string("lastServer");
        title.skip_multiplayer_warning = self.settings.file.bool("skipMultiplayerWarning");
        if single_player && reason.is_none() {
            self.show_title(title);
            self.capture(false);
            return Ok(());
        }
        title.mode = sunrise_client::title::Mode::Multiplayer(match reason {
            Some(reason) => sunrise_client::multiplayer::Screen::Disconnected {
                reason,
                retry: None,
                lines: std::cell::Cell::new(1),
            },
            None => sunrise_client::multiplayer::Screen::list(&saves),
        });
        self.show_title(title);
        self.offer_retry();
        self.capture(false);
        Ok(())
    }
    /// Switch to a (possibly new) dimension: a fresh client level.
    pub(in crate::game) fn enter_remote_dimension(
        &mut self,
        spawn: &play::SpawnInfo,
        entry: RemoteDimensionEntry,
        reason: sunrise_client::level_loading::Reason,
    ) {
        // The replacement player starts with its inventory menu, even if the
        // server switched levels without closing the previous container.
        self.remote_close_local();
        self.carried = None;
        self.craft_grid.fill(None);
        self.set_screen(Screen::Playing);
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        r.craft_result = None;
        r.state_id = 0;
        // handleRespawn keeps the ClientLevel only for the same level key:
        // two levels sharing a dimension type are still two levels.
        let same = matches!(
            entry,
            RemoteDimensionEntry::Respawn | RemoteDimensionEntry::DeathRespawn
        ) && r.heard_level.as_deref() == Some(spawn.dimension.as_str());
        // An open LevelLoadingScreen keeps its presentation, even if it is vanilla.
        // New screens for login and death do not receive transition keys.
        let transition_from = if matches!(entry, RemoteDimensionEntry::Respawn)
            && !r.level_load.is_active()
            && r.heard_level.is_some()
        {
            Some(r.level_key)
        } else {
            None
        };
        let adjust_mode = if matches!(
            entry,
            RemoteDimensionEntry::Respawn | RemoteDimensionEntry::DeathRespawn
        ) {
            self.mode
        } else {
            sunrise_client::command::args::GameMode::Survival
        };
        let mode = crate::game::game_mode::mode_from_id(spawn.game_type);
        let previous = spawn
            .previous_game_type
            .map(crate::game::game_mode::mode_from_id);
        let body_flight = self.player.as_ref().map(|p| (p.flying, p.flying_speed));
        let r = self.remote.as_mut().unwrap();
        let new_logical_player =
            !matches!(entry, RemoteDimensionEntry::Login) || !r.has_logical_player;
        // A Sunrise server's bulk edit sends whole columns with the light
        // from before its relight; any other server's packet light is final.
        let relight_resent = r.quiet_commands;
        let format = r.level_format(spawn);
        let level_key = mod_api::__private::dimension_key(&spawn.dimension);
        let replacement = if same {
            None
        } else {
            // ClientLevel's constructor posts Load before Minecraft.setLevel unloads
            // the old level. Give the new level its own handle without retiring the
            // old one, which must still read its existing chunks during Unload.
            let epoch = crate::game::native_lifecycle::fresh_epoch();
            let handle = mod_api::LevelRef::new(epoch, level_key);
            let mut world = Game::world_for_mods(self.res.facts().air, spawn.seed);
            world.shape = format.shape;
            world.game_time = self.world.game_time;
            world.weather = self.world.weather;
            r.constructing_level = Some(ConstructingClientLevel {
                handle,
                world,
                facts: format.facts.clone(),
            });
            self.native_client_level_load(&spawn.dimension);
            let level = self
                .remote
                .as_mut()
                .unwrap()
                .constructing_level
                .take()
                .unwrap();
            self.unload_client_level();
            self.forget_all_remote_columns();
            Some(level)
        };
        // Keep the retiring level's metadata in place until its Unload has returned.
        self.track_remote_levels(&spawn.dimension);
        let r = self.remote.as_mut().unwrap();
        if !same {
            r.collider_pass_start = None;
            // A same-level respawn keeps the existing ClientLevel's seed and sea level.
            r.hashed_seed = spawn.seed;
            r.sea_level = spawn.sea_level;
        }
        let dimension = format.dimension;
        r.chunk_format = format.chunk_format;
        r.level_facts = format.facts;
        r.synced_type = format.synced_type;
        r.dimension = dimension;
        r.resolve_clocks();
        self.world.day_time = r.clock_time(r.default_clock);
        r.level = spawn.dimension.clone();
        r.level_key = level_key;
        if new_logical_player {
            self.drop_spam_throttler = Default::default();
            r.abilities = sunrise_client::lan::abilities::PlayerAbilities::default();
            r.own_player_info = std::cell::OnceCell::new();
            // The new LocalPlayer, admitted now, counts its ticks from 0.
            r.own_tick_count = 0;
        } else if let Some((flying, flying_speed)) = body_flight {
            r.sync_body_flight(flying, flying_speed);
        }
        r.has_logical_player = true;
        // handleLogin/handleRespawn run adjustPlayer(newPlayer) before setLocalMode.
        r.abilities.update_for_mode(adjust_mode.id());
        r.flat = spawn.flat;
        r.loaded_sent = false;
        r.abilities = None;
        // A replacement LevelLoadingScreen starts its bar from nothing (the
        // first one carries on from the server's start).
        if self.player.is_some() {
            self.loading_progress = 0.0;
        }
        // startWaitingForNewLevel: `LevelLoadTracker.startClientLoad` and the
        // `LevelLoadingScreen`.
        let transition_factory = transition_from.and_then(|from| {
            sunrise_client_render::dimension_transition::select(r.level_key, from)
        });
        r.level_load.start_client_load(client_millis(), reason);
        // A replacement LocalPlayer has a fresh ambient mood and sendPosition baseline.
        self.ambience.moodiness = 0.0;
        r.position_sender = Default::default();
        r.predictions.clear();
        self.set_local_mode_with_previous(mode, previous);
        self.dim = Dim::from_id(&spawn.dimension);
        self.build_level_environments();
        self.sync_timeline_registries();
        // ClientPacketListener.handleLogin / handleRespawn: the server's
        // Player.getLastDeathLocation, for the recovery compass.
        self.level.last_death = spawn
            .last_death
            .as_ref()
            .map(|d| (d.dimension.clone(), d.pos));
        if let Some(mut level) = replacement {
            self.clock.reset_client_level_tick_state();
            let mut streaming = Streaming::new(
                Arc::new(NoTerrain),
                self.worker_context(),
                self.res.threads(),
            );
            streaming.remote = true;
            streaming.radius = self.streaming.radius;
            // SpawnInfo already carries the hashed seed the client's BiomeManager holds.
            streaming.biome_zoom = sunrise_core::biome::BiomeZoom::new(spawn.seed);
            self.streaming = streaming;
            level.world.day_time = self.world.day_time;
            self.install_client_world(level.world);
            if let Some(remote) = self.remote.as_ref() {
                self.world
                    .chunks
                    .track_connection_groups(remote.group_count.clone());
            }
            // The border and weather belong to this ClientLevel, not the connection.
            self.border = Default::default();
            // A same-level respawn keeps the level; a replacement retires the
            // old handles and adopts the epoch already given to Load listeners.
            self.native_lifecycle_open_at(level.handle.epoch());
            self.native_lifecycle_world_ready();
            self.mobs.clear();
            self.clear_remote_objects();
            self.clear_remote_projectiles();
            self.dropped.clear();
            self.clear_client_level_particles();
            if let Some(r) = self.remote.as_mut() {
                r.heard_level = Some(spawn.dimension.clone());
                r.entities.clear();
                r.synced_attributes.clear();
                r.passenger_vehicles.clear();
                r.lerps.clear();
                r.general.clear();
                r.cart_lerps.clear();
                r.stepped.clear();
                r.head_lerps.clear();
                r.pending_items.clear();
                // The entity ids are the old world's; the new dimension's
                // player entities re-map to their profiles on AddEntity.
                r.level_players.clear();
                r.level_uuids.clear();
                r.level_entity_uuids.clear();
                r.level_player_infos.clear();
                r.level_player_names.clear();
                r.level_player_textures.clear();
                // A new ClientLevel: a new chunk storage, centred on 0, 0.
                r.chunk_center = (0, 0);
                r.awaiting_viewed_chunk_center = false;
            }
            self.clear_lod();
            self.pending_uploads.clear();
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.clear_sections(self.world.layout());
            }
            // Minecraft.setLevel -> updateLevelInEngines, after the old-level unload.
            self.stop_sounds_for_new_level();
        }
        self.audio.set_world(&spawn.dimension);
        self.streaming.relight_resent = relight_resent;
        let has_sky = self
            .remote
            .as_ref()
            .is_none_or(|r| r.dimension.has_skylight);
        self.streaming.sky_light = has_sky;
        self.streaming.nether_shading = self.nether_lighting();
        let player = self.player.as_ref();
        self.particles
            .detach_tracking_target(TrackingTarget::LocalPlayer, || {
                player.map(player_tracking_body)
            });
        if new_logical_player {
            self.drop_remote_player();
            // handleLogin / handleRespawn: ClientLevel.addEntity admits the local player
            // now, before the position packet creates its motion body.
            self.resident_entities.local_player.renew();
        } else {
            self.retain_remote_player_on_login();
        }
        // handleLogin / handleRespawn: LocalPlayer.setPortalCooldown, so a
        // player who arrives standing in a portal gets no overlay from it.
        self.portals.player.cooldown = spawn.portal_cooldown;
        if let Some(profile) = self.remote.as_ref().map(|remote| remote.profile) {
            self.initialize_entity(Holder::Player(profile), "minecraft:player");
        }
        self.finish_remote_level();
        if let Some(registration) = transition_factory
            && registration.factory_owner.is_enabled()
            && let Ok(presentation) = registration.factory_owner.call(
                "dimension transition screen",
                "dimension_transition_screen_factory",
                || {
                    sunrise_client_render::dimension_transition::ActiveTransitionScreen::new(
                        registration.draw_owner.clone(),
                        registration.factory.create(self, reason.into()),
                    )
                },
            )
            && let Some(r) = self.remote.as_mut()
        {
            r.level_load.set_transition_screen(presentation);
        }
    }

    /// `ClientPacketListener.handleLogin` when `Minecraft.player != null`: retain the live player, run `resetPos`, rebind residency, and reset input.
    fn retain_remote_player_on_login(&mut self) {
        self.reset_client_item_use();
        self.hook.discard();
        if let Some(r) = self.remote.as_mut() {
            r.respawn_velocity = None;
        }
        let retained_max_health = self.player.as_mut().map(|player| {
            let _ = player.set_pose(sunrise_client::pose::Pose::Standing);
            player.velocity = Vec3::ZERO;
            player.pitch = 0.0;
            let max_health = player
                .attribute("minecraft:max_health")
                .ok()
                .map(|v| v as f32);
            if let Some(max) = max_health {
                let _ = player.set_health(max, false);
            }
            max_health
        });
        if let Some(max_health) = retained_max_health {
            self.pitch = 0.0;
            if let Some(max) = max_health
                && let Some(r) = self.remote.as_mut()
            {
                r.health.0 = max;
            }
            self.ui.death_time = 0;
            self.input_player_replaced();
        }
        self.bind_local_player_residency();
    }
}
