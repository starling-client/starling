//! Water left alone by a mod that took over its tick, and water that spreads as vanilla.
use super::*;

#[test]
fn water_spreads_on_a_floor() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let water = w.id("water", &[]);
    w.set(BlockPos::new(0, 1, 0), water);
    for _ in 0..60 {
        w.tick();
    }
    for d in 1..=7 {
        let pos = BlockPos::new(d, 1, 0);
        assert_eq!(
            w.prop(pos, "level"),
            d.to_string(),
            "flow at distance {d}: {}",
            w.name(pos)
        );
    }
    assert_eq!(w.get(BlockPos::new(8, 1, 0)), w.b().air());
    // Diagonals follow Manhattan distance.
    assert_eq!(w.prop(BlockPos::new(2, 1, 2), "level"), "4");
}

#[test]
fn refused_fluid_tick_leaves_the_water_alone() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let source = BlockPos::new(0, 1, 0);
    w.level.guarded.insert(source);
    let water = w.id("water", &[]);
    w.set(source, water);
    for _ in 0..60 {
        w.tick();
    }
    assert_eq!(w.get(source), water);
    assert_eq!(w.get(BlockPos::new(1, 1, 0)), w.b().air());
    // Allowed again, its tick is gone: nothing was left scheduled to run it.
    w.level.guarded.clear();
    for _ in 0..60 {
        w.tick();
    }
    assert_eq!(w.get(BlockPos::new(1, 1, 0)), w.b().air());
}

#[test]
fn refused_non_source_tick_keeps_exact_state_and_schedules_nothing() {
    let mut w = world!();
    let pos = BlockPos::new(0, 1, 0);
    let flowing = w.id("water", &[("level", "3")]);
    w.put(pos, flowing);
    w.level.guarded.insert(pos);
    assert_eq!(w.ticks.fluids.count(), 0);
    w.b().scheduled_fluid_tick(&mut w.level, &mut w.ticks, pos);
    assert_eq!(w.get(pos), flowing);
    assert_eq!(w.ticks.fluids.count(), 0);
    assert_eq!(w.ticks.blocks.count(), 0);
}

/// Water passes between two cells by their collision shapes at their positions: a cell whose
/// block entity publishes a closed shape takes no water from any side, and once the shape opens
/// and its neighbours are told, the water flows in.
#[test]
#[ignore = "daily"]
fn a_published_closed_shape_keeps_water_out_until_it_opens() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let closed = BlockPos::new(1, 1, 0);
    w.level.published.insert(closed, VoxelShape::block());
    w.set(BlockPos::new(0, 1, 0), w.id("water", &[]));
    for _ in 0..60 {
        w.tick();
    }
    assert_eq!(
        w.prop(BlockPos::new(0, 1, 1), "level"),
        "1",
        "the water spread around it"
    );
    assert_eq!(
        w.get(closed),
        w.b().air(),
        "nothing flowed into the closed cell"
    );
    // The shape opens: the block entity's change tells its neighbours, which re-evaluate.
    w.level.published.clear();
    let air = w.b().air();
    let block = w.b().block(air);
    w.b()
        .update_neighbors_at(&mut w.level, &mut w.ticks, closed, block);
    for _ in 0..60 {
        w.tick();
    }
    assert_ne!(w.get(closed), air, "water flowed in once the shape opened");
}

/// Two partial faces that together cover the wall keep water out; two that leave a gap let it
/// through. Each publish is a new shape, so a changed shape never reads an old answer.
#[test]
#[ignore = "daily"]
fn published_half_faces_close_the_wall_only_together() {
    use sunrise_core::geometry::{Aabb, Vec3};
    let half = |low: f64, high: f64| {
        VoxelShape::from_box(Aabb::new(
            Vec3::new(0.0, low, 0.0),
            Vec3::new(1.0, high, 1.0),
        ))
        .unwrap()
    };
    let mut w = world!();
    w.floor(0, 10, "stone");
    let (source, wall) = (BlockPos::new(0, 1, 0), BlockPos::new(1, 1, 0));
    // Closed on its other sides, so the water reaches the wall only through the source's face.
    for around in [(1, 1, 1), (1, 1, -1), (2, 1, 0)] {
        w.level.published.insert(
            BlockPos::new(around.0, around.1, around.2),
            VoxelShape::block(),
        );
    }
    w.level.published.insert(source, half(0.0, 0.5));
    w.level.published.insert(wall, half(0.5, 1.0));
    w.set(source, w.id("water", &[]));
    for _ in 0..60 {
        w.tick();
    }
    assert_eq!(
        w.get(wall),
        w.b().air(),
        "the halves meet and close the wall"
    );
    assert_eq!(w.prop(BlockPos::new(0, 1, 1), "level"), "1");
    // The wall's shape moves to the bottom half: the faces leave a gap.
    w.level.published.insert(wall, half(0.0, 0.5));
    let air = w.b().air();
    let block = w.b().block(air);
    w.b()
        .update_neighbors_at(&mut w.level, &mut w.ticks, wall, block);
    for _ in 0..60 {
        w.tick();
    }
    assert_ne!(w.get(wall), air, "water passes through the gap");
}
