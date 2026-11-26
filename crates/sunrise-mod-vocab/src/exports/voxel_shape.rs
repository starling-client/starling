pub use block::voxel_shape::{VoxelShape, VoxelShapeError};

pub mod __private {
    pub use crate::block::voxel_shape::__private::{voxel_shape_from_core, voxel_shape_into_core};
}
