//! Main-thread actions of `ClientConfigurationPacketListenerImpl`.
use super::{
    CommonIo, Dynamic, Empty, Event, QueuedWriter, Reason, cfg, ingress, select_known_packs,
    switch_client_protocol,
};
use crate::raw_payloads::{PlayTransition, Transport};
use std::sync::{Arc, Mutex, atomic::AtomicBool, mpsc};
use sunrise_protocol::{payload_negotiation::Phase, retained::OwnedHeap};

#[derive(Default)]
pub(super) struct Configuration {
    registries: Mutex<Registries>,
}

#[derive(Default)]
struct Registries {
    data: Dynamic,
    dialog_snapshot: Option<Arc<Dynamic>>,
    refusal: Option<String>,
}

pub(super) enum Action {
    Ping(cfg::Ping),
    SelectKnownPacks(cfg::SelectKnownPacks),
    RegistryData(cfg::RegistryData),
    ConsumeEffectTypes(cfg::RegistryData),
    UpdateTags(cfg::UpdateTags),
    ShowDialog(cfg::ShowDialog),
}

impl Configuration {
    pub(super) fn queue(
        self: &Arc<Self>,
        io: &Arc<CommonIo>,
        send: &ingress::Producer<Event>,
        action: Action,
    ) -> Result<(), Reason> {
        send.send_wait(Event::Configuration(Box::new(ConfigurationTask {
            listener: self.clone(),
            io: io.clone(),
            action,
        })))
        .map_err(|error| Reason::plain(error.to_string()))
    }

    fn take_registries(&self) -> Result<Dynamic, String> {
        let mut registries = self.registries.lock().unwrap();
        if let Some(refusal) = registries.refusal.take() {
            return Err(refusal);
        }
        Ok(std::mem::take(&mut registries.data))
    }
}

pub struct ConfigurationTask {
    listener: Arc<Configuration>,
    io: Arc<CommonIo>,
    action: Action,
}

impl ConfigurationTask {
    pub(super) fn owned_bytes(&self) -> usize {
        size_of::<Self>()
            + match &self.action {
                Action::Ping(_) => 0,
                Action::SelectKnownPacks(p) => p.owned_heap_bytes(),
                Action::RegistryData(p) | Action::ConsumeEffectTypes(p) => p.owned_heap_bytes(),
                Action::UpdateTags(p) => p.owned_heap_bytes(),
                Action::ShowDialog(p) => p.owned_heap_bytes(),
            }
    }

    pub fn handle(self) -> Option<Event> {
        if !self.io.accepts_packets() {
            return None;
        }
        let reply = match self.action {
            Action::Ping(p) => cfg::Serverbound::Pong(p),
            Action::SelectKnownPacks(p) => {
                cfg::Serverbound::SelectKnownPacks(cfg::SelectKnownPacks {
                    packs: select_known_packs(p.packs),
                })
            }
            Action::RegistryData(p) => {
                let mut registries = self.listener.registries.lock().unwrap();
                if p.registry == "minecraft:dialog" {
                    registries.dialog_snapshot = None;
                }
                registries.data.add(p);
                return None;
            }
            Action::ConsumeEffectTypes(p) => {
                let mut registries = self.listener.registries.lock().unwrap();
                if let Err(error) = registries.data.add_consume_effects(p) {
                    // FinishConfiguration refuses in order, without dropping earlier replies.
                    registries.refusal.get_or_insert_with(|| {
                        format!("{}: {error}", crate::content_identity::MISMATCH)
                    });
                }
                return None;
            }
            Action::UpdateTags(p) => {
                let mut registries = self.listener.registries.lock().unwrap();
                if p.registries
                    .iter()
                    .any(|row| row.registry == "minecraft:dialog")
                {
                    registries.dialog_snapshot = None;
                }
                registries.data.add_tags(&p);
                return None;
            }
            Action::ShowDialog(p) => {
                let mut registries = self.listener.registries.lock().unwrap();
                let Registries {
                    data,
                    dialog_snapshot,
                    ..
                } = &mut *registries;
                let snapshot = dialog_snapshot.get_or_insert_with(|| {
                    // Copy only the dynamic dialog registry and its tags, so
                    // later RegistryData and UpdateTags cannot change its links.
                    let mut snapshot = Dynamic::default();
                    let key = "minecraft:dialog";
                    if let Some(entries) = data.registries.get(key) {
                        snapshot.registries.insert(key.into(), entries.clone());
                    }
                    if let Some(tags) = data.tags.get(key) {
                        snapshot.tags.insert(key.into(), tags.clone());
                    }
                    Arc::new(snapshot)
                });
                return Some(Event::ShowDialog {
                    dialog: p.dialog,
                    registries: snapshot.clone(),
                });
            }
        };
        if let Some(writer) = self.io.writer.lock().unwrap().as_ref() {
            let _ = writer.send(&reply.encode());
        }
        None
    }
}

pub(super) enum PlayPreparation {
    Ready(PlayTransition),
    // A peer without an Offer still needs its configuration setup frozen, but
    // has no negotiated callbacks to wait for on the reader.
    WithoutOffer(Arc<Transport>),
}

/// `ClientConfigurationPacketListenerImpl.handleConfigurationFinished`:
/// finish the registries and datapack state after earlier main callbacks, then
/// publish Play. An ordered refusal leaves those earlier replies on the wire.
pub struct FinishConfiguration {
    pub(super) listener: Arc<Configuration>,
    pub(super) writer: Arc<Mutex<Option<QueuedWriter>>>,
    pub(super) configuring: Arc<AtomicBool>,
    pub(super) play: Result<PlayPreparation, String>,
    pub(super) content_verified: bool,
    pub(super) menu_extra_revision: u8,
    pub(super) resume: mpsc::Sender<Result<Dynamic, Reason>>,
}

impl FinishConfiguration {
    pub(super) fn owned_bytes(&self) -> usize {
        size_of::<Self>() + self.play.as_ref().err().map_or(0, String::capacity)
    }

    pub fn acknowledge(self) {
        let result = self.play.and_then(|play| {
            let mut registries = self.listener.take_registries()?;
            crate::datapack_net::finish_transfer()?;
            registries.custom_components = self.content_verified;
            registries.menu_extra_revision = self.menu_extra_revision;
            let publish = || {
                switch_client_protocol(
                    &self.writer,
                    &self.configuring,
                    Phase::Play,
                    &cfg::Serverbound::FinishConfiguration(Empty).encode(),
                )
                .map_err(|e| e.to_string())
            };
            match play {
                PlayPreparation::Ready(prepared) => prepared.commit(publish)?,
                PlayPreparation::WithoutOffer(payloads) => {
                    payloads.accept_empty_and_play(publish)?
                }
            }
            Ok(registries)
        });
        if result.is_err()
            && let Some(writer) = self.writer.lock().unwrap().as_ref()
        {
            writer.close_after_pending();
        }
        let _ = self.resume.send(result.map_err(Reason::plain));
    }
}
