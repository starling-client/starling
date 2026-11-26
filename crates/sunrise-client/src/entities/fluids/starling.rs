//! Starling's part of fluids.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// `FluidState.getFlow` at `p`: the way the water or lava there flows (nothing when still or
/// none). Create's water wheels turn by it.
pub fn flow_vector(world: &World, fluids: &FluidResources, p: [i32; 3]) -> Vec3 {
    struct Scene<'a> {
        world: &'a World,
        fluids: &'a FluidResources,
    }
    impl FluidFlowWorld for Scene<'_> {
        type Error = Error;
        type Block = BlockStateId;
        fn fluid(&mut self, p: BlockPos) -> Result<FluidState, Error> {
            Ok(self.fluids.fluid(self.world.block(p.x, p.y, p.z)))
        }
        fn block(&mut self, p: BlockPos) -> Result<BlockStateId, Error> {
            Ok(self.world.block(p.x, p.y, p.z))
        }
        fn blocks_fluid_flow(&self, s: BlockStateId) -> bool {
            self.fluids.blocks_flow(s)
        }
        fn is_ice_class(&self, s: BlockStateId) -> bool {
            self.fluids.ice(s)
        }
        fn is_face_sturdy(
            &mut self,
            s: BlockStateId,
            _: BlockPos,
            d: Direction,
        ) -> Result<bool, Error> {
            Ok(self.fluids.sturdy(s, d))
        }
    }
    let mut scene = Scene { world, fluids };
    let state = fluids.fluid(world.block(p[0], p[1], p[2]));
    let Ok(flow) = flow_at(&mut scene, state, BlockPos::new(p[0], p[1], p[2]));
    flow
}
