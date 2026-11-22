//! Receiving-provider credit follows ordered events and decoder placeholders.
use super::{session::RemoteOpening, *};

fn local_transfer_notice() -> Event {
    Event::Disconnected {
        reason: sunrise_client::lang::get("disconnect.transfer"),
        key: Some("disconnect.transfer".into()),
        from_server: true,
    }
}

#[derive(Default)]
struct ListenerTurn {
    opening: Option<RemoteOpening>,
    end: Option<Event>,
}

impl Remote {
    pub(super) fn receive_credited_backlog(&mut self, events: &mut VecDeque<Received>) {
        // Chunk packets start decoding now; the frame that reaches one in
        // order inserts its column. Packets inside a delimited bundle stay
        // inline so the entire bundle dispatches atomically in one turn.
        for receipt in self.session.poll() {
            let (event, credit) = receipt.into_parts();
            events.push_back(match event {
                Event::Packet(p) if matches!(*p, C::LevelChunkWithLight(_)) => {
                    match self.decoder.submit(self.chunk_format.clone(), p) {
                        None => Received::Chunk(self.chunk_format.clone(), credit),
                        Some(p) => Received::Event(Event::Packet(p), credit),
                    }
                }
                event => Received::Event(event, credit),
            });
        }
    }
}

impl Game<'_> {
    #[cfg(test)]
    pub(in crate::game) fn wait_for_test_remote_event(&mut self, until: impl Fn(&Event) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let remote = self.remote.as_mut().unwrap();
        let mut backlog = std::mem::take(&mut remote.backlog);
        loop {
            remote.receive_credited_backlog(&mut backlog);
            if backlog
                .iter()
                .any(|item| matches!(item, Received::Event(event, _) if until(event)))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "expected event did not reach main"
            );
            std::thread::yield_now();
        }
        remote.backlog = backlog;
    }

    #[cfg(test)]
    pub(in crate::game) fn acknowledge_test_configuration(&mut self) {
        self.wait_for_test_remote_event(|event| matches!(event, Event::FinishConfiguration(_)));
        self.poll_remote_events(true).unwrap();
    }

    /// `Minecraft.runTick`: exhaust the packet phase before starting client tasks. A
    /// worker-decoded column defers its position, every later packet and the task phase.
    pub(super) fn poll_remote_events(&mut self, before_tick: bool) -> Result<()> {
        let mut receive = !before_tick;
        let mut chunks = Duration::ZERO;
        loop {
            // Capture tasks before checking the packet queue, so a task published after the
            // empty packet boundary cannot overtake a packet that arrived before that task.
            self.pending_client_tasks
                .append(&mut self.client_tasks.take_turn());
            let Some(remote) = self.remote.as_mut() else {
                break;
            };
            let mut events = std::mem::take(&mut remote.backlog);
            if receive {
                remote.receive_credited_backlog(&mut events);
            }
            if events.is_empty() {
                remote.backlog_since = None;
                break;
            }
            let overdue = remote
                .backlog_since
                .is_some_and(|since| since.elapsed() >= BACKLOG_LIMIT);
            self.dispatch_packet_turn(events, overdue, before_tick, &mut chunks)?;
            if self
                .remote
                .as_ref()
                .is_some_and(|remote| !remote.backlog.is_empty())
            {
                return Ok(());
            }
            // A handler may have published more packets or installed another listener.
            receive = true;
        }
        self.send_suggestion_request();
        while self.client_tasks.is_open()
            && let Some(task) = self.pending_client_tasks.pop_front()
        {
            task.run(self);
        }
        Ok(())
    }

    fn dispatch_packet_turn(
        &mut self,
        mut events: VecDeque<Received>,
        overdue: bool,
        before_tick: bool,
        chunks: &mut Duration,
    ) -> Result<()> {
        // A run of ForgetLevelChunk (thousands when the server's view moves far: a
        // teleport at 32 chunks) is forgotten at once, before whatever follows it.
        let mut forgotten = sunrise_client::fast_hash::FxHashSet::default();
        let mut forgotten_credits = Vec::new();
        let mut turn = ListenerTurn::default();
        while let Some(item) = events.pop_front() {
            // PacketProcessor checks eligibility for the queued outer action,
            // before any handler, including worker-decoded columns.
            let needs_open = match &item {
                Received::Event(event, _) => event.requires_open_listener(),
                Received::Chunk(..) => true,
            };
            if needs_open
                && !self
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.session.accepts_packets())
            {
                continue;
            }
            if let Received::Event(Event::Packet(packet), _) = &item
                && let C::ForgetLevelChunk(p) = &**packet
            {
                self.observe_client_packet(packet);
                forgotten.insert((p.pos[0], p.pos[1]));
                forgotten_credits.push(item.into_event_credit());
                continue;
            }
            sunrise_client::spans::time("pr_forget", || {
                self.forget_remote_columns(&mut forgotten, true)
            });
            forgotten_credits.clear();
            let chunk = matches!(item, Received::Chunk(_, _));
            // Starling: an overdue backlog takes all its chunk packets, or on a phone
            // `OVERDUE_CHUNK_PACKETS_PER_FRAME` of them.
            let budget = match overdue {
                false => CHUNK_PACKETS_PER_FRAME,
                true if cfg!(target_os = "android") => OVERDUE_CHUNK_PACKETS_PER_FRAME,
                true => Duration::MAX,
            };
            if chunk && !before_tick && *chunks >= budget {
                events.push_front(item);
                break;
            }
            let started = chunk.then(Instant::now);
            let (event, _credit) = match item {
                Received::Event(event, credit) => (event, credit),
                Received::Chunk(format, credit) => {
                    // A column still decoding waits for the next frame, with
                    // what came after it, overdue or not.
                    let Some(next) = self.remote.as_ref().map(|r| {
                        if before_tick {
                            r.decoder.wait_next(TICK_DECODE_WAIT)
                        } else {
                            r.decoder.next()
                        }
                    }) else {
                        break;
                    };
                    let (packet, column) = match next {
                        Next::Ready(decoded) => decoded,
                        Next::Decoding => {
                            events.push_front(Received::Chunk(format, credit));
                            break;
                        }
                        Next::Lost => {
                            log::warn!("Sunrise: the chunk decoder stopped; a column is missing");
                            continue;
                        }
                    };
                    if !self
                        .remote
                        .as_ref()
                        .is_some_and(|r| r.session.accepts_packets())
                    {
                        continue;
                    }
                    self.observe_client_packet(&packet);
                    if let C::LevelChunkWithLight(p) = *packet {
                        sunrise_client::spans::time("pr_chunk", || {
                            self.remote_chunk(&p, Some((&format, column)))
                        });
                    }
                    *chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
                    continue;
                }
            };
            self.handle_remote_event(event, &mut turn)?;
            if turn.opening.is_some() {
                // The handoff already closed reader admission. Freeze any
                // published tail before dropping any older receipt, then keep
                // the old listener installed for its eligible common tasks.
                let tail = self
                    .remote
                    .as_ref()
                    .map(|old| old.session.poll())
                    .unwrap_or_default();
                let admitted = events
                    .into_iter()
                    .filter_map(|item| match item {
                        Received::Event(event, credit) => Some((event, credit)),
                        Received::Chunk(..) => None,
                    })
                    .chain(tail.into_iter().map(|receipt| receipt.into_parts()));
                for (event, _credit) in admitted {
                    if matches!(event, Event::Cookie(_) | Event::Transfer(_)) {
                        self.handle_remote_event(event, &mut turn)?;
                    }
                }
                self.install_remote_transfer(
                    turn.opening.take().expect("handoff kept its first opening"),
                )?;
                return Ok(());
            }
            *chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
            if self.remote.is_none() {
                break;
            }
        }
        if let Some(end) = turn.end.take() {
            self.finish_remote_disconnect(end)?;
            return Ok(());
        }
        if self.remote.is_some() {
            sunrise_client::spans::time("pr_forget", || {
                self.forget_remote_columns(&mut forgotten, true)
            });
        }
        if let Some(r) = self.remote.as_mut() {
            r.backlog_since = if events.is_empty() {
                None
            } else {
                Some(r.backlog_since.unwrap_or_else(Instant::now))
            };
            r.backlog = events;
        }
        Ok(())
    }

    /// `ClientPacketListener.handleBundlePacket`: use one main-thread turn, so
    /// the chunk decoder/frame quota cannot split a bundle at a column packet.
    fn handle_play_bundle(
        &mut self,
        bundle: sunrise_client::net::CompletedBundle,
        turn: &mut ListenerTurn,
    ) -> Result<()> {
        use sunrise_client::net::{BundleMember, BundleStep};
        use sunrise_client::raw_payloads::{Classified, Incoming, dispatch_bundle};
        use sunrise_protocol::{connection_groups, payload_negotiation::Phase};
        let sunrise_client::net::CompletedBundle { packets, io } = bundle;
        let Some(remote) = self.remote.as_ref() else {
            return Ok(());
        };
        if !io.accepts_bundle() {
            return Ok(());
        }
        let endpoint = remote.session.payload_endpoint();
        let negotiated = endpoint.transport().epoch().is_some_and(|epoch| {
            epoch.phase == Phase::Play
                && epoch
                    .peer
                    .engine_receives(Phase::Play, connection_groups::CHANNEL)
        });
        let channel = |member: &BundleMember, channel: &str| matches!(member, BundleMember::Packet(C::CustomPayload(p)) if p.channel == channel);
        if packets
            .iter()
            .any(|member| channel(member, connection_groups::CHANNEL))
            || (packets.len() > 1
                && packets
                    .iter()
                    .all(|member| channel(member, sunrise_protocol::viewed_levels::CHANNEL)))
        {
            let Some(group_packets) = packets
                .into_iter()
                .map(|member| match member {
                    BundleMember::Packet(packet) => Some(packet),
                    BundleMember::AttachmentSync(staged) => {
                        Some(C::CustomPayload(staged.into_message().1))
                    }
                    BundleMember::OversizedDebugSample => None,
                })
                .collect::<Option<Vec<_>>>()
            else {
                return self.refuse_play_payload("connection groups: unexpected packet in bundle");
            };
            self.route_connection_group_bundle(&group_packets, negotiated)?;
            return Ok(());
        }
        // One PacketProcessor task: a handler failure leaves the prefix applied,
        // skips the suffix, and disconnects via onPacketError.
        let stop_on_packet_error = |game: &Self, turn: &mut ListenerTurn| {
            if let Some(remote) = &game.remote {
                remote.session.close_after_pending();
            }
            turn.end.get_or_insert_with(|| io.packet_error());
        };
        for packet in packets {
            match io.bundle_member(packet) {
                BundleStep::Event(event) => {
                    if let Err(error) = self.handle_remote_event(event, turn) {
                        log::warn!("Sunrise: bundle packet handler failed: {error}");
                        stop_on_packet_error(self, turn);
                        break;
                    }
                }
                BundleStep::HandlerFailed => {
                    log::warn!("Sunrise: oversized debug sample in play bundle");
                    stop_on_packet_error(self, turn);
                    break;
                }
                BundleStep::Payload(payload) => {
                    match Incoming::classify(endpoint.transport(), Phase::Play, payload) {
                        Ok(Classified::Application(incoming)) => {
                            for task in dispatch_bundle(incoming, &endpoint) {
                                self.run_payload(endpoint.clone(), task);
                            }
                        }
                        Ok(Classified::Other(payload)) => {
                            self.handle_remote_event(
                                Event::Packet(Box::new(C::CustomPayload(payload))),
                                turn,
                            )?;
                        }
                        Ok(Classified::Skipped | Classified::Raw(_)) => {}
                        Err(error) => {
                            log::warn!("Sunrise: bundle payload refused: {error}");
                            if let Some(remote) = &self.remote {
                                remote.session.close_after_pending();
                            }
                            turn.end
                                .get_or_insert_with(|| io.incompatible_payload(&error.to_string()));
                        }
                    }
                }
                BundleStep::Transfer(packet) => self.handle_listener_transfer(packet, turn)?,
                BundleStep::Close(notice) => {
                    if let Some(remote) = &self.remote {
                        remote.session.close_after_pending();
                    }
                    // Discard the old listener only after its remaining handlers run.
                    turn.end.get_or_insert(notice);
                }
                BundleStep::Continue => {}
            }
        }
        Ok(())
    }

    fn handle_listener_transfer(
        &mut self,
        packet: sunrise_protocol::packets::configuration::Transfer,
        turn: &mut ListenerTurn,
    ) -> Result<()> {
        // ConnectScreen.startConnecting keeps the first opening, even if a
        // later eligible Transfer runs as a separate old-listener task.
        if turn.opening.is_none() {
            turn.opening = self.start_remote_transfer(packet)?;
            if turn.opening.is_none() {
                turn.end.get_or_insert_with(local_transfer_notice);
            }
        }
        Ok(())
    }

    fn handle_remote_event(&mut self, event: Event, turn: &mut ListenerTurn) -> Result<()> {
        match event {
            Event::Status(s) => {
                if let Some(r) = self.remote.as_mut() {
                    r.status = s;
                }
            }
            Event::Joined {
                registries,
                name,
                uuid,
            } => {
                self.set_registries(registries);
                self.connection_joined();
                self.close_dialogs();
                if let Some(r) = self.remote.as_mut() {
                    r.joined = true;
                    r.starling.play_began = Some(Instant::now());
                    r.profile = uuid;
                    r.profile_name = name.clone();
                    r.status = sunrise_client::lang::get_or(
                        "multiplayer.downloadingTerrain",
                        "Loading terrain...",
                    );
                }
                self.begin_book_negotiation();
            }
            Event::Features(features) => {
                if let Some(remote) = &mut self.remote {
                    remote.features = features.into();
                }
            }
            Event::Brand(brand) => {
                if let Some(remote) = &mut self.remote {
                    remote.brand = brand;
                }
            }
            Event::QuietCommands => {
                if let Some(remote) = &mut self.remote {
                    remote.quiet_commands = true;
                }
            }
            Event::CustomBooks => {
                if let Some(remote) = &mut self.remote {
                    remote.custom_books = true;
                }
            }
            Event::FarTerrain(offer) => {
                if let Some(remote) = &mut self.remote {
                    remote.far_terrain = Some(offer);
                }
            }
            Event::Cookie(task) => task.handle(),
            Event::Configuration(task) => {
                if let Some(effect) = task.handle() {
                    self.handle_remote_event(effect, turn)?;
                }
            }
            Event::FinishConfiguration(change) => change.acknowledge(),
            Event::Reconfiguring(resume) => {
                self.reset_book_view();
                // handleConfigurationStart: the held-back chat shows, then
                // clearClientLevel opens ServerReconfigScreen over no level.
                self.flush_chat_queue();
                self.send_chat_acknowledgement();
                self.close_dialogs();
                // The server configuring us now (a proxy's next one)
                // offers its own far terrain, or none, and announces its
                // own features again.
                if let Some(remote) = &mut self.remote {
                    remote.viewed.clear();
                    remote.far_terrain = None;
                    remote.quiet_commands = false;
                    remote.custom_books = false;
                    remote.cauldron_entries = None;
                    remote.pending_cauldrons.get_mut().clear();
                }
                self.clear_lod();
                if let Some(remote) = &self.remote {
                    remote.session.reset_far_terrain();
                }
                self.remove_local_player_body();
                self.set_screen(Screen::Playing);
                self.capture(false);
                // clearClientLevel drops the client level now, so its
                // unload comes before the configuration screen, not
                // after the next login's load.
                self.forget_all_remote_columns();
                if let Some(old) = self.remote.as_mut().and_then(|r| r.heard_level.take()) {
                    self.native_client_level_unload(&old);
                }
                // Minecraft.clearClientLevel -> updateLevelInEngines(null) stops
                // sound immediately after the level-unload event fires.
                self.stop_sounds_for_new_level();
                self.native_lifecycle.begin_close();
                self.native_lifecycle_world_closed();
                let ticks = self.ticks;
                if let Some(r) = self.remote.as_mut() {
                    self.native_net.holds.borrow_mut().clear();
                    r.reconfiguring = Some(ticks);
                    r.chat_tree_received = false;
                }
                resume.acknowledge();
            }
            Event::Reconfigured(registries) => {
                // handleConfigurationStart → clearClientLevel, and the next
                // ClientPacketListener starts with no players or scoreboard,
                // no waypoints (its own ClientWaypointManager: a proxy's next
                // server never untracks the last one's), and its new
                // ClientSuggestionProvider with no custom chat completions.
                // clearClientLevel clears the narrator.
                self.clear_narrator();
                self.hud_disconnected();
                self.score.board = Default::default();
                self.clear_waypoints();
                if let Some(r) = self.remote.as_mut() {
                    r.clear_attachments();
                    r.players.clear();
                    r.tab_players.clear();
                    r.listed.clear();
                    r.signatures = Default::default();
                    r.custom_completions.clear();
                }
                self.ui.registries.custom_completions.clear();
                self.set_registries(registries);
            }
            Event::Transfer(task) => {
                if let Some(packet) = task.into_eligible_packet() {
                    self.handle_listener_transfer(packet, turn)?;
                }
            }
            Event::PackPush(p) => self.handle_pack_push(p),
            Event::PackPop(id) => self.handle_pack_pop(id),
            Event::ServerLinks(links) => {
                if let Some(r) = self.remote.as_mut() {
                    r.server_links = crate::game::server_links::trusted(links);
                }
            }
            Event::ShowDialog { dialog, registries } => {
                self.show_dialog_in_phase(&dialog, Some(registries));
            }
            Event::ClearDialog => self.clear_dialog(),
            Event::CodeOfConduct(text) => self.handle_code_of_conduct(text),
            Event::DecodeError(e) => {
                if let Some(r) = self.remote.as_mut() {
                    r.decode_errors += 1;
                    if r.decode_errors <= 20 {
                        log::warn!("Sunrise: skipped undecodable packet {e}");
                    }
                }
            }
            event @ Event::Disconnected { .. } => {
                turn.end.get_or_insert(event);
            }
            Event::Bundle(bundle) => self.handle_play_bundle(*bundle, turn)?,
            Event::RawPayload(incoming) => {
                if let Some(remote) = self.remote.as_ref() {
                    self.run_payload(remote.session.payload_endpoint(), *incoming);
                }
            }
            Event::ObservePacket(packet) => self.observe_client_packet(&packet),
            Event::AttachmentSync(staged) => self.apply_staged_attachment_sync(*staged),
            Event::Packet(packet) => match *packet {
                C::CustomPayload(p) if p.channel == sunrise_client::perf_net::CHANNEL => {
                    self.remote_perf_reply(&p.data)
                }
                packet => self.handle_packet(packet)?,
            },
        }
        Ok(())
    }

    pub(in crate::game) fn apply_staged_attachment_sync(
        &mut self,
        staged: sunrise_protocol::attachment_sync::StagedMessage,
    ) {
        let (message, payload) = staged.into_message();
        self.observe_client_packet(&C::CustomPayload(payload));
        self.remote_attachment_sync(message);
    }

    fn finish_remote_disconnect(&mut self, event: Event) -> Result<()> {
        let Event::Disconnected {
            reason,
            key,
            from_server,
        } = event
        else {
            unreachable!("listener only retains the disconnect notice");
        };
        self.reset_book_view();
        log::info!("Sunrise: disconnected: {reason}");
        let by = if from_server {
            sunrise_hooks::connection::By::Server
        } else {
            sunrise_hooks::connection::By::Connection
        };
        self.connection_ended(&reason, key.as_deref(), by);
        self.leave_server(Some(Text::literal(reason)))
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{connect_play_test, spawn_menu_test};
    use super::*;
    use crate::game::native_entity_refs::tests::with_client_game;
    use mod_api::{ClientHost, ClientPacketEvent, EntityRef, Hooks, ModEntry};
    use std::sync::mpsc;

    #[test]
    #[ignore = "daily"]
    fn executor_sees_entity_removal_published_during_packet_dispatch() {
        struct Listener {
            publish: Box<dyn Fn(C) + Send + Sync>,
            entity: EntityRef,
            record: mpsc::Sender<Option<[f64; 3]>>,
        }

        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn client_packet(&self, host: &mut dyn ClientHost, event: &mut ClientPacketEvent<'_>) {
                if event.packet.id.path() != "ping" {
                    return;
                }
                let entity = self.entity;
                let record = self.record.clone();
                host.client_executor()
                    .unwrap()
                    .execute(Box::new(move |host| {
                        record.send(entity.position(host)).unwrap()
                    }));
                // Publish behind the callback while the first packet is still being handled.
                (self.publish)(C::RemoveEntities(play::RemoveEntities {
                    ids: vec![entity.id() as i32],
                }));
            }
        }

        with_client_game(|game| {
            let (_reader, _writer) = connect_play_test(game, |_, _| {});
            spawn_menu_test(game);
            let publisher = game.remote.as_ref().unwrap().session.received_event_probe();
            let publish = move |packet| publisher(Event::Packet(Box::new(packet)));
            game.spawn_entity(20, "minecraft:pig", [2.0, 64.0, 0.0], [0.0; 3]);
            let entity = game.entity(20).unwrap();
            assert_eq!(entity.position(game), Some([2.0, 64.0, 0.0]));

            let (record, calls) = mpsc::channel();
            publish(C::Ping(sunrise_protocol::packets::configuration::Ping {
                id: 1,
            }));
            let mods = vec![ModEntry::new(
                "during_packet",
                Box::new(Listener {
                    publish: Box::new(publish),
                    entity,
                    record,
                }),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || game.poll_remote().unwrap());
            assert_eq!(calls.try_recv(), Ok(None));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_bundled_long_debug_sample_applies_only_the_prefix_before_packet_error() {
        use crate::game::native_entity_refs::tests::run_fresh_cases;
        if run_fresh_cases(
            "game::remote::raw_ingress::tests::a_bundled_long_debug_sample_applies_only_the_prefix_before_packet_error",
            "SUNRISE_DEBUG_SAMPLE_BUNDLE_CHILD",
            &["debug-sample"],
        ) {
            return;
        }
        use std::sync::{Arc, Mutex};
        use sunrise_client::net::{PacketRecorder, RecorderStatus};
        use sunrise_protocol::ids::play::clientbound::{BUNDLE_DELIMITER, SET_HELD_SLOT};

        struct RecordIds(Arc<Mutex<Vec<i32>>>);
        impl PacketRecorder for RecordIds {
            fn is_active(&self) -> bool {
                true
            }
            fn received(&self, _: &C, id: i32, _: &[u8]) {
                self.0.lock().unwrap().push(id);
            }
            fn record_packet(&self, _: i32, _: &[u8]) {}
            fn status(&self) -> RecorderStatus {
                RecorderStatus {
                    active: true,
                    path: Default::default(),
                    elapsed: Duration::ZERO,
                    packet_count: self.0.lock().unwrap().len() as u64,
                    bytes_written: 0,
                    error: None,
                }
            }
            fn stop(self: Box<Self>) -> std::io::Result<std::path::PathBuf> {
                Ok(Default::default())
            }
        }

        with_client_game(|game| {
            let (_reader, mut writer) = connect_play_test(game, |_, _| {});
            spawn_menu_test(game);
            let recorded = Arc::new(Mutex::new(Vec::new()));
            game.remote
                .as_ref()
                .unwrap()
                .session
                .start_recording(RecordIds(recorded.clone()), |_| {});
            let delimiter = C::BundleDelimiter(sunrise_protocol::packets::Empty);
            game.selected = 0;
            for packet in [
                delimiter.clone(),
                C::SetHeldSlot(play::SetHeldSlot { slot: 1 }),
                C::DebugSample(play::DebugSample {
                    sample: vec![0; 5],
                    kind: play::DEBUG_SAMPLE_TICK_TIME,
                }),
                C::SetHeldSlot(play::SetHeldSlot { slot: 2 }),
                delimiter,
            ] {
                writer.send(&packet.encode()).unwrap();
            }
            let outcome =
                |event: &Event| matches!(event, Event::Bundle(_) | Event::Disconnected { .. });
            game.wait_for_test_remote_event(outcome);
            let backlog = &mut game.remote.as_mut().unwrap().backlog;
            let pos = backlog
                .iter()
                .position(|item| matches!(item, Received::Event(event, _) if outcome(event)))
                .unwrap();
            let Some(Received::Event(Event::Bundle(bundle), _credit)) = backlog.remove(pos) else {
                panic!("the receiver closed before handing the completed bundle to main");
            };
            assert_eq!(
                *recorded.lock().unwrap(),
                [
                    BUNDLE_DELIMITER,
                    SET_HELD_SLOT,
                    SET_HELD_SLOT,
                    BUNDLE_DELIMITER
                ],
                "only the real bundle members surrounding the fault reach the recorder",
            );
            use sunrise_client::net::BundleMember;
            assert!(matches!(
                bundle.packets.as_slice(),
                [
                    BundleMember::Packet(C::SetHeldSlot(_)),
                    BundleMember::OversizedDebugSample,
                    BundleMember::Packet(C::SetHeldSlot(_)),
                ]
            ));
            assert_eq!(game.selected, 0, "no member applied on the reader");
            let mut turn = ListenerTurn::default();
            game.handle_play_bundle(*bundle, &mut turn).unwrap();
            assert_eq!(
                game.selected, 1,
                "the prefix applied but the suffix did not"
            );
            assert!(
                matches!(turn.end, Some(Event::Disconnected { key: Some(key), from_server: false, .. }) if key == "disconnect.packetError")
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn executor_waits_for_entity_removal_behind_a_decoding_chunk() {
        with_client_game(|game| {
            let (_reader, _writer) = connect_play_test(game, |_, _| {});
            spawn_menu_test(game);
            let remote = game.remote.as_mut().unwrap();
            let format = remote.chunk_format.clone();
            let publish = remote.session.received_event_probe();
            publish(Event::Status(String::new()));
            let (_, credit) = remote
                .session
                .poll()
                .into_iter()
                .next()
                .unwrap()
                .into_parts();
            // Insert the leased decoder slot without supplying its result yet.
            assert!(matches!(remote.decoder.next(), Next::Decoding));
            remote.backlog.push_back(Received::Chunk(format, credit));
            publish(Event::Packet(Box::new(C::RemoveEntities(
                play::RemoveEntities { ids: vec![20] },
            ))));
            game.spawn_entity(20, "minecraft:pig", [2.0, 64.0, 0.0], [0.0; 3]);
            let entity = game.entity(20).unwrap();
            assert_eq!(entity.position(game), Some([2.0, 64.0, 0.0]));

            let (record, calls) = std::sync::mpsc::channel();
            let executor = game.client_tasks.executor("producer").unwrap();
            executor.execute(Box::new(move |host| {
                record.send(entity.position(host)).unwrap()
            }));

            game.poll_remote().unwrap();
            assert!(matches!(
                calls.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ));
            assert_eq!(entity.position(game), Some([2.0, 64.0, 0.0]));

            // Retire the synthetic chunk slot without relying on worker timing.
            assert!(matches!(
                game.remote.as_mut().unwrap().backlog.pop_front(),
                Some(Received::Chunk(..))
            ));
            game.poll_remote_before_tick().unwrap();
            assert_eq!(calls.try_recv(), Ok(None));
        });
    }
}
