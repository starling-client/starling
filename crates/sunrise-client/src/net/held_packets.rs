//! Checked publication for the client actor's ordered hold queue.
use super::*;
impl Session {
    /// Send a play packet; silently dropped once the connection is gone,
    /// and while the server has the connection back in configuration (a
    /// proxy switching servers). Vanilla's `handleConfigurationStart` clears
    /// the level before it acknowledges, so no play packet follows the
    /// acknowledgement; one that did would reach the server as a
    /// configuration packet, and Velocity drops such a client.
    pub fn send(&self, packet: play::Serverbound) -> bool {
        if self.debug
            && !matches!(
                packet,
                play::Serverbound::ClientTickEnd(_)
                    | play::Serverbound::MovePlayerPos(_)
                    | play::Serverbound::MovePlayerPosRot(_)
            )
        {
            eprintln!("[net] -> {packet:?}");
        }
        let body = match packet.try_encode() {
            Ok(body) => body,
            Err(error) => {
                eprintln!("[net] send refused: {error}");
                return false;
            }
        };
        let mut writer = self.writer.lock().unwrap();
        if self.configuring.load(Ordering::Relaxed) {
            return false;
        }
        let Some(w) = writer.as_mut() else {
            return false;
        };
        match w.send_play(&body) {
            Ok(()) => true,
            // Held packets alone fill the credit: this send is refused, the writer stays.
            Err(SendFailure::Held) => false,
            Err(SendFailure::Failed(_)) => {
                w.shutdown();
                false
            }
        }
    }

    /// Physical connection and play incarnation; changes even after Play/CFG/Play.
    pub fn hold_incarnation(&self) -> Option<(u64, u64)> {
        let writer = self.writer.lock().unwrap();
        writer
            .as_ref()?
            .play_incarnation()
            .map(|generation| (self.serial(), generation))
    }
    /// Reserve an engine send against the same physical writer as other play traffic.
    pub fn prepare_held_play(
        &self,
        packet: play::Serverbound,
        generation: u64,
    ) -> std::result::Result<PreparedPacket, mod_api::SendPacketError> {
        use mod_api::SendPacketError;
        let body = packet
            .try_encode()
            .map_err(|error| SendPacketError::EncodeFailed {
                message: error.to_string(),
            })?;
        let writer = self.writer.lock().unwrap();
        let writer = writer.as_ref().ok_or(SendPacketError::NotConnected)?;
        match writer.prepare_packet(body, self.serial(), Some(generation)) {
            Ok(prepared) => Ok(prepared),
            // Held packets alone fill the credit: refuse this send, keep the writer and holds.
            Err(PrepareRefusal::Held) => Err(SendPacketError::Backpressure),
            Err(PrepareRefusal::Failed(error)) => {
                if matches!(
                    error,
                    SendPacketError::Backpressure
                        | SendPacketError::FrameTooLarge { .. }
                        | SendPacketError::EncodeFailed { .. }
                ) {
                    // The same terminal writer-error policy as Session::send for engine
                    // packets (backpressure and transport framing failures).
                    writer.shutdown();
                }
                Err(error)
            }
        }
    }
}

#[cfg(feature = "test-payload-probe")]
impl Session {
    /// Physical play transport for actor tests, plus the network thread's direct reply path.
    #[doc(hidden)]
    pub fn packet_hold_probe() -> (Self, Connection, impl Fn(i64)) {
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let (_, physical) = Connection::local(client).split();
        let writer = QueuedWriter::new(physical).unwrap();
        writer
            .transition_protocol(sunrise_protocol::payload_negotiation::Phase::Play, &[3])
            .unwrap();
        let mut peer = Connection::local(server);
        peer.reader
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        assert_eq!(peer.recv().unwrap(), (3, vec![]));
        let session = Self::replay("hold probe");
        *session.writer.lock().unwrap() = Some(writer.clone());
        (session, peer, move |id| {
            writer
                .send_bypassing_held(&play::Serverbound::KeepAlive(cfg::KeepAlive { id }).encode())
                .unwrap();
        })
    }
}

#[cfg(feature = "test-payload-probe")]
impl Session {
    /// A live play receive loop on a memory pipe: the real network-thread path, with the server's end returned.
    #[doc(hidden)]
    pub fn play_loop_probe() -> (Self, Connection) {
        Self::play_loop_probe_with_registries(Arc::default())
    }

    /// [`Self::play_loop_probe`] with explicit synchronized registries for packet decode.
    #[doc(hidden)]
    pub fn play_loop_probe_with_registries(registries: Arc<Dynamic>) -> (Self, Connection) {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let (mut reader, physical) = Connection::local(client).split();
        let writer = QueuedWriter::new(physical).unwrap();
        writer
            .transition_protocol(sunrise_protocol::payload_negotiation::Phase::Play, &[3])
            .unwrap();
        let mut peer = Connection::local(server);
        peer.reader
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        assert_eq!(peer.recv().unwrap(), (3, vec![]));
        let mut session = Self::replay("play loop probe");
        *session.writer.lock().unwrap() = Some(writer);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("sunrise-play-loop-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let inputs = sunrise_world::native_load_context::LoadInputs::new(
            Default::default(),
            root.join("settings"),
        );
        let data = sunrise_world::item_defaults::ItemData::load(
            &root,
            sunrise_world::native_load_context::LoadContext::new(
                sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
                Arc::new(inputs),
                [],
            ),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        let (shared, configuring, send) = (
            session.writer.clone(),
            session.configuring.clone(),
            session.payload_events.clone(),
        );
        let (far, recorder, cookies, pings) = (
            session.far.clone(),
            session.recorder.clone(),
            session.cookies.clone(),
            session.pings.clone(),
        );
        session.thread = Some(std::thread::spawn(move || {
            let _ = receive_play(
                &mut reader,
                registries,
                &shared,
                &configuring,
                &Arc::new(Language::new()),
                &send,
                &cookies,
                &far,
                &data,
                &recorder,
                &crate::raw_payloads::Transport::empty(),
                &pings,
                &Arc::default(),
            );
        }));
        (session, peer)
    }
}

#[cfg(feature = "test-payload-probe")]
impl Session {
    /// Complete a protocol round trip for deterministic hold race tests.
    #[doc(hidden)]
    pub fn packet_hold_rollover(&self) {
        use sunrise_protocol::payload_negotiation::Phase;
        let writer = self.writer.lock().unwrap();
        let writer = writer.as_ref().unwrap();
        writer
            .transition_protocol(Phase::Configuration, &[3])
            .unwrap();
        writer.transition_protocol(Phase::Play, &[3]).unwrap();
    }
}
