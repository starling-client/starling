//! Starling: the old mod messages, for the website's mods made for them (ayynoxs_map_pack,
//! elytratrails). A native mod's server half and its client half send each other bytes on
//! `mod_id:name` channels, all on one custom payload channel (`horizon_protocol::mod_net`), as
//! Horizon had them before the fork's declared payloads (payload_transport.rs), which a mod made
//! for the old API never declares. Messages wait here: what a guest sent reaches
//! `native_net_from_client` (native_compat.rs) when the host's tick runs the queue, and what a mod
//! sends a player leaves at the tick's end, from whichever hook sent it.
use super::Game;
use horizon_protocol::{
    mod_net::{self, Message},
    packets::{
        configuration::CustomPayload,
        play::{Clientbound as C, Serverbound as S},
    },
};
use std::collections::HashMap;

/// Messages a tick at most: from one player, to one player, and from the window to its server.
pub(super) const PER_TICK: u32 = 32;

/// The messages waiting, and what each player used of this tick's allowance.
#[derive(Default)]
pub(super) struct OldModNet {
    from_players: Vec<(u128, Message)>,
    from_counts: HashMap<u128, u32>,
    to_players: Vec<(u128, Vec<u8>)>,
    to_counts: HashMap<u128, u32>,
    to_server: u32,
}

/// A message's payload as the sends check it.
fn payload(channel: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    if !mod_net::channel_ok(channel) {
        return Err(format!("the channel {channel:?} is not mod_id:name"));
    }
    mod_net::encode(&Message {
        channel: channel.to_owned(),
        data: data.to_vec(),
    })
    .map_err(|e| e.to_string())
}

impl Game<'_> {
    /// A `horizon:mod_net` payload from guest `conn`: queued for `run_old_mod_net`. Past the caps
    /// it is dropped in silence.
    pub(super) fn guest_old_mod_net(&mut self, conn: u32, data: &[u8]) {
        let Some(player) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| g.uuid)
        else {
            return;
        };
        let count = self.old_mod_net.from_counts.entry(player).or_default();
        if *count >= PER_TICK {
            return;
        }
        *count += 1;
        if let Ok(message) = mod_net::decode(data) {
            self.old_mod_net.from_players.push((player, message));
        }
    }

    /// The waiting messages from players reach the mods, with the hub in the game.
    pub(super) fn run_old_mod_net(&mut self) {
        if self.old_mod_net.from_players.is_empty() {
            return;
        }
        for (player, m) in std::mem::take(&mut self.old_mod_net.from_players) {
            self.native_net_from_client(player, &m.channel, &m.data);
        }
    }

    /// What the mods sent players this tick leaves; the tick's allowances start over after the
    /// last level's pass.
    pub(super) fn flush_old_mod_net(&mut self) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        for (player, data) in std::mem::take(&mut self.old_mod_net.to_players) {
            if let Some(g) = host.hub.guests.iter().find(|g| {
                g.placed && g.participates_online() && g.uuid == player && g.brand == "horizon"
            }) {
                host.hub.send_to(
                    g.conn,
                    &C::CustomPayload(CustomPayload {
                        channel: mod_net::CHANNEL.into(),
                        data,
                    }),
                );
            }
        }
        // The allowance is a server tick's, not a level pass's.
        if self.level_pass.last {
            self.old_mod_net.from_counts.clear();
            self.old_mod_net.to_counts.clear();
        }
    }

    /// The window's own allowance starts over (once a client tick).
    pub(super) fn tick_old_mod_net(&mut self) {
        self.old_mod_net.to_server = 0;
    }

    /// A `horizon:mod_net` payload the server sent the window.
    pub(super) fn remote_old_mod_net(&mut self, data: &[u8]) {
        match mod_net::decode(data) {
            Ok(m) => self.native_net_from_server(&m.channel, &m.data),
            Err(e) => log::debug!("Horizon: a bad mod message: {e}"),
        }
    }

    /// Sends `data` on `channel` (`mod_id:name`, at most 30 000 bytes, at most 32 a player a
    /// tick) to `player`'s client, at the tick's end. Ok means accepted: a player who left or
    /// isn't on a Horizon client doesn't get it.
    pub(super) fn old_net_send_to_player(
        &mut self,
        player: u128,
        channel: &str,
        data: &[u8],
    ) -> Result<(), String> {
        if self.remote.is_some() {
            return Err("a client has no players to send to".into());
        }
        let payload = payload(channel, data)?;
        let count = self.old_mod_net.to_counts.entry(player).or_default();
        if *count >= PER_TICK {
            return Err(format!("past {PER_TICK} messages to this player this tick"));
        }
        *count += 1;
        self.old_mod_net.to_players.push((player, payload));
        Ok(())
    }

    /// The client half: sends `data` on `channel` to the server (the same caps, 32 a tick). An
    /// error when the server doesn't carry mod messages (a vanilla server), and until the player
    /// has joined: a server says it carries them during configuration, before it can take one.
    pub(super) fn old_net_send_to_server(
        &mut self,
        channel: &str,
        data: &[u8],
    ) -> Result<(), String> {
        let Some(remote) = self.remote.as_ref() else {
            return Err("the server carries no mod messages".into());
        };
        if !remote.joined || remote.reconfiguring.is_some() {
            return Err("the player has not joined yet".into());
        }
        if !remote.mod_net {
            return Err("the server carries no mod messages".into());
        }
        let payload = payload(channel, data)?;
        if self.old_mod_net.to_server >= PER_TICK {
            return Err(format!("past {PER_TICK} messages this tick"));
        }
        self.old_mod_net.to_server += 1;
        self.send(S::CustomPayload(CustomPayload {
            channel: mod_net::CHANNEL.into(),
            data: payload,
        }));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::payload;

    #[test]
    fn a_message_needs_a_mod_channel_and_fits_the_cap() {
        assert!(payload("elytratrails:style", &[1, 2]).is_ok());
        assert!(payload("horizon:mod_net", &[]).is_err());
        assert!(payload("no_name", &[]).is_err());
        assert!(payload("m:n", &vec![0; horizon_protocol::mod_net::MAX_DATA + 1]).is_err());
    }
}
