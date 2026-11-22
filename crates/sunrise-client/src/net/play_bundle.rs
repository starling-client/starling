//! PacketBundlePacker assembles on receive; ClientPacketListener handles the
//! completed bundle in one ordinary main-thread task.
use super::{Cookie, Event, PlayIo, PlayIoAction, Reason, brand_event};
use std::sync::Arc;
use sunrise_protocol::{
    attachment_sync,
    packets::{custom_payload::CustomPayload, play::Clientbound as C},
};

/// `BundlerInfo.BUNDLE_SIZE_LIMIT` counts decoded members, not delimiters or bytes.
const BUNDLE_SIZE_LIMIT: usize = 4096;

pub enum BundleMember {
    Packet(C),
    AttachmentSync(Box<attachment_sync::StagedMessage>),
    /// Defer the logger fault, not the unusable array, until the bundle handles this member.
    OversizedDebugSample,
}

impl BundleMember {
    fn owned_heap_bytes(&self) -> usize {
        match self {
            Self::Packet(packet) => packet.owned_heap_bytes(),
            Self::AttachmentSync(staged) => std::mem::size_of::<attachment_sync::StagedMessage>()
                .saturating_add(staged.owned_heap_bytes()),
            Self::OversizedDebugSample => 0,
        }
    }
}

#[derive(Default)]
pub(super) struct PlayBundle(Option<Vec<BundleMember>>);

pub(super) enum PlayTurn {
    Single(BundleMember),
    Bundle(Vec<BundleMember>),
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum InvalidBundle {
    Terminal,
    TooManyPackets,
}

impl PlayBundle {
    pub(super) fn assembling(&self) -> bool {
        self.0.is_some()
    }

    pub(super) fn accept(
        &mut self,
        member: BundleMember,
    ) -> Result<Option<PlayTurn>, InvalidBundle> {
        if matches!(member, BundleMember::Packet(C::BundleDelimiter(_))) {
            return Ok(match self.0.take() {
                Some(packets) => Some(PlayTurn::Bundle(packets)),
                None => {
                    self.0 = Some(Vec::new());
                    None
                }
            });
        }
        let Some(packets) = &mut self.0 else {
            return Ok(Some(PlayTurn::Single(member)));
        };
        if matches!(member, BundleMember::Packet(C::StartConfiguration(_))) {
            return Err(InvalidBundle::Terminal);
        }
        // `BundlerInfo.addPacket` checks before retaining the next member.
        if packets.len() >= BUNDLE_SIZE_LIMIT {
            return Err(InvalidBundle::TooManyPackets);
        }
        packets.push(member);
        Ok(None)
    }
}

pub struct CompletedBundle {
    pub packets: Vec<BundleMember>,
    pub io: Arc<PlayIo>,
}

impl CompletedBundle {
    pub(super) fn owned_bytes(&self) -> usize {
        self.packets
            .capacity()
            .saturating_mul(std::mem::size_of::<BundleMember>())
            .saturating_add(
                self.packets
                    .iter()
                    .map(BundleMember::owned_heap_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(std::mem::size_of::<Self>())
    }
}

pub enum BundleStep {
    Event(Event),
    Payload(CustomPayload),
    Transfer(sunrise_protocol::packets::configuration::Transfer),
    Close(Event),
    HandlerFailed,
    Continue,
}

impl PlayIo {
    /// Vanilla checks whether the listener can handle the outer packet. It
    /// does not re-check eligibility between handlers once the loop starts.
    pub fn accepts_bundle(&self) -> bool {
        self.common.accepts_packets()
    }

    /// `ClientCommonPacketListenerImpl.onPacketError`, after a main-thread handler fails.
    pub fn packet_error(&self) -> Event {
        Reason::keyed("disconnect.packetError", &self.lang).ended(false)
    }

    pub fn incompatible_payload(&self, detail: &str) -> Event {
        Reason::keyed_arg("multiplayer.disconnect.incompatible", &self.lang, detail).ended(false)
    }

    pub fn bundle_member(&self, member: BundleMember) -> BundleStep {
        let packet = match member {
            BundleMember::Packet(packet) => packet,
            BundleMember::AttachmentSync(staged) => {
                return BundleStep::Event(Event::AttachmentSync(staged));
            }
            BundleMember::OversizedDebugSample => return BundleStep::HandlerFailed,
        };
        let action = match packet {
            C::KeepAlive(p) => PlayIoAction::KeepAlive(p),
            C::ChunkBatchStart(_) => PlayIoAction::ChunkBatchStart,
            C::ChunkBatchFinished(p) => PlayIoAction::ChunkBatchFinished(p.size),
            packet if self.run_reader_diagnostic(&packet) => {
                return if mod_api::__private::listened(mod_api::HookId::ClientPacket) {
                    BundleStep::Event(Event::ObservePacket(Box::new(packet)))
                } else {
                    BundleStep::Continue
                };
            }
            C::CookieRequest(p) => {
                self.common.handle_cookie(Cookie::Request(p));
                return BundleStep::Continue;
            }
            C::StoreCookie(p) => {
                self.common.handle_cookie(Cookie::Store(p));
                return BundleStep::Continue;
            }
            C::ResourcePackPush(p) => return BundleStep::Event(Event::PackPush(p)),
            C::ResourcePackPop(p) => return BundleStep::Event(Event::PackPop(p.id)),
            C::ServerLinks(p) => return BundleStep::Event(Event::ServerLinks(p.links)),
            C::Transfer(p) => {
                self.common
                    .transferring
                    .store(true, std::sync::atomic::Ordering::Release);
                return BundleStep::Transfer(p);
            }
            C::Disconnect(p) => {
                return BundleStep::Close(Reason::tag(&p.reason, &self.lang).ended(true));
            }
            C::CustomPayload(p) if p.channel == "minecraft:brand" => {
                return brand_event(&p).map_or(BundleStep::Continue, BundleStep::Event);
            }
            C::CustomPayload(p) if p.channel == crate::lod_net::CHANNEL => {
                PlayIoAction::FarTile(p.data)
            }
            C::CustomPayload(payload) => return BundleStep::Payload(payload),
            other => return BundleStep::Event(Event::Packet(Box::new(other))),
        };
        self.handle(action);
        BundleStep::Continue
    }
}
