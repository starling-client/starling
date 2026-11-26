pub use world::fluid_interaction::{
    BlockSelector, FluidBecomes, FluidInteraction, FluidInteractionAt, FluidInteractionHandler,
    FluidNeighbour, PlacedState,
};
mod __private {
    pub use crate::world::fluid_interaction::RegisteredFluidInteraction;
}
