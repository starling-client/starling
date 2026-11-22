//! Shared cauldron client selection. Prediction never invokes authoritative callbacks.
use super::remote::DryUse;
use super::*;
use sunrise_client::cauldron::{
    self, Outcome,
    interactions::{Dispatcher, Entry},
};
use sunrise_core::vanilla_bytes::Writer;
use sunrise_protocol::{
    buf::{Codec, Ident, Reader},
    packets::{configuration::CustomPayload, play::Clientbound},
};
use sunrise_world::level::InteractionResult;

pub(super) use sunrise_protocol::packets::custom_payload::{
    CAULDRON_ENTRIES_CHANNEL as ENTRIES_CHANNEL, CAULDRON_USE_CHANNEL as USE_CHANNEL,
};

/// Publish the server's selection snapshot only to peers that can suspend
/// and resume these clicks. Vanilla and other peers keep their vanilla packets.
pub(super) fn publish_registrations<'a>(
    table: &cauldron_interactions::Table,
    guests: impl IntoIterator<Item = &'a sunrise_client::lan::guest::Guest>,
) {
    let mut packet = None;
    for guest in guests {
        let transport = guest.sender.payload_transport();
        if sunrise_client::raw_payloads::negotiated(transport, ENTRIES_CHANNEL)
            && sunrise_client::raw_payloads::negotiated(transport, USE_CHANNEL)
        {
            guest.send(packet.get_or_insert_with(|| registration_packet(table)));
        }
    }
}

/// Only selection keys are synchronized; the server never exports or predicts
/// a callback. An empty snapshot also clears entries after a reload.
pub(super) fn registration_packet(table: &cauldron_interactions::Table) -> Clientbound {
    let mut body = Writer::new();
    table.for_each_mod_entry(|block, item, tag| {
        body.string(block);
        body.string(item);
        body.bool(tag);
    });
    Clientbound::CustomPayload(CustomPayload {
        channel: ENTRIES_CHANNEL.into(),
        data: body.data,
    })
}

pub(super) fn decode_registrations(data: &[u8]) -> sunrise_protocol::Result<Dispatcher<()>> {
    let mut table = Dispatcher::default();
    let mut input = Reader::new(data);
    while input.remaining() != 0 {
        let block = Ident::read(&mut input)?;
        let item = Ident::read(&mut input)?;
        let _ = if input.bool()? {
            table.register_tag(block, item, ())
        } else {
            table.register(block, item, ())
        };
    }
    Ok(table)
}

/// The two independent decisions made by the server's UseItemOn turn.
#[derive(Default)]
pub(super) struct UseOnOutcome {
    /// The client may run its local item-on-block phase when the block passed or was skipped.
    pub(super) block_fell_through: bool,
    /// Only a final Pass lets the client continue to air use or the other hand.
    pub(super) passed: bool,
}

pub(super) fn use_result(receipt: i64, outcome: UseOnOutcome) -> Clientbound {
    let mut body = Writer::new();
    body.i64(receipt);
    body.bool(outcome.block_fell_through);
    body.bool(outcome.passed);
    Clientbound::CustomPayload(CustomPayload {
        channel: USE_CHANNEL.into(),
        data: body.data,
    })
}

struct DryHand<'a> {
    creative: bool,
    hand: Option<ItemStack>,
    inventory: &'a [Option<ItemStack>],
}
impl cauldron::Player for DryHand<'_> {
    fn creative(&self) -> bool {
        self.creative
    }
    fn hand(&mut self) -> &mut Option<ItemStack> {
        &mut self.hand
    }
    fn add(&mut self, _: &mut ItemStack) -> bool {
        true
    }
    fn contains(&self, stack: &ItemStack) -> bool {
        self.inventory
            .iter()
            .flatten()
            .any(|have| have.same_item_same_components(stack))
    }
    fn drop_stack(&mut self, _: ItemStack) {}
}
impl Game<'_> {
    /// The server's registration snapshot, not the client's installed mods,
    /// determines whether the click needs an authoritative block result.
    pub(super) fn has_server_cauldron_handler(
        &self,
        pos: [i32; 3],
        held: Option<&ItemStack>,
    ) -> bool {
        let (Some(table), Some(held)) = (
            self.remote
                .as_ref()
                .and_then(|r| r.cauldron_entries.as_ref()),
            held,
        ) else {
            return false;
        };
        let block = self.block_name(pos);
        let items = self.items();
        matches!(
            table.select(
                &block,
                &held.item,
                cauldron::removes_dye(items, &held.item),
                |tag| { items.item_in_tag(tag, &held.item) }
            ),
            Some(Entry::Mod(()))
        )
    }

    /// Predict vanilla only. The remote click path suspends a server-registered
    /// handler before reaching either of these prediction routes.
    pub(super) fn client_cauldron_prediction(
        &self,
        pos: [i32; 3],
        held: Option<&ItemStack>,
    ) -> (Outcome, DryUse) {
        let pass = || (Outcome::Pass, DryUse::default());
        let Some(held) = held else {
            return pass();
        };
        if self.held_mapping(Mapping::Sneak) {
            return pass();
        }
        let Some((state, underwater)) = self.cauldron_at(pos) else {
            return pass();
        };
        let table = Dispatcher::vanilla();
        let removes_dye = cauldron::removes_dye(self.items(), &held.item);
        let items = self.items();
        let entry = table.select(&state.name, &held.item, removes_dye, |tag| {
            items.item_in_tag(tag, &held.item)
        });
        let mut hand = DryHand {
            creative: self.creative,
            hand: Some(held.clone()),
            inventory: &self.inventory,
        };
        let result = cauldron::interact_selected(&state, underwater, entry.as_ref(), &mut hand);
        let at = sunrise_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        let changes = result
            .block
            .and_then(|block| self.pack.assets.registry.resolve(&block).ok())
            .map(|block| (at, block))
            .into_iter()
            .collect();
        let sounds = result
            .sound
            .map(|sound| (at, Arc::from(sound), mod_api::SoundSource::Blocks, 1.0, 1.0))
            .into_iter()
            .collect();
        (result.outcome, (changes, sounds))
    }
    /// The visual prediction route uses the same decision as the item-use consumption guard.
    pub(super) fn remote_cauldron_reacts(
        &self,
        hit: &Hit,
        held: Option<&ItemStack>,
    ) -> Option<(InteractionResult, DryUse)> {
        let (outcome, changes) = self.client_cauldron_prediction(hit.pos, held);
        let answer = match outcome {
            Outcome::Pass => return None,
            Outcome::Consume => InteractionResult::Consume,
            Outcome::Success => InteractionResult::Success,
        };
        Some((answer, changes))
    }
}
