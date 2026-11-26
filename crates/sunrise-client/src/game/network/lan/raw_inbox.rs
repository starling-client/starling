//! Reborrow live world state after a turn that ran outside the Hub.
use super::*;
use crate::game::Dim;

impl Game<'_> {
    pub(super) fn resume_raw_inbox(
        &mut self,
        hub: &mut Hub,
        entities: &[sunrise_client::lan::entity::EntityState],
        spawn: [f64; 3],
    ) -> Vec<HostEvent> {
        // Raw transactions finished above. Reborrow current world state only
        // now; no callback captures a TickCtx or an as_guest world swap.
        let clock_rate = self.network_clock_rate();
        let mut ctx = TickCtx {
            sea_level: self.end.locator().map_or(63, |terrain| terrain.sea_level()),
            world: &mut self.world,
            entities,
            next_entity: &mut self.next_entity,
            spawn,
            spawn_angle: self.level.world_spawn_angle,
            spawn_dimension: crate::game::Dim::of_world_spawn(&self.level).id(),
            difficulty: i32::from(self.ui.difficulty),
            difficulty_locked: self.level.difficulty_locked,
            clock_rate,
            tick_rate: self.clock.rate(),
            tick_frozen: self.clock.frozen(),
            tick_steps: self.clock.step_state(),
            collision: Some(self.collision),
            border: &self.border,
            streaming: Some(&self.streaming),
        };
        hub.drain_packets(&mut ctx)
    }
}

impl Game<'_> {
    pub(super) fn drain_raw_turns(
        &mut self,
        mut host: LanHost,
        mut events: Vec<HostEvent>,
        entities: &[sunrise_client::lan::entity::EntityState],
        spawn: [f64; 3],
    ) -> Option<LanHost> {
        let source = self.dim;
        let mut other_entities = States::default();
        let mut active_spawn = spawn;
        loop {
            let active_entities = if self.dim == source {
                entities
            } else {
                other_entities.as_slice()
            };
            let back = sunrise_client::spans::time("lan_events", || {
                self.lan_process_events(host, events, active_entities, active_spawn)
            });
            let Some(back) = back else {
                if self.dim != source
                    && let Err(error) = self.enter_level(source)
                {
                    log::error!("could not reopen {}: {error}", source.id());
                }
                return None;
            };
            host = back;

            let next = host
                .hub
                .packet_resume_dimension()
                .map_or(source, Dim::from_id);
            if next != self.dim {
                if self.dim != source {
                    host.states = std::mem::take(&mut other_entities);
                }
                // The teleport has already opened its destination. Resume only the
                // travelling connection there, with that level's world and solids.
                self.lan = Some(host);
                if let Err(error) = self.enter_level(next) {
                    log::error!("could not reopen {}: {error}", next.id());
                    return None;
                }
                host = self.lan.take()?;
                active_spawn = if next == source {
                    spawn
                } else {
                    other_entities = std::mem::take(&mut host.states);
                    self.world_spawn_position()
                };
                let mut solids = std::mem::take(&mut host.hub.move_solids);
                self.fill_move_solids(&mut solids, !host.hub.guests.is_empty());
                host.hub.move_solids = solids;
            }
            if !host.hub.packets_paused() {
                return Some(host);
            }
            let active_entities = if self.dim == source {
                entities
            } else {
                other_entities.as_slice()
            };
            events = self.resume_raw_inbox(&mut host.hub, active_entities, active_spawn);
        }
    }
}
