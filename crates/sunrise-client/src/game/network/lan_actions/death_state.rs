use crate::game::{Game, api_values::TryIntoApi};
use mod_api::HurtSource;
use std::sync::Arc;
use sunrise_client::{health::AcceptedHit, items::ItemStack};

/// The source a player's death hooks hear (`living_death`, `living_drops`,
/// `living_death_post`): the accepted hit retained on the body, else the
/// death's own source, `generic_kill` for `/kill`.
pub(in crate::game) struct FatalSource {
    accepted: Option<Arc<AcceptedHit>>,
    pub(in crate::game) damage_type: String,
    pub(in crate::game) causing: u32,
}

impl FatalSource {
    pub(in crate::game) fn source(&self) -> HurtSource<'_> {
        self.accepted.as_ref().map_or(
            HurtSource {
                damage_type: &self.damage_type,
                causing: self.causing as i32,
                direct: self.causing as i32,
                position: None,
                direct_type: None,
            },
            |hit| hit.source(),
        )
    }

    fn weapon(&self) -> Option<&ItemStack> {
        self.accepted.as_ref().and_then(|hit| hit.weapon.as_ref())
    }
}

impl Game<'_> {
    /// The fatal source of a guest's death, read before `living_death` and
    /// kept for the drops and the death's tail.
    pub(in crate::game) fn guest_fatal_source(&self, gi: usize, conn: u32) -> FatalSource {
        let (damage_type, causing) = self.guest_damage_source(conn);
        let guest = self.lan.as_ref().and_then(|host| host.hub.guests.get(gi));
        let body = if self.lan_acting == Some(conn) {
            self.player.as_ref()
        } else {
            guest.and_then(|guest| guest.motion.as_deref())
        };
        let accepted = body
            .and_then(|body| body.health.accepted_hit.as_ref())
            .or(guest.and_then(|guest| guest.carried_hit.as_ref()))
            .filter(|hit| {
                damage_type != "minecraft:generic_kill" || hit.source().damage_type == damage_type
            })
            .cloned();
        FatalSource {
            accepted,
            damage_type,
            causing,
        }
    }

    /// A veto belongs to the guest's living body, not its replaceable combat history.
    pub(in crate::game) fn guest_death_canceled(
        &mut self,
        gi: usize,
        conn: u32,
        fatal: Option<&FatalSource>,
    ) -> bool {
        if self
            .lan
            .as_ref()
            .is_some_and(|host| host.hub.guests[gi].death_canceled)
        {
            return true;
        }
        let Some(fatal) =
            fatal.filter(|_| mod_api::__private::listened(mod_api::HookId::LivingDeath))
        else {
            return false;
        };
        let Some(guest) = self.lan.as_ref().and_then(|host| host.hub.guests.get(gi)) else {
            return false;
        };
        let entity = guest.entity_id as u32;
        let acting = self.lan_acting == Some(conn);
        let source = fatal.source();
        let weapon = fatal.weapon().and_then(|stack| stack.try_into_api().ok());
        let mut cancel = false;
        self.native_living_death(
            entity,
            "minecraft:player",
            &source,
            weapon.as_ref(),
            &mut cancel,
        );
        if cancel {
            let acting_health = if acting {
                self.player.as_mut().map(|body| {
                    body.health.dead = false;
                    body.health.health()
                })
            } else {
                None
            };
            if let Some(host) = self.lan.as_mut() {
                host.hub.guests.update(gi, |guest| {
                    guest.death_canceled = acting_health.unwrap_or(guest.health) <= 0.0;
                    if !guest.death_canceled {
                        guest.death_time = 0;
                    }
                    if let Some(body) = guest.motion.as_mut() {
                        body.health.dead = false;
                    }
                });
            }
        }
        cancel
    }
}
