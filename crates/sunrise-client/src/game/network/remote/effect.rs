use super::*;
use sunrise_client::effects::EffectInstance;

/// `LivingEntity.forceAddEffect` with the packet's `shouldBlend` (flag 8): the effect replaces
/// any of its kind, an update keeping its fade (`copyBlendState`). A new one not blended starts
/// at its target (`skipBlending`), except the kinds in `beside`, whose fade the caller keeps
/// beside the list. Returns the instance when it is new, for those fades.
fn force_add_effect<'a>(
    list: &'a mut Vec<EffectInstance>,
    mut e: EffectInstance,
    blend: bool,
    beside: &[&str],
) -> Option<&'a EffectInstance> {
    match list.iter().position(|x| x.id == e.id) {
        Some(i) => {
            e.client_blend = list[i].client_blend;
            list[i] = e;
            None
        }
        None => {
            if !blend && !beside.contains(&e.id) {
                let has_effect = !e.ends_within(0);
                e.client_blend = (has_effect, has_effect);
            }
            list.push(e);
            list.last()
        }
    }
}

impl Game<'_> {
    /// `ClientPacketListener.handleUpdateMobEffect` (`LivingEntity.forceAddEffect`): for the
    /// player itself, or another living entity the level holds (a mob, another player or an
    /// armor stand). Any other entity, or an effect the registry doesn't name, changes nothing.
    pub(super) fn remote_effect(&mut self, p: &play::UpdateMobEffect) {
        let Some(def) = self.remote_effect_def(p.effect) else {
            return;
        };
        let mut e = EffectInstance::from_def(def, p.duration, p.amplifier);
        e.ambient = p.flags & 1 != 0;
        e.visible = p.flags & 2 != 0;
        e.show_icon = p.flags & 4 != 0;
        let blend = p.flags & 8 != 0;
        let darkness_added =
            |e: &EffectInstance| super::super::sculk::darkness_blend_added(e, blend);
        if self.remote.as_ref().is_some_and(|r| r.entity_id == p.id) {
            let list = &mut self.potions.effects.list;
            match force_add_effect(list, e, blend, &["darkness", "nausea"]) {
                Some(e) if e.id == "darkness" => self.sculk.darkness = darkness_added(e),
                Some(e) if e.id == "nausea" => {
                    let (o_nausea, nausea) = screen_effects::nausea_blend_added(e, blend);
                    self.screen_fx.o_nausea = o_nausea;
                    self.screen_fx.nausea = nausea;
                }
                _ => {}
            }
            return;
        }
        let id = p.id as u32;
        let (list, darkness) = if let Some(m) = self.mob_index.get_mut(&mut self.mobs, id) {
            (
                &mut m.extra.client_effects.list,
                &mut m.extra.darkness_blend,
            )
        } else if let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id) {
            (&mut a.client_effects.list, &mut a.darkness_blend)
        } else {
            return;
        };
        if let Some(e) = force_add_effect(list, e, blend, &["darkness"])
            && e.id == "darkness"
        {
            *darkness = darkness_added(e);
        }
    }
}
