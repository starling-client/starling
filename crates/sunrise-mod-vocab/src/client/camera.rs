//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// `Mth.lerp(float, float, float)`.
pub fn lerp(alpha: f32, p0: f32, p1: f32) -> f32 {
    p0 + alpha * (p1 - p0)
}
/// `Mth.wrapDegrees(float)`.
pub fn wrap_degrees(angle: f32) -> f32 {
    let mut normalized = angle % 360.0;
    if normalized >= 180.0 {
        normalized -= 360.0;
    }
    if normalized < -180.0 {
        normalized += 360.0;
    }
    normalized
}
/// What `AbstractClientPlayer.getFieldOfViewModifier` reads.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct FovPlayer {
    /// `Abilities.flying`.
    pub flying: bool,
    /// `Abilities.getWalkingSpeed()`.
    pub walking_speed: f32,
    /// `getAttributeValue(MOVEMENT_SPEED)`.
    pub movement_speed: f64,
    /// The item in use and its use ticks, when using one.
    pub using: Option<(UseItem, i32)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum UseItem {
    Bow,
    Spyglass,
    Other,
}
/// Why `ClientHost::set_local_rotation` changed nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum LocalRotationError {
    /// No client world is open.
    NoWorld,
    /// The yaw or the pitch is NaN or infinite.
    NotFinite,
}
/// `CameraType`: where the camera sits for the local player. `key.togglePerspective` (F5) cycles
/// it first person, third person behind, third person in front.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum CameraType {
    #[default]
    FirstPerson,
    ThirdPersonBack,
    ThirdPersonFront,
}
impl CameraType {
    /// `CameraType.isFirstPerson`.
    pub fn is_first_person(self) -> bool {
        self == Self::FirstPerson
    }
    /// `CameraType.isMirrored`: third person in front.
    pub fn is_mirrored(self) -> bool {
        self == Self::ThirdPersonFront
    }
    /// `CameraType.cycle` (F5).
    pub fn cycle(self) -> Self {
        match self {
            Self::FirstPerson => Self::ThirdPersonBack,
            Self::ThirdPersonBack => Self::ThirdPersonFront,
            Self::ThirdPersonFront => Self::FirstPerson,
        }
    }
}
/// The camera entity as `Camera.alignWithEntity` and the field-of-view
/// death and hurt effects see it.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct CameraEntity {
    pub position: [f64; 3],
    /// `xo`, `yo`, `zo`.
    pub previous: [f64; 3],
    pub x_rot: f32,
    pub y_rot: f32,
    pub x_rot_o: f32,
    pub y_rot_o: f32,
    /// `Entity.getEyeHeight()`.
    pub eye_height: f32,
    /// A living entity's `getScale()` and camera distance attribute.
    pub living: Option<LivingCamera>,
    /// The vehicle's scale and camera distance when it is living.
    pub mount: Option<(f32, f64)>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct LivingCamera {
    pub scale: f32,
    pub camera_distance: f64,
    /// `isSleeping()` with `getBedOrientation()` as its `toYRot()`.
    pub sleeping: Option<Option<f32>>,
    /// `isDeadOrDying()` and `deathTime`.
    pub dead: Option<i32>,
}
impl CameraEntity {
    /// `Entity.getXRot(partialTicks)`.
    pub fn view_x_rot(&self, partial: f32) -> f32 {
        if partial == 1.0 {
            self.x_rot
        } else {
            lerp(partial, self.x_rot_o, self.x_rot)
        }
    }
    /// `LocalPlayer.getViewYRot` (`getYRot(partialTicks)`, a `Mth.rotLerp`).
    pub fn view_y_rot(&self, partial: f32) -> f32 {
        if partial == 1.0 {
            self.y_rot
        } else {
            self.y_rot_o + partial * wrap_degrees(self.y_rot - self.y_rot_o)
        }
    }
}

/// The placed camera position and its explicit world-space axis overrides.
/// An overridden axis stays pinned even when it equals the placed coordinate.
// no wire: lends the already placed camera to a render-frame callback.
// exhaustive: the game supplies the placement; mods use the methods to edit it.
#[derive(Clone, Copy, Debug)]
pub struct CameraPosition {
    placed: [f64; 3],
    overrides: [Option<f64>; 3],
}

impl CameraPosition {
    // hidden: game-side camera position construction for camera_position dispatch.
    #[doc(hidden)]
    pub const fn placed_at(placed: [f64; 3]) -> Self {
        Self {
            placed,
            overrides: [None; 3],
        }
    }

    /// The coordinates after the previous listeners' edits, in X, Y, Z order.
    pub fn coordinates(&self) -> [f64; 3] {
        std::array::from_fn(|i| self.overrides[i].unwrap_or(self.placed[i]))
    }

    /// Set explicit X, Y and Z overrides. `None` leaves that axis and any prior override alone.
    /// Returns `false` without changing any axis if a proposed coordinate is not finite.
    pub fn set(&mut self, axes: [Option<f64>; 3]) -> bool {
        if !axes.into_iter().flatten().all(f64::is_finite) {
            return false;
        }
        for (kept, axis) in self.overrides.iter_mut().zip(axes) {
            if axis.is_some() {
                *kept = axis;
            }
        }
        true
    }

    // hidden: game-side readout of camera_position overrides after dispatch.
    #[doc(hidden)]
    pub const fn overrides(&self) -> [Option<f64>; 3] {
        self.overrides
    }
}

/// Column-major 4x4 presentation-only view transform composed at vanilla's
/// `GameRenderer.bobHurt` / `bobView` stage (`render_view_transform`).
///
/// Operations post-multiply onto the current matrix (`self = self * m`), matching vanilla's
/// `PoseStack` multiplication order. A method returning `false` leaves the previous transform
/// intact: the proposed transform or a composition used by the frame cannot be inverted finitely.
// no wire: client-local per-frame render transform at the GameRenderer bob stage.
// exhaustive: its fields are private, so mods compose through its transform methods.
#[derive(Clone, Copy)]
pub struct ViewTransform<'frame> {
    matrix: [f32; 16],
    usable_in_frame: &'frame dyn Fn([f32; 16]) -> bool,
}

impl<'frame> ViewTransform<'frame> {
    const IDENTITY: [f32; 16] = glam::Mat4::IDENTITY.to_cols_array();

    // hidden: game-side per-frame scope constructor for render_view_transform dispatch.
    #[doc(hidden)]
    pub fn with_frame<R>(
        usable_in_frame: impl Fn([f32; 16]) -> bool,
        callback: impl for<'scope> FnOnce(&mut ViewTransform<'scope>) -> R,
    ) -> R {
        callback(&mut ViewTransform {
            matrix: Self::IDENTITY,
            usable_in_frame: &usable_in_frame,
        })
    }

    /// Returns the column-major 4x4 matrix.
    pub const fn matrix(&self) -> [f32; 16] {
        self.matrix
    }

    /// Replaces the transform if it and the frame matrices it affects have finite inverses.
    pub fn set_matrix(&mut self, matrix: [f32; 16]) -> bool {
        let m = glam::Mat4::from_cols_array(&matrix);
        if !m.is_finite() {
            return false;
        }
        let determinant = m.determinant();
        if determinant == 0.0
            || !determinant.is_finite()
            || !m.inverse().is_finite()
            || !(self.usable_in_frame)(matrix)
        {
            return false;
        }
        self.matrix = matrix;
        true
    }

    /// Resets the transform to identity, subject to the same check as [`Self::set_matrix`].
    pub fn reset(&mut self) -> bool {
        self.set_matrix(Self::IDENTITY)
    }

    /// Post-multiplies by a column-major 4x4 matrix, with the same check as [`Self::set_matrix`].
    pub fn mul(&mut self, rhs: [f32; 16]) -> bool {
        let product = glam::Mat4::from_cols_array(&self.matrix) * glam::Mat4::from_cols_array(&rhs);
        self.set_matrix(product.to_cols_array())
    }

    /// Post-multiplies a translation `(x, y, z)` in blocks along camera-local axes.
    pub fn translate(&mut self, x: f32, y: f32, z: f32) -> bool {
        self.mul([
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            x, y, z, 1.0,
        ])
    }

    /// Post-multiplies a rotation about the local X axis by `radians`.
    pub fn rotate_x(&mut self, radians: f32) -> bool {
        let (s, c) = radians.sin_cos();
        self.mul([
            1.0, 0.0, 0.0, 0.0, //
            0.0, c, s, 0.0, //
            0.0, -s, c, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    /// Post-multiplies a rotation about the local Y axis by `radians`.
    pub fn rotate_y(&mut self, radians: f32) -> bool {
        let (s, c) = radians.sin_cos();
        self.mul([
            c, 0.0, -s, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            s, 0.0, c, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    /// Post-multiplies a rotation about the local Z axis by `radians`.
    pub fn rotate_z(&mut self, radians: f32) -> bool {
        let (s, c) = radians.sin_cos();
        self.mul([
            c, s, 0.0, 0.0, //
            -s, c, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    /// Post-multiplies a scale `(x, y, z)`.
    pub fn scale(&mut self, x: f32, y: f32, z: f32) -> bool {
        self.mul([
            x, 0.0, 0.0, 0.0, //
            0.0, y, 0.0, 0.0, //
            0.0, 0.0, z, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ])
    }
}

#[cfg(test)]
mod view_transform_tests {
    use super::ViewTransform;

    #[test]
    fn post_multiplies_translation_and_rejects_non_finite_inputs() {
        let mut t = ViewTransform {
            matrix: ViewTransform::IDENTITY,
            usable_in_frame: &|_| true,
        };
        assert_eq!(t.matrix(), ViewTransform::IDENTITY);
        assert!(t.translate(1.0, -2.0, 3.0));
        assert!(t.scale(2.0, 2.0, 2.0));
        assert_eq!(
            t.matrix(),
            [
                2.0, 0.0, 0.0, 0.0, //
                0.0, 2.0, 0.0, 0.0, //
                0.0, 0.0, 2.0, 0.0, //
                1.0, -2.0, 3.0, 1.0,
            ]
        );
        let before = t.matrix();
        assert!(!t.translate(f32::NAN, 0.0, 0.0));
        assert!(!t.rotate_z(f32::INFINITY));
        assert!(!t.scale(1.0, f32::NEG_INFINITY, 1.0));
        assert!(!t.set_matrix([f32::NAN; 16]));
        assert_eq!(t.matrix(), before);
        assert!(t.reset());
        assert_eq!(t.matrix(), ViewTransform::IDENTITY);
    }

    #[test]
    fn singular_view_transform_writes_are_refused_without_losing_the_previous_transform() {
        let mut transform = ViewTransform {
            matrix: ViewTransform::IDENTITY,
            usable_in_frame: &|_| true,
        };
        assert!(transform.translate(2.0, 3.0, 4.0));
        let previous = transform.matrix();
        let singular = [
            1.0, 0.0, 0.0, 0.0, // the first two columns are identical
            1.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        assert!(!transform.set_matrix(singular));
        assert!(!transform.mul(singular));
        assert!(!transform.scale(0.0, 1.0, 1.0));
        assert_eq!(transform.matrix(), previous);
    }

    #[test]
    fn rotations_match_right_handed_axes() {
        let mut rz = ViewTransform {
            matrix: ViewTransform::IDENTITY,
            usable_in_frame: &|_| true,
        };
        assert!(rz.rotate_z(core::f32::consts::FRAC_PI_2));
        let m = rz.matrix();
        assert!(m[0].abs() < 1e-6);
        assert!((m[1] - 1.0).abs() < 1e-6);
        assert!((m[4] + 1.0).abs() < 1e-6);
        assert!(m[5].abs() < 1e-6);
    }
}
