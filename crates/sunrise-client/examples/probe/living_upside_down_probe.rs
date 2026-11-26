//! The shared living render transform, for the upside-down oracle.
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use sunrise_client::mobs::{Mob, MobTypes, render};
use sunrise_core::geometry::Vec3;

pub(super) fn run(args: &[String]) -> sunrise_client::Result<()> {
    let types = MobTypes::load(Path::new(&args[0]))?;
    let mut out = String::new();
    for line in fs::read_to_string(&args[1])?.lines() {
        let fields: Vec<_> = line.split_ascii_whitespace().collect();
        let [species, age, scale, name] = fields.as_slice() else {
            return Err(format!("invalid living render case: {line}").into());
        };
        let kind = Arc::new(types.types[&format!("minecraft:{species}")].clone());
        let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
        mob.extra.age = if *age == "baby" { -24000 } else { 0 };
        mob.extra.model_scale = scale.parse()?;
        mob.custom_name = Some((*name).into());
        if *species == "magma_cube" {
            mob.extra.size = 3;
        }

        let pose = render::base_transform_at(&mob, 1.0, glam::Mat4::IDENTITY);
        writeln!(
            out,
            "{:08x} {:08x} {:08x} {:08x}",
            mob.height().to_bits(),
            mob.model_scale().to_bits(),
            pose.y_axis.y.to_bits(),
            pose.w_axis.y.to_bits(),
        )?;
    }
    fs::write(&args[2], out)?;
    Ok(())
}
