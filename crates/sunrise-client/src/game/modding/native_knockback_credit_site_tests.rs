//! Pushes that no melee path starts still carry their owner as `living_knockback`'s kill
//! credit: owned evoker fangs, a harming splash potion and a harming area cloud, each heard by
//! a real listener, with the credit it received and the motion it left.
use super::native_knockback_tests::{
    assert_sword_credit_reached_the_push, credit_pig, heard, sword_evoker,
};
use super::potions::PotionOwner;
use sunrise_client::items::ItemStack;
use sunrise_client::mobs::Who;
use sunrise_core::geometry::Vec3;

fn credited_push(hit: impl FnOnce(&mut super::Game<'_>)) {
    let mut pushed = Vec3::ZERO;
    let heard = heard(|game, log| {
        log.lock().unwrap().reduce_sword_credit = true;
        credit_pig(game);
        sword_evoker(game);
        hit(game);
        pushed = game.mobs[0].velocity;
    });
    assert_sword_credit_reached_the_push(&heard, pushed);
}

#[test]
#[ignore = "daily"]
fn owned_fangs_credit_their_evoker_and_its_sword() {
    credited_push(|game| {
        // Bites on this tick, at the pig's feet.
        game.insert_fangs(900, [0.0, 0.0, 0.0], 0.0, -7, 50);
        game.tick_fangs();
    });
}

#[test]
#[ignore = "daily"]
fn a_harming_splash_potion_credits_its_thrower_and_its_sword() {
    credited_push(|game| {
        let mut potion = ItemStack::new("minecraft:splash_potion", 1);
        potion.data_mut().potion = Some("minecraft:harming".into());
        let mut owner = PotionOwner::default();
        owner.attacker = Some(50);
        game.splash(
            &potion,
            Vec3::new(0.0, 0.0, 0.0),
            None,
            None,
            0,
            &owner,
            true,
            Vec3::ZERO,
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_harming_area_cloud_credits_its_owner_and_its_sword() {
    credited_push(|game| {
        let text = sunrise_client::text::Text::translate("entity.minecraft.area_effect_cloud", &[]);
        game.cloud_affect(
            super::potions::Victim::Mob(100),
            &[("instant_damage", 1, 0)],
            (&[], &[], 1.0),
            [0.0, 0.0, 0.0],
            &text,
            (Some(Who::Mob(50)), None),
        );
    });
}
