//! How far a block entity is drawn: `BlockEntityRenderDispatcher.tryExtractRenderState`'s
//! `shouldRenderOffScreen` and `shouldRender` (`getViewDistance`) for each renderer. Every
//! block entity draw asks this, so they all keep vanilla's distances.
use glam::DVec3;

/// The renderers whose reach differs; every other block entity renderer keeps the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// `BlockEntityRenderer`'s defaults: 64 blocks around the camera.
    Default,
    /// `PistonHeadRenderer`: 68 blocks.
    Piston,
    /// `BlockEntityWithBoundingBoxRenderer` (structure blocks): 96 blocks, off screen too.
    BoundingBox,
    /// `BeaconRenderer`: the render distance, measured across the ground, off screen too.
    Beacon,
    /// `TestInstanceRenderer`: the beacon's reach or the bounding box's, off screen too.
    TestInstance,
    /// `TheEndGatewayRenderer`: 256 blocks.
    EndGateway,
}

impl Reach {
    /// The reach of the block entity on block `name`.
    pub fn of(name: &str) -> Self {
        match name {
            "minecraft:moving_piston" => Self::Piston,
            "minecraft:structure_block" => Self::BoundingBox,
            "minecraft:beacon" => Self::Beacon,
            "minecraft:test_instance_block" => Self::TestInstance,
            "minecraft:end_gateway" => Self::EndGateway,
            _ => Self::Default,
        }
    }

    /// `shouldRenderOffScreen`: drawn whether or not its section is in view (beams, outlines).
    pub fn off_screen(self) -> bool {
        matches!(self, Self::BoundingBox | Self::Beacon | Self::TestInstance)
    }

    /// `getViewDistance`, in blocks, at `render_distance` chunks (`Options.getEffectiveRenderDistance`).
    pub fn view_distance(self, render_distance: i32) -> i32 {
        match self {
            Self::Default => 64,
            Self::Piston => 68,
            Self::BoundingBox => 96,
            Self::Beacon => render_distance * 16,
            Self::TestInstance => (render_distance * 16).max(96),
            Self::EndGateway => 256,
        }
    }

    /// `shouldRender`: the block entity at `pos` is drawn from `camera`.
    pub fn should_render(self, pos: [i32; 3], camera: DVec3, render_distance: i32) -> bool {
        // Vec3.atCenterOf(pos).closerThan(camera, distance): strictly closer.
        let centre = DVec3::new(
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        );
        let closer = |offset: DVec3, distance: i32| {
            let distance = f64::from(distance);
            offset.length_squared() < distance * distance
        };
        // BeaconRenderer.shouldRender: both points multiplied by (1, 0, 1).
        let across = |offset: DVec3| DVec3::new(offset.x, 0.0, offset.z);
        let offset = camera - centre;
        match self {
            Self::Beacon => closer(across(offset), self.view_distance(render_distance)),
            Self::TestInstance => {
                closer(across(offset), Self::Beacon.view_distance(render_distance))
                    || closer(offset, Self::BoundingBox.view_distance(render_distance))
            }
            _ => closer(offset, self.view_distance(render_distance)),
        }
    }
}
