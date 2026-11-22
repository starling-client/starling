/// The `living_travel` hook's mode (#2414), as the fixture's stand-in for
/// the mod API's own `TravelMode`: the branch the travel is about to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TravelMode {
    Air,
    Water,
    Lava,
    Glide,
    Flying,
    VelocityOnly,
}
