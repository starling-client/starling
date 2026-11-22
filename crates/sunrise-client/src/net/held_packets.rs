//! Checked publication for the client actor's ordered hold queue.
use super::*;
impl Session {
    /// Send a play packet; silently dropped once the connection is gone,
    /// and while the server has the connection back in configuration (a
    /// proxy switching servers). Vanilla's `handleConfigurationStart` clears
    /// the level before it acknowledges, so no play packet follows the
    /// acknowledgement; one that did would reach the server as a
    /// configuration packet, and Velocity drops such a client.
    pub fn send(&self, packet: play::Serverbound) {
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
                return;
            }
        };
        let mut writer = self.writer.lock().unwrap();
        if self.configuring.load(Ordering::Relaxed) {
            return;
        }
        if let Some(w) = writer.as_mut()
            && w.send(&body).is_err()
        {
            w.shutdown();
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
    /// Publish an already encoded engine packet, retaining the normal writer's bounds.
    pub fn send_held_body(
        &self,
        body: Vec<u8>,
        incarnation: (u64, u64),
    ) -> std::result::Result<(), mod_api::SendPacketError> {
        if self.serial() != incarnation.0 {
            return Err(mod_api::SendPacketError::NotConnected);
        }
        let writer = self.writer.lock().unwrap();
        let writer = writer
            .as_ref()
            .ok_or(mod_api::SendPacketError::NotConnected)?;
        writer
            .prepare_packet(body, Some(incarnation.1))?
            .try_publish()
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
                .send(&play::Serverbound::KeepAlive(cfg::KeepAlive { id }).encode())
                .unwrap();
        })
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
