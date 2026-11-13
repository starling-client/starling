//! A chemicals library: tanks that hold one chemical, and the per-tick transfer between
//! neighbours that a network runs for every pair of tanks, its hottest function. Every eligible
//! function here gets a mod point when the crate builds; addons patch it through them.
pub mod tanks;

/// A chemical: an id and how fast it flows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, mod_api::Wire)]
pub struct Chemical {
    pub id: u32,
    pub viscosity: u32,
}

pub use tanks::Tank;

/// Moves up to `rate` units of `from`'s chemical into `to`, slowed by its viscosity; returns
/// the amount moved. A tank holding another chemical takes none.
pub fn transfer(from: &mut Tank, to: &mut Tank, rate: u32) -> u32 {
    if from.amount == 0 || (to.amount > 0 && to.chemical != from.chemical) {
        return 0;
    }
    let moved = flow(rate, from.chemical.viscosity)
        .min(from.amount)
        .min(to.capacity - to.amount);
    from.amount -= moved;
    to.amount += moved;
    to.chemical = from.chemical;
    moved
}

/// How much `rate` moves of a chemical this viscous: a private helper, a point all the same.
fn flow(rate: u32, viscosity: u32) -> u32 {
    rate / viscosity.max(1)
}

/// One network tick: each tank passes chemical to the next. Returns the total moved.
pub fn tick(tanks: &mut [Tank], rate: u32) -> u32 {
    let mut moved = 0;
    for i in 1..tanks.len() {
        let (left, right) = tanks.split_at_mut(i);
        moved += transfer(&mut left[i - 1], &mut right[0], rate);
    }
    moved
}

/// The library's own accounting, opted out: no addon may listen here.
#[mod_api::no_point]
pub fn audit(tanks: &[Tank]) -> u32 {
    tanks.iter().map(|tank| tank.amount).sum()
}
