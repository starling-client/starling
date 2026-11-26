pub use world::inside_write::{
    EntityWrite, InsideEffect, InsideEffects, InsideEntityAccess, InsideEntityView,
};

mod __private {
    pub use crate::world::inside_write::BuiltinInsideAction;
}
