use sunrise_core::geometry::{Aabb, ShapeOp, Vec3, VoxelShape as CoreVoxelShape};

// no wire: block collision queries lend immutable geometry in-process.
/// An immutable voxel shape. Cloning retains the geometry without copying its boxes.
// exhaustive: the field is private; shapes are built only through the constructors.
#[derive(Clone, Debug)]
pub struct VoxelShape(CoreVoxelShape);

// no wire: construction failures are returned directly to the caller in-process.
/// Why a [`VoxelShape`] could not be built from boxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VoxelShapeError {
    /// A coordinate is not finite, or a minimum exceeds its corresponding maximum.
    InvalidBounds,
    /// The union cannot be represented by a voxel grid.
    Unrepresentable,
}

impl std::fmt::Display for VoxelShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidBounds => "voxel shape boxes must have finite, ordered bounds",
            Self::Unrepresentable => "voxel shape union cannot be represented",
        })
    }
}

impl std::error::Error for VoxelShapeError {}

impl VoxelShape {
    /// A shape with no occupied volume (`Shapes.empty`).
    pub fn empty() -> Self {
        Self(CoreVoxelShape::empty())
    }

    /// Build the union of `[min_x, min_y, min_z, max_x, max_y, max_z]` boxes, as
    /// `Shapes.or` combines `Shapes.box` results. Bounds must be finite and ordered.
    /// Valid boxes too thin to occupy a voxel under `Shapes.create` contribute no volume.
    pub fn try_from_boxes(boxes: &[[f64; 6]]) -> Result<Self, VoxelShapeError> {
        let mut shape = CoreVoxelShape::empty();
        for bounds in boxes {
            let &[x0, y0, z0, x1, y1, z1] = bounds;
            if !bounds.iter().all(|v| v.is_finite()) || x0 > x1 || y0 > y1 || z0 > z1 {
                return Err(VoxelShapeError::InvalidBounds);
            }
            let bounds = Aabb::new(Vec3::new(x0, y0, z0), Vec3::new(x1, y1, z1));
            shape = CoreVoxelShape::from_box(bounds)
                .and_then(|part| shape.join(&part, ShapeOp::OR))
                .map_err(|_| VoxelShapeError::Unrepresentable)?;
        }
        Ok(Self(shape))
    }

    /// Whether this shape has no occupied volume.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate over the shape's occupied boxes as `[min_x, min_y, min_z, max_x, max_y, max_z]`.
    /// This borrows the shape without allocating. The union may merge or split input boxes.
    /// Boxes are in the shape's coordinate space (world space for block collision visits).
    pub fn boxes(&self) -> impl ExactSizeIterator<Item = [f64; 6]> + '_ {
        self.0.boxes().iter().map(|bounds| {
            let lo = bounds.min_position();
            let hi = bounds.max_position();
            [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z]
        })
    }
}

pub(crate) mod __private {
    use super::{CoreVoxelShape, VoxelShape};

    /// Wrap engine geometry without rebuilding its grid.
    pub fn voxel_shape_from_core(shape: CoreVoxelShape) -> VoxelShape {
        VoxelShape(shape)
    }

    /// Take the engine geometry without rebuilding its grid.
    pub fn voxel_shape_into_core(shape: VoxelShape) -> CoreVoxelShape {
        shape.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compound_voxel_shape_validates_geometry_not_box_count() {
        let mut input: Vec<_> = (0..65)
            .rev()
            .map(|i| {
                let x = f64::from(2 * i);
                [x, 0.25, -2.0, x + 1.0, 0.75, -1.0]
            })
            .collect();
        let shape = VoxelShape::try_from_boxes(&input).unwrap();
        assert!(!shape.is_empty());
        let mut actual: Vec<_> = shape.boxes().collect();
        actual.sort_by(|a, b| a[0].total_cmp(&b[0]));
        assert_eq!(actual.len(), 65);
        assert_eq!(actual.first(), Some(&[0.0, 0.25, -2.0, 1.0, 0.75, -1.0]));
        assert_eq!(actual.last(), Some(&[128.0, 0.25, -2.0, 129.0, 0.75, -1.0]));
        for (i, bounds) in (0..65).zip(actual) {
            let x = f64::from(i) * 2.0;
            assert_eq!(bounds, [x, 0.25, -2.0, x + 1.0, 0.75, -1.0]);
        }
        input[0][0] = f64::INFINITY;
        assert!(matches!(
            VoxelShape::try_from_boxes(&input),
            Err(VoxelShapeError::InvalidBounds)
        ));
        assert!(matches!(
            VoxelShape::try_from_boxes(&[[0.0, 1.0, 0.0, 1.0, 0.0, 1.0]]),
            Err(VoxelShapeError::InvalidBounds)
        ));
    }
}
