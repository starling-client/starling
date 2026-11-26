//! Shulker bullets for tools/test_shulker_bullet_parity.py: origin, stand-on checks, and 26.3 tick rotation.
#![forbid(unsafe_code)]
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
    sync::Arc,
};
use sunrise_client::{
    Result, collisions,
    mobs::{Mob, MobTypes, Who, shulker},
    projectiles::{Hit, Projectile, ProjectileKind, ProjectileWorld, Turn},
    world::World,
};
use sunrise_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::LegacyRandom,
};
use sunrise_world::{chunk::BlockState, registry::BlockStateId};

/// The bullet's first move only asks which neighbours are empty: all are.
struct Open(Vec<(Who, Aabb)>);
impl ProjectileWorld for Open {
    fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
        None
    }
    fn in_water(&self, _: Aabb) -> bool {
        false
    }
    fn targets(&self) -> &[(Who, Aabb)] {
        &self.0
    }
}

fn vector(v: &Value) -> Vec3 {
    let n = |i: usize| v[i].as_f64().unwrap();
    Vec3::new(n(0), n(1), n(2))
}

fn hex_f64(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}

fn hex_f32(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}

fn breeze_box(at: Vec3) -> Aabb {
    let half = f64::from(0.6_f32 / 2.0_f32);
    let height = f64::from(1.77_f32);
    Aabb::new(
        Vec3::new(at.x - half, at.y, at.z - half),
        Vec3::new(at.x + half, at.y + height, at.z + half),
    )
}

/// `"minecraft:oak_slab[type=top]"` as a registry state.
fn parse_state(text: &str) -> BlockState {
    let (name, properties) = text.split_once('[').unwrap_or((text, ""));
    BlockState {
        name: name.into(),
        properties: properties
            .trim_end_matches(']')
            .split(',')
            .filter(|p| !p.is_empty())
            .map(|p| {
                let (key, value) = p.split_once('=').unwrap();
                (key.to_string(), value.to_string())
            })
            .collect(),
    }
}

pub fn main() -> Result<()> {
    let args: Vec<_> = crate::args().collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/sunrise-client")
        .join(sunrise_core::VANILLA_VERSION);
    let registry = sunrise_world::level::data::load_registry(&root)?;
    let binding = collisions::load(&root, &registry)?;
    let types = MobTypes::load(&root)?;
    let kind = Arc::new(
        types
            .types
            .get("minecraft:shulker")
            .ok_or("no shulker")?
            .clone(),
    );
    let mut out = BufWriter::new(File::create(&args[2])?);
    for line in BufReader::new(File::open(&args[1])?).lines() {
        let f: Value = serde_json::from_str(&line?)?;
        if f["mode"] == "origin" {
            let mut m = Mob::new(1, kind.clone(), vector(&f["position"]), 0.0);
            m.extra.variant3 = f["face"].as_u64().unwrap() as u8;
            m.anim.v[shulker::PEEK] = f32::from_bits(f["peek"].as_u64().unwrap() as u32);
            m.extra.model_scale = f["scale"].as_f64().unwrap() as f32;
            let target = Aabb::new(Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 1.0, 0.5))
                .move_by(vector(&f["target"]));
            let axis = match m.extra.variant3 {
                0 | 1 => 1,
                2 | 3 => 2,
                _ => 0,
            };
            let world = Open(vec![(Who::Mob(2), target)]);
            let at = shulker::bullet_origin(&m);
            let p = Projectile::shulker_bullet(&world, at, Who::Mob(1), Who::Mob(2), axis, 1);
            let b = p.position;
            writeln!(
                out,
                "{:016x} {:016x} {:016x}",
                b.x.to_bits(),
                b.y.to_bits(),
                b.z.to_bits()
            )?;
        } else if f["mode"] == "turn" {
            let mut p = Projectile::new(
                ProjectileKind::ShulkerBullet {
                    target: None,
                    dir: None,
                    steps: 0,
                    delta: [0.0; 3],
                    seed: 0,
                },
                Vec3::new(0.5, 110.0, 0.5),
                None,
            );
            p.no_gravity = f["no_gravity"].as_bool().unwrap();
            p.velocity = Vec3::new(
                hex_f64(&f["velocity"][0]),
                hex_f64(&f["velocity"][1]),
                hex_f64(&f["velocity"][2]),
            );
            p.yaw = hex_f32(&f["yaw"]);
            p.pitch = hex_f32(&f["pitch"]);
            p.set_old_pos_and_rot();
            let mut targets = Vec::new();
            if let Some(seed) = f["deflect_seed"].as_i64() {
                p = p.with_entity_random(LegacyRandom::new(seed));
                let at = Vec3::new(
                    hex_f64(&f["breeze"][0]),
                    hex_f64(&f["breeze"][1]),
                    hex_f64(&f["breeze"][2]),
                );
                targets.push((Who::Mob(99), breeze_box(at)));
            }
            let world = Open(targets);
            let ticks = f["ticks"].as_u64().unwrap();
            for tick in 0..ticks {
                let (_, hit) = p.tick(&world);
                if matches!(hit, Hit::Entity { .. }) {
                    assert!(p.deflect(Turn::Reverse, [1.0; 3], (None, None, None), false));
                    p.turn_bullet();
                }
                if tick > 0 {
                    write!(out, ",")?;
                }
                write!(
                    out,
                    "{:08x}:{:08x}:{:08x}:{:08x}",
                    p.yaw.to_bits(),
                    p.pitch.to_bits(),
                    p.previous_yaw.to_bits(),
                    p.previous_pitch.to_bits()
                )?;
            }
            writeln!(out)?;
        } else {
            let pos = &f["pos"];
            let pos = BlockPos::new(
                pos[0].as_i64().unwrap() as i32,
                pos[1].as_i64().unwrap() as i32,
                pos[2].as_i64().unwrap() as i32,
            );
            let mut world = World::new(BlockStateId(0), 0);
            let chunk = (pos.x >> 4, pos.z >> 4);
            world.chunks.insert(
                chunk,
                Arc::new(sunrise_client::chunk::Chunk::empty(chunk.0, chunk.1)),
            );
            let state = registry.resolve(&parse_state(f["block"].as_str().unwrap()))?;
            world.set(pos.x, pos.y, pos.z, state)?;
            let y = f["y"].as_f64().unwrap();
            writeln!(
                out,
                "{}",
                collisions::projectile_can_stand_on(&world, &binding, pos, y)
            )?;
        }
    }
    Ok(())
}
