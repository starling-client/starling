use super::*;
use sunrise_client::mobs::{anim, render};

impl render::RenderDataSource for Game<'_> {
    fn mob_data(&self) -> &sunrise_client::mobs::MobData {
        self.res.mob_data()
    }

    fn equipment_layers(&self, mob: &Mob) -> Vec<render::Layer> {
        self.mob_equipment_layers(mob, super::equipment_piece::EquipmentPass::Draw)
    }

    fn equipment_inventory(&self, mob: &Mob) -> Vec<render::Layer> {
        self.mob_equipment_layers(mob, super::equipment_piece::EquipmentPass::TextureInventory)
    }

    fn humanoid_setup(&self) -> Option<&dyn anim::HumanoidSetup> {
        mod_api::__private::listened(mod_api::HookId::HumanoidSetupAnim)
            .then_some(self as &dyn anim::HumanoidSetup)
    }
}

impl anim::HumanoidSetup for Game<'_> {
    fn before_arm_pose(&self, mob: &Mob, state: &anim::State<'_>, poses: &mut anim::Poses) {
        use super::api_values::TryIntoApi;
        let stacks = [false, true].map(|off| {
            render::held_item_stack(&mob.extra, off).and_then(|stack| stack.try_into_api().ok())
        });
        let [main, off] = anim::humanoid_arm_poses(mob, mob.extra.held_arm_pose, state.swing);
        let humanoid = mod_api::HumanoidRenderState {
            main_hand: stacks[0].as_ref(),
            off_hand: stacks[1].as_ref(),
            main_arm_left: mob.main_arm_left(),
            is_using_item: mob.extra.using_item > 0,
            use_hand: (mob.extra.using_item > 0).then_some(if mob.extra.using_off_hand {
                Hand::Off
            } else {
                Hand::Main
            }),
            ticks_using_item: mob.extra.using_item,
            use_animation: mob.extra.use_animation,
            main_arm_pose: main.try_into_api().unwrap(),
            off_arm_pose: off.try_into_api().unwrap(),
        };
        self.setup_humanoid_parts(mob.id, &mob.kind.id, &humanoid, state.partial, poses);
    }
}

impl Game<'_> {
    /// Lends the live pose to listeners at the shared humanoid seam.
    pub(super) fn setup_humanoid_parts(
        &self,
        entity_id: u32,
        entity_type: &str,
        humanoid: &mod_api::HumanoidRenderState<'_>,
        partial_tick: f32,
        poses: &mut anim::Poses,
    ) {
        self.native_humanoid_setup_anim(mod_api::__private::HumanoidSetupAnimCall {
            entity_id,
            entity_type,
            humanoid,
            partial_tick,
            game_time: self.title.is_none().then_some(self.world.game_time),
            parts: poses,
        });
    }
}
