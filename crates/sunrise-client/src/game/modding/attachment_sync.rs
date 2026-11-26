//! Attachment sync (docs/modding/src/how-to/native-mods.md, "Server attachments"): the server
//! sends the synced values of the holders a level pass lent to mods, and whole
//! snapshots to the players who started seeing a holder; a client keeps them.
//!
//! The channel is an engine declaration in the negotiated payload table
//! (`raw_payloads::engine`): a message goes only to a player whose connection offered it
//! and who sees the holder. It is prepared against the current epoch and published at
//! the root commit, in order with the player's other packets.
use super::Game;
use crate::lan::entity::{project_attachment_sync, report_attachment_sync_refusal};
use crate::lan::{conn::PlayerSender, hub::AttachmentSnapshot};
use mod_api::{PayloadBytes, PayloadId, PayloadSendError};
use sunrise_client::raw_payloads::Endpoint;
use sunrise_mod_runtime::attachments::{Attachments, Holder, attachment_sync_watermark};
use sunrise_protocol::attachment_sync::{self, Message};

impl Game<'_> {
    /// The messages that give a player `holder`'s synced attachments whole: its
    /// own, and for a chunk those of its block entities. A player holder with
    /// none still gives one, to drop what the client holds.
    fn attachment_snapshot(&mut self, holder: Holder) -> Vec<Message> {
        let changes = match holder {
            // Read in place: a chunk lent to a mod is copied and saved.
            Holder::Chunk([x, z]) => self
                .world
                .chunks
                .get(&(x, z))
                .and_then(|chunk| chunk.saved.as_ref())
                .map(|extras| extras.attachments.synced(project_attachment_sync(holder))),
            // Read in place: lending the map would give the entity one.
            Holder::Entity(id) => self
                .entity_attachments
                .get(&id)
                .map(|map| map.synced(project_attachment_sync(holder))),
            _ => self
                .attachment_map_read(holder)
                .ok()
                .flatten()
                .map(|map| map.synced(project_attachment_sync(holder))),
        }
        .unwrap_or_default();
        let mut holders = vec![(holder, changes)];
        if let Holder::Chunk([x, z]) = holder {
            holders.extend(
                self.world
                    .block_entities
                    .chunk_attachments(x, z)
                    .into_iter()
                    .map(|((x, y, z), map)| {
                        let holder = Holder::BlockEntity([x, y, z]);
                        (holder, map.synced(project_attachment_sync(holder)))
                    }),
            );
        }
        holders
            .into_iter()
            .filter(|(holder, changes)| !changes.is_empty() || matches!(holder, Holder::Player(_)))
            .map(|(holder, changes)| Message {
                holder,
                replace: true,
                changes,
            })
            .collect()
    }

    /// The holders players started seeing are sent whole to them, in the pass
    /// of the level they are in (another's stay queued), then the block
    /// entities a piston landed whole to the players who see them, then the
    /// accepted updates since the last pass to the players who see those holders.
    pub(super) fn flush_attachment_sync(&mut self) {
        let landings = std::mem::take(&mut self.world.attachment_landings);
        let snapshots = self
            .lan
            .as_mut()
            .map(|host| std::mem::take(&mut host.hub.attachment_snapshots))
            .unwrap_or_default();
        // Colliders: the changes first, to those who already track the entity; the players who
        // just started to are sent the whole collider below instead.
        let fresh = Self::fresh_collider_pairs(
            &snapshots
                .iter()
                .map(|s| (s.player, s.holder))
                .collect::<Vec<_>>(),
        );
        if self.lan.is_some() {
            self.flush_collider_sync(&fresh);
        }
        let mut delta_cutoffs = std::collections::HashMap::new();
        for snapshot in snapshots {
            let AttachmentSnapshot {
                player,
                holder,
                paired_sync_from,
            } = snapshot;
            let collider_entity = match holder {
                Holder::Entity(id) => Some(id),
                _ => None,
            };
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            let hub = &mut host.hub;
            let Some(guest) = hub.online_players().find(|g| g.uuid == player) else {
                continue;
            };
            // Each channel is its own: a peer with only the collider channel still gets the
            // collider.
            let reads = guest.negotiated_attachment_sync();
            let reads_collider = collider_entity.is_some() && guest.negotiated_collider_sync();
            if !reads && !reads_collider {
                continue;
            }
            if guest.dimension != hub.config.dimension {
                hub.attachment_snapshots.push(snapshot);
                continue;
            }
            // Both non-player entities' (`Holder::Entity`) and tracked players'
            // (`Holder::Player`) initial non-empty synced attachments were already sent
            // atomically inside `ServerEntity::pairing`'s `BundleDelimiter` bundle; keep
            // `Holder::Entity` snapshots here for collider sync only. The recipient's own
            // `Holder::Player(uuid)` from `send_level_info` still sends with `replace: true`
            // even when empty, clearing stale replica fields on join or respawn.
            if reads {
                if let Some((paired, from)) = paired_sync_from {
                    delta_cutoffs.insert((player, paired), from);
                } else if !matches!(holder, Holder::Entity(_)) {
                    delta_cutoffs.insert((player, holder), attachment_sync_watermark());
                }
            }
            let messages = if reads && !matches!(holder, Holder::Entity(_)) {
                self.attachment_snapshot(holder)
            } else {
                Vec::new()
            };
            let sender = self
                .lan
                .as_ref()
                .and_then(|host| host.hub.guests.iter().find(|g| g.uuid == player))
                .map(|guest| guest.sender.clone());
            if let Some(sender) = sender {
                for message in &messages {
                    self.send_attachment_sync(std::slice::from_ref(&sender), message);
                }
                if let (true, Some(id)) = (reads_collider, collider_entity) {
                    self.send_collider_snapshot(&sender, id);
                }
            }
        }
        // A landed block entity is a new holder: what its clients held of it
        // went with their moving piston's copy.
        for holder in landings {
            let senders = self
                .lan
                .as_ref()
                .map_or_else(Vec::new, |host| host.hub.attachment_recipients(holder));
            if senders.is_empty() {
                continue;
            }
            for message in self.attachment_snapshot(holder) {
                self.send_attachment_sync(&senders, &message);
            }
        }
        let mut recipients = Vec::new();
        let mut ordered = Vec::new();
        for holder in std::mem::take(&mut self.world.attachment_syncs) {
            if !self
                .attachment_map_read(holder)
                .ok()
                .flatten()
                .is_some_and(Attachments::has_pending_sync)
            {
                continue;
            }
            let senders: Vec<_> = self
                .lan
                .as_ref()
                .into_iter()
                .flat_map(|host| host.hub.attachment_recipient_rows(holder))
                .map(|guest| (guest.uuid, guest.sender.clone()))
                .collect();
            let Ok(map) = self.attachment_map_quiet(holder, mod_api::AttachmentAccess::Existing)
            else {
                continue;
            };
            if senders.is_empty() {
                map.discard_sync_changes();
                continue;
            }
            if !matches!(holder, Holder::BlockEntity(_)) {
                let index = recipients.len();
                recipients.push((holder, senders));
                ordered.extend(
                    map.take_ordered_sync()
                        .into_iter()
                        .map(|change| (index, change)),
                );
                continue;
            }
            let changes = map.take_sync_changes(project_attachment_sync(holder));
            if changes.is_empty() {
                continue;
            }
            let message = Message {
                holder,
                replace: false,
                changes,
            };
            self.send_attachment_sync(senders.iter().map(|(_, sender)| sender), &message);
        }
        // Do not merge unrelated writes into one frame: project and send each
        // accepted update in its original order, even across different holders.
        ordered.sort_unstable_by_key(|(_, change)| change.order());
        for (index, pending) in ordered {
            let (holder, senders) = &recipients[index];
            let order = pending.order();
            let mut senders = senders
                .iter()
                .filter(|(player, _)| {
                    delta_cutoffs
                        .get(&(*player, *holder))
                        .is_none_or(|from| order >= *from)
                })
                .map(|(_, sender)| sender)
                .peekable();
            if senders.peek().is_none() {
                continue;
            }
            let Some(change) = pending.project(project_attachment_sync(*holder)) else {
                continue;
            };
            self.send_attachment_sync(
                senders,
                &Message {
                    holder: *holder,
                    replace: false,
                    changes: vec![change],
                },
            );
        }
    }

    /// Queue `message` for each of `senders` through the negotiated transport. A peer
    /// that has the channel absent, or is between epochs, is sent nothing and that is no
    /// error. Every other refusal is named in the log: a message
    /// past the connection's frame limits or one that cannot be written is not sent and the
    /// player stays joined; a full outgoing queue closes the connection, as a full queue
    /// did before, since the replica would miss the update.
    fn send_attachment_sync<'sender>(
        &mut self,
        senders: impl IntoIterator<Item = &'sender PlayerSender>,
        message: &Message,
    ) {
        let mut senders = senders.into_iter().peekable();
        if senders.peek().is_none() {
            return;
        }
        let body = match attachment_sync::encode_retaining(message, |change, error| {
            report_attachment_sync_refusal(
                message.holder,
                format_args!(
                    "{}:{}: {error}",
                    change.namespace.to_string_lossy(),
                    change.name.to_string_lossy()
                ),
            );
        }) {
            Ok(Some(body)) => Ok(body),
            Ok(None) => return,
            Err(error) => Err(error.to_string()),
        };
        for sender in senders {
            let Err(error) = self.queue_attachment_sync(sender, &body) else {
                continue;
            };
            if !refused(&error) {
                continue;
            }
            log::warn!(
                "Sunrise: attachment sync for {:?} not sent: {error:?}",
                message.holder
            );
            if matches!(error, PayloadSendError::Backpressure) {
                sender.close();
            }
        }
    }

    /// Queue one encoded message (or its encoding failure) for one connection: the
    /// transport's typed answer, `FrameTooLarge` past the connection's frame limits.
    fn queue_attachment_sync(
        &mut self,
        sender: &PlayerSender,
        body: &Result<Vec<u8>, String>,
    ) -> Result<(), PayloadSendError> {
        let body = body
            .as_ref()
            .map_err(|reason| PayloadSendError::EncodeFailed(reason.clone()))?;
        self.queue_current_payload(
            Endpoint::server(sender.clone()),
            &PayloadId(attachment_sync::CHANNEL.into()),
            PayloadBytes(body.clone()),
        )
        .map(|_| ())
    }
}

/// Whether a send error is a refusal to name, not a peer without the channel or a
/// connection between epochs.
fn refused(error: &PayloadSendError) -> bool {
    !matches!(
        error,
        PayloadSendError::UnsupportedPeer(_)
            | PayloadSendError::NotNegotiated
            | PayloadSendError::WrongProtocol(_)
            | PayloadSendError::StaleEpoch
            | PayloadSendError::Disconnected
    )
}

impl Game<'_> {
    /// Apply an already decoded sync in play packet order, when its holder should exist.
    pub(super) fn remote_attachment_sync(&mut self, message: Message) {
        let Some(remote) = self.remote.as_ref() else {
            return;
        };
        let foreign_player = match message.holder {
            Holder::Entity(id) => {
                let id_i32 = id as i32;
                let known = id_i32 == remote.entity_id
                    || remote.tracked(id_i32).is_some()
                    || remote.tracked_entity(id_i32).is_some()
                    || self.level_dragon_part(id).is_some();
                if !known {
                    log::warn!("Sunrise: received attachment sync for unknown entity {id}");
                    return;
                }
                None
            }
            Holder::Player(uuid) if uuid != remote.profile() => {
                let Some(id) = remote.player_entity(uuid) else {
                    log::warn!("Sunrise: received attachment sync for unknown player {uuid:032x}");
                    return;
                };
                Some(id)
            }
            // The recipient's own initial sync does not come from an entity pairing.
            _ => None,
        };
        let remote = self.remote.as_mut().unwrap();
        let map = remote.attachments.entry(message.holder).or_default();
        if message.replace {
            map.replace_contents(Attachments::default());
        }
        for change in message.changes {
            map.apply_sync(change);
        }
        if let Holder::Entity(id) = message.holder {
            remote.general.attachments_arrived(id as i32, map);
        }
        self.bind_remote_attachment_with_player(message.holder, foreign_player);
    }

    /// Publish a synced copy once its scoped client holder can be named.
    pub(super) fn bind_remote_attachment(&mut self, holder: Holder) {
        self.bind_remote_attachment_with_player(holder, None);
    }

    fn bind_remote_attachment_with_player(&mut self, holder: Holder, player: Option<u32>) {
        use mod_api::{AttachmentHolder as H, ChunkRef, EntityRef};
        let Some(remote) = &self.remote else { return };
        let Some(level) = self.level(&remote.level) else {
            return;
        };
        let scoped = match holder {
            Holder::Entity(id) => H::Entity(EntityRef::new(level.epoch(), id)),
            Holder::Player(uuid) => {
                let id = if uuid == remote.profile() {
                    u32::try_from(remote.entity_id).ok()
                } else {
                    player.or_else(|| remote.player_entity(uuid))
                };
                let Some(id) = id else { return };
                H::Player(mod_api::__private::game::player_ref(
                    EntityRef::new(level.epoch(), id),
                    uuid,
                ))
            }
            Holder::Chunk(pos) => H::Chunk(ChunkRef::new(level, pos)),
            Holder::BlockEntity(pos) => {
                let Some(entity) = self.block_entity(level, pos) else {
                    return;
                };
                H::BlockEntity(entity)
            }
            Holder::Level => H::Level(level),
            _ => return,
        };
        if let Some(map) = self
            .remote
            .as_mut()
            .and_then(|r| r.attachments.get_mut(&holder))
            && !map.has_index_binding()
        {
            map.bind_index(self.attachment_indexes.entry(self.dim).or_default(), scoped);
        }
    }
}

#[cfg(test)]
#[path = "attachment_sync/general_snapshot_tests.rs"]
mod general_snapshot_tests;
#[cfg(test)]
#[path = "attachment_sync/route_cases_tests.rs"]
mod route_cases_tests;
#[cfg(test)]
#[path = "attachment_sync/transport_tests.rs"]
mod transport_tests;
