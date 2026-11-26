//! Dropped-item environmental damage, without a game or a server.
use std::{fs, path::Path, sync::Arc};
use sunrise_client::{
    assets::Assets, blocks::BlockFacts, chunk::Chunk, entities::ItemEntity, world::World,
};
use sunrise_core::geometry::Vec3;

pub fn main() -> sunrise_client::Result<()> {
    let args: Vec<_> = crate::args().collect();
    let root = Path::new(args.get(1).ok_or("missing resources")?);
    let assets = Assets::load(root)?;
    let facts = BlockFacts::load(root, assets.registry)?;
    let defaults = sunrise_world::item_defaults::read(root)?;
    let damage = sunrise_client::health::HealthResources::load(root)?;
    let items = sunrise_client::items::Items::load(
        root,
        assets.registry,
        &facts,
        &sunrise_world::loot::PackOverlay::NONE,
        &defaults,
    )?;
    let can_hurt = |stack: &sunrise_client::items::ItemStack, name: &str| {
        let default = defaults
            .entries()
            .get(stack.item.as_ref())
            .and_then(|components| components.get("minecraft:damage_resistant"));
        sunrise_client::item_damage::stack_can_be_hurt_by(default, &damage.damage, stack, name)
            .unwrap()
    };
    let mut world = World::new(facts.air, 1);
    world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let mut output = String::new();
    for line in fs::read_to_string(args.get(2).ok_or("missing inputs")?)?.lines() {
        let f: Vec<_> = line.split('\t').collect();
        if f.len() != 6 {
            return Err("expected six tab-separated columns".into());
        }
        let position = Vec3::new(f[2].parse()?, f[3].parse()?, f[4].parse()?);
        let block = if f[5] == "empty" {
            "minecraft:cauldron"
        } else {
            "minecraft:lava_cauldron"
        };
        let secondary = match f[5] {
            "pair" => "minecraft:lava_cauldron",
            "mixed" => "minecraft:lava",
            _ => "minecraft:air",
        };
        world.set(
            1,
            0,
            0,
            assets
                .registry
                .default_state(secondary)
                .ok_or("missing secondary block")?,
        )?;
        world.set(
            0,
            0,
            0,
            assets
                .registry
                .default_state(block)
                .ok_or("missing cauldron")?,
        )?;
        let mut item = ItemEntity {
            size_answer: None,
            attachment_lifetime: Default::default(),
            fall_distance: 0.0,
            random: sunrise_core::random::LegacyRandom::new(0),
            rotation: [0.0; 2],
            join: Default::default(),
            health: 5,
            fire_ticks: 0,
            id: 1,
            stack: sunrise_client::item_data::parse_stack(f[1])?.ok_or("empty fixture stack")?,
            thrower: None,
            target: None,
            position,
            previous: position,
            velocity: Vec3::ZERO,
            ground: Default::default(),
            age: 0,
            stack_revision: 0,
            lifespan: ItemEntity::LIFETIME,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 0,
            needs_sync: false,
            stuck: Vec3::ZERO,
            was_touching_water: false,
            no_gravity: true,
            vertical_collision: false,
        };
        output.push_str(f[0]);
        for _ in 0..2 {
            if item.health > 0 {
                item.environment_tick(&world, &facts, &items, assets.registry, &can_hurt);
            }
            output.push_str(&format!(
                "\t{},{},{}",
                item.health,
                item.fire_ticks,
                item.health <= 0
            ));
        }
        output.push('\n');
    }
    fs::write(args.get(3).ok_or("missing output")?, output)?;
    Ok(())
}
