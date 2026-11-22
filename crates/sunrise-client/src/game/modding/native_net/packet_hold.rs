//! Actor-owned holds. Overflow and partial publication are isolated in Holds.
use super::Game;
use crate::game::remote::{PendingCauldron, Remote};
use mod_api::{HoldError, HoldId, SendPacketError, ServerboundPlayPacket};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use sunrise_core::vanilla_bytes::Writer;
use sunrise_protocol::packets::play::Serverbound as S;

const PACKETS: usize = 256;
const BYTES: usize = 1_048_576;
static NEXT: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub(in crate::game) struct HeldPlay {
    body: Vec<u8>,
    cauldron: Option<(i64, PendingCauldron)>,
}

impl AsRef<[u8]> for HeldPlay {
    fn as_ref(&self) -> &[u8] {
        &self.body
    }
}

impl HeldPlay {
    fn publish(self, remote: &Remote, incarnation: (u64, u64)) -> Result<(), SendPacketError> {
        remote.session.send_held_body(self.body, incarnation)?;
        if let Some((receipt, click)) = self.cauldron {
            remote.pending_cauldrons.borrow_mut().insert(receipt, click);
        }
        Ok(())
    }
}

#[derive(Default)]
pub(in crate::game) struct Holds<T = HeldPlay> {
    incarnation: Option<(u64, u64)>,
    active: Option<(HoldId, Vec<T>, usize)>,
    overflowed: HashSet<HoldId>,
}
impl Holds<HeldPlay> {
    pub(in crate::game) fn clear_cauldron_continuations(&mut self) {
        if let Some((_, packets, _)) = &mut self.active {
            for packet in packets {
                packet.cauldron = None;
            }
        }
    }
}
impl<T: AsRef<[u8]> + Default> Holds<T> {
    pub(in crate::game) fn clear(&mut self) {
        *self = Self::default();
    }
    fn sync(&mut self, incarnation: Option<(u64, u64)>) {
        if self.incarnation != incarnation {
            self.clear();
            self.incarnation = incarnation;
        }
    }
    fn start(&mut self) -> Result<HoldId, HoldError> {
        if self.incarnation.is_none() {
            return Err(HoldError::NoWorld);
        }
        if self.active.is_some() {
            return Err(Self::limit());
        }
        let id = HoldId::new(NEXT.fetch_add(1, Ordering::Relaxed));
        self.active = Some((id, Vec::new(), 0));
        Ok(id)
    }
    fn take(&mut self, id: HoldId) -> Result<Vec<T>, HoldError> {
        if self.overflowed.remove(&id) {
            return Err(Self::limit());
        }
        if self.active.as_ref().is_none_or(|a| a.0 != id) {
            return Err(HoldError::Unknown);
        }
        Ok(self.active.take().unwrap().1)
    }
    fn limit() -> HoldError {
        HoldError::Limit {
            packets: PACKETS,
            bytes: BYTES,
        }
    }
    // Overflow policy: end and flush, then report Limit on the id once.
    fn queue(
        &mut self,
        packet: T,
        send: impl FnMut(T) -> Result<(), SendPacketError>,
    ) -> Result<usize, HoldError> {
        let Some((id, packets, bytes)) = self.active.as_mut() else {
            return Self::publish([packet], send);
        };
        let len = packet.as_ref().len();
        if packets.len() < PACKETS && len <= BYTES.saturating_sub(*bytes) {
            *bytes += len;
            packets.push(packet);
            return Ok(0);
        }
        self.overflowed.insert(*id);
        let mut packets = self.active.take().unwrap().1;
        packets.push(packet);
        let result = Self::publish(packets, send);
        if matches!(
            &result,
            Err(HoldError::Send {
                error: SendPacketError::NotInPlay | SendPacketError::NotConnected,
                ..
            })
        ) {
            self.clear();
        }
        result
    }
    // Send refusal policy: discard the tail and report the published prefix.
    fn publish(
        packets: impl IntoIterator<Item = T>,
        mut send: impl FnMut(T) -> Result<(), SendPacketError>,
    ) -> Result<usize, HoldError> {
        packets.into_iter().try_fold(0, |sent, packet| {
            send(packet).map_err(|error| HoldError::Send { sent, error })?;
            Ok(sent + 1)
        })
    }
}

impl Game<'_> {
    fn hold_incarnation(&self) -> Option<(u64, u64)> {
        self.remote
            .as_ref()
            .filter(|r| r.joined && r.reconfiguring.is_none())?
            .session
            .hold_incarnation()
    }
    pub(in crate::game) fn hold_packets(&mut self) -> Result<HoldId, HoldError> {
        let mut holds = self.native_net.holds.borrow_mut();
        holds.sync(self.hold_incarnation());
        holds.start()
    }
    pub(in crate::game) fn release_held(&mut self, hold: HoldId) -> Result<usize, HoldError> {
        let (incarnation, packets) = {
            let mut holds = self.native_net.holds.borrow_mut();
            holds.sync(self.hold_incarnation());
            let packets = holds.take(hold)?;
            (holds.incarnation.ok_or(HoldError::Unknown)?, packets)
        };
        let remote = self.remote.as_ref().ok_or(HoldError::Unknown)?;
        Holds::publish(packets, |packet| packet.publish(remote, incarnation))
    }
    pub(in crate::game) fn drop_held(&mut self, hold: HoldId) -> Result<(), HoldError> {
        let mut holds = self.native_net.holds.borrow_mut();
        holds.sync(self.hold_incarnation());
        holds.take(hold).map(|_| ())
    }
    /// The logical play packet already passed the client send hook. A cauldron
    /// continuation travels with its envelope; only a published packet expects a reply.
    pub(in crate::game) fn send_or_hold_play(
        &self,
        packet: S,
        cauldron: Option<(i64, PendingCauldron)>,
    ) {
        let Some(remote) = &self.remote else {
            return;
        };
        let mut holds = self.native_net.holds.borrow_mut();
        holds.sync(self.hold_incarnation());
        let tracked = cauldron.is_some();
        if holds.active.is_none() && !tracked {
            remote.session.send(packet);
            return;
        }
        let Some(incarnation) = holds.incarnation else {
            return;
        };
        match packet.try_encode() {
            Ok(body) => {
                let result = holds.queue(HeldPlay { body, cauldron }, |packet| {
                    packet.publish(remote, incarnation)
                });
                if tracked && let Err(error) = result {
                    log::warn!("cauldron use request was not published: {error:?}");
                }
            }
            Err(error) if tracked => log::warn!("cauldron use request encoding failed: {error}"),
            Err(_) => {}
        }
    }
    pub(in crate::game) fn publish_mod_held(
        &self,
        prepared: sunrise_client::net::PreparedPacket,
        packet: ServerboundPlayPacket,
    ) {
        if !prepared.is_current() {
            return;
        }
        let mut holds = self.native_net.holds.borrow_mut();
        holds.sync(self.hold_incarnation());
        if holds.active.is_none() {
            prepared.publish();
            return;
        }
        // The envelope has already passed the packet service's validation and reservation.
        // Drop its reservation before applying the hold's own bounded storage.
        drop(prepared);
        let Some(remote) = &self.remote else {
            return;
        };
        let id = sunrise_protocol::ids::play::serverbound::NAMES
            .iter()
            .position(|id| *id == packet.id.path())
            .unwrap();
        let mut body = Writer::new();
        body.varint(id as i32);
        body.bytes(&packet.body);
        let incarnation = holds.incarnation.expect("active hold has an incarnation");
        let packet = HeldPlay {
            body: body.data,
            cauldron: None,
        };
        let _ = holds.queue(packet, |packet| packet.publish(remote, incarnation));
    }
}
impl Game<'_> {
    /// `Connection.send` for the game's play packets. True if there is a remote
    /// and `client_packet_send` did not cancel; publication can still be held or refused.
    pub(in crate::game) fn send_play(&self, packet: S) -> bool {
        let queued = self.remote.is_some() && !self.client_packet_send_canceled(&packet);
        if queued {
            self.send_or_hold_play(packet, None);
        }
        queued
    }
}

#[cfg(test)]
#[path = "packet_hold_tests.rs"]
mod tests;
