//! API and area slots and the internals source callback share one unified manifest.
mod area;
pub use area::area;
mod count;
pub use count::count;
#[derive(Default)]
struct Check;

pub fn tick(_: mod_api::HostSide<'_>, _: &mut mod_points::TickArgs) {}

#[mod_api::horizon_mod]
impl mod_api::Hooks for Check {}
