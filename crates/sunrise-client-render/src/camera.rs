// `joml::Mat4::rotation`, `rotate`, `rotate_affine`, `scale`, `transform_affine`,
// `transform_position` and `invert_orthonormal`, and `joml::Quat::rotation_x`/`_y`/`_z`, `rotate_z` and
// `rotation_zyx`, in this file are adapted from JOML 1.10.9's `Matrix4f`, `Quaternionf`,
// `Vector3f` and `Vector4f` (https://github.com/JOML-CI/JOML, MIT; see THIRD_PARTY_NOTICES.md).
//
// Copyright (c) 2015-2026 Richard Greenlees
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

//! The camera (`Camera.tick`, `tickFov`, `calculateFov`, `alignWithEntity`,
//! `getMaxZoom`, `setRotation`, `move`), view and hurt bobbing
//! (`GameRenderer.bobView`, `bobHurt`), the walk and bob history they read
//! (`ClientAvatarState`), and `AbstractClientPlayer.getFieldOfViewModifier`.
//!
//! The float arithmetic follows vanilla and JOML's default (non-FMA,
//! non-fast-math) code paths operation by operation.
pub use mod_api::client::camera::{
    CameraEntity, CameraType, FovPlayer, LivingCamera, UseItem, lerp, wrap_degrees,
};
use sunrise_core::geometry::trig;

/// `AbstractBoat.clampRotation` for a rider that isn't a mob: the view kept
/// within 105 degrees of the boat's yaw (`Mth.wrapDegrees`, then
/// `yRot + targetDelta - delta` in that order, in floats). Returns the new yaw
/// and the turn it made. The per-turn path also adds that turn to `yRotO`.
pub fn clamp_boat_rotation(yaw: f32, boat_yaw: f32) -> (f32, f32) {
    let delta = wrap_degrees(yaw - boat_yaw);
    let target = delta.clamp(-105.0, 105.0);
    (yaw + target - delta, target - delta)
}

/// `(float) Math.PI`.
const PI: f32 = std::f32::consts::PI;
/// `(float) (Math.PI / 180.0)`.
const DEG_TO_RAD: f32 = trig::DEG_TO_RAD;

/// `Math.min(float, float)`: NaN wins and -0.0 is below 0.0.
pub fn java_min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        f32::from_bits(a.to_bits() | b.to_bits())
    } else if a <= b {
        a
    } else {
        b
    }
}
/// `Math.max(float, float)`.
pub fn java_max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        f32::from_bits(a.to_bits() & b.to_bits())
    } else if a >= b {
        a
    } else {
        b
    }
}
/// `Mth.lerp(double, double, double)`.
pub fn lerp_d(alpha: f64, p0: f64, p1: f64) -> f64 {
    p0 + alpha * (p1 - p0)
}
/// `Mth.clamp(float, float, float)`.
pub fn clamp(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else {
        java_min(value, max)
    }
}

/// JOML's float trigonometry (`org.joml.Math` without fast math).
pub mod joml {
    use super::PI;
    const TAU: f32 = std::f32::consts::TAU;
    const HALF_PI: f32 = std::f32::consts::FRAC_PI_2;

    /// `Math.sin(float)`: `java.lang.Math.sin` narrowed.
    pub fn sin(rad: f32) -> f32 {
        sunrise_core::jmath::sin(f64::from(rad)) as f32
    }
    /// `Math.cosFromSin(float, float)`.
    pub fn cos_from_sin(sin: f32, angle: f32) -> f32 {
        let cos = (1.0 - sin * sin).sqrt();
        let a = angle + HALF_PI;
        let mut b = a - (a / TAU) as i32 as f32 * TAU;
        if f64::from(b) < 0.0 {
            b += TAU;
        }
        if b >= PI { -cos } else { cos }
    }

    /// A JOML `Quaternionf` (x, y, z, w).
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Quat {
        pub x: f32,
        pub y: f32,
        pub z: f32,
        pub w: f32,
    }
    impl Quat {
        pub const IDENTITY: Self = Self {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        };
        /// `Quaternionf.rotationYXZ(angleY, angleX, angleZ)`.
        pub fn rotation_yxz(angle_y: f32, angle_x: f32, angle_z: f32) -> Self {
            let sx = sin(angle_x * 0.5);
            let cx = cos_from_sin(sx, angle_x * 0.5);
            let sy = sin(angle_y * 0.5);
            let cy = cos_from_sin(sy, angle_y * 0.5);
            let sz = sin(angle_z * 0.5);
            let cz = cos_from_sin(sz, angle_z * 0.5);
            let x = cy * sx;
            let y = sy * cx;
            let z = sy * sx;
            let w = cy * cx;
            Self {
                x: x * cz + y * sz,
                y: y * cz - x * sz,
                z: w * sz - z * cz,
                w: w * cz + z * sz,
            }
        }
        /// `Quaternionf.rotationZYX(angleZ, angleY, angleX)`.
        pub fn rotation_zyx(angle_z: f32, angle_y: f32, angle_x: f32) -> Self {
            let sx = sin(angle_x * 0.5);
            let cx = cos_from_sin(sx, angle_x * 0.5);
            let sy = sin(angle_y * 0.5);
            let cy = cos_from_sin(sy, angle_y * 0.5);
            let sz = sin(angle_z * 0.5);
            let cz = cos_from_sin(sz, angle_z * 0.5);
            let cycz = cy * cz;
            let sysz = sy * sz;
            let sycz = sy * cz;
            let cysz = cy * sz;
            Self {
                x: sx * cycz - cx * sysz,
                y: cx * sycz + sx * cysz,
                z: cx * cysz - sx * sycz,
                w: cx * cycz + sx * sysz,
            }
        }
        /// `Quaternionf.rotationX(angle)`.
        pub fn rotation_x(angle: f32) -> Self {
            let (x, w) = Self::half_turn(angle);
            Self {
                x,
                y: 0.0,
                z: 0.0,
                w,
            }
        }
        /// `Quaternionf.rotationY(angle)`.
        pub fn rotation_y(angle: f32) -> Self {
            let (y, w) = Self::half_turn(angle);
            Self {
                x: 0.0,
                y,
                z: 0.0,
                w,
            }
        }
        /// `Quaternionf.rotationZ(angle)`.
        pub fn rotation_z(angle: f32) -> Self {
            let (z, w) = Self::half_turn(angle);
            Self {
                x: 0.0,
                y: 0.0,
                z,
                w,
            }
        }
        /// `Quaternionf.rotateZ(angle)` for provider roll, with HotSpot's `Math.sin`.
        pub(crate) fn rotate_z(self, angle: f32) -> Self {
            let half = angle * 0.5;
            let sin = sunrise_core::jmath::sin(f64::from(half)) as f32;
            let cos = cos_from_sin(sin, half);
            Self {
                x: self.x * cos + self.y * sin,
                y: self.y * cos - self.x * sin,
                z: self.w * sin + self.z * cos,
                w: self.w * cos - self.z * sin,
            }
        }
        fn half_turn(angle: f32) -> (f32, f32) {
            let sin = sin(angle * 0.5);
            (sin, cos_from_sin(sin, angle * 0.5))
        }
        /// `Quaternionf.transform(x, y, z, dest)` (`Vector3f.rotate`).
        pub fn transform(&self, v: [f32; 3]) -> [f32; 3] {
            let [x, y, z] = v;
            let xx = self.x * self.x;
            let yy = self.y * self.y;
            let zz = self.z * self.z;
            let ww = self.w * self.w;
            let xy = self.x * self.y;
            let xz = self.x * self.z;
            let yz = self.y * self.z;
            let xw = self.x * self.w;
            let zw = self.z * self.w;
            let yw = self.y * self.w;
            let k = 1.0 / (xx + yy + zz + ww);
            [
                (xx - yy - zz + ww) * k * x + (2.0 * (xy - zw) * k * y + 2.0 * (xz + yw) * k * z),
                2.0 * (xy + zw) * k * x + ((yy - xx - zz + ww) * k * y + 2.0 * (yz - xw) * k * z),
                2.0 * (xz - yw) * k * x + (2.0 * (yz + xw) * k * y + (zz - xx - yy + ww) * k * z),
            ]
        }
    }

    const PERSPECTIVE: u8 = 1;
    const AFFINE: u8 = 2;
    const IDENTITY: u8 = 4;
    const TRANSLATION: u8 = 8;
    const ORTHONORMAL: u8 = 16;

    /// A JOML `Matrix4f` with its property flags, which select the code
    /// paths (and so the rounding) of later operations. `m[column * 4 + row]`.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Mat4 {
        pub m: [f32; 16],
        pub properties: u8,
    }
    impl Default for Mat4 {
        fn default() -> Self {
            Self::IDENTITY
        }
    }
    impl Mat4 {
        /// `new Matrix4f()`.
        pub const IDENTITY: Self = Self {
            m: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            properties: 30,
        };
        fn get(&self, column: usize, row: usize) -> f32 {
            self.m[column * 4 + row]
        }
        fn set(&mut self, column: usize, row: usize, v: f32) {
            self.m[column * 4 + row] = v;
        }
        fn reset_unless_identity(&mut self) {
            if self.properties & IDENTITY == 0 {
                self.m = Self::IDENTITY.m;
            }
        }
        /// `Matrix4f.translation(x, y, z)`.
        pub fn translation(&mut self, x: f32, y: f32, z: f32) {
            self.reset_unless_identity();
            self.set(3, 0, x);
            self.set(3, 1, y);
            self.set(3, 2, z);
            self.properties = 26;
        }
        /// `Matrix4f.setTranslation(x, y, z)`.
        pub fn set_translation(&mut self, x: f32, y: f32, z: f32) {
            self.set(3, 0, x);
            self.set(3, 1, y);
            self.set(3, 2, z);
            self.properties &= !(PERSPECTIVE | IDENTITY);
        }
        /// `Matrix4f.translate(x, y, z)`.
        pub fn translate(&mut self, x: f32, y: f32, z: f32) {
            if self.properties & IDENTITY != 0 {
                self.translation(x, y, z);
                return;
            }
            for row in 0..4 {
                let v = self.get(0, row) * x
                    + (self.get(1, row) * y + (self.get(2, row) * z + self.get(3, row)));
                self.set(3, row, v);
            }
            self.properties &= !(PERSPECTIVE | IDENTITY);
        }
        fn rotation_axis(&mut self, axis: usize, ang: f32) {
            let sin = sin(ang);
            let cos = cos_from_sin(sin, ang);
            self.reset_unless_identity();
            match axis {
                0 => {
                    self.set(1, 1, cos);
                    self.set(1, 2, sin);
                    self.set(2, 1, -sin);
                    self.set(2, 2, cos);
                }
                1 => {
                    self.set(0, 0, cos);
                    self.set(0, 2, -sin);
                    self.set(2, 0, sin);
                    self.set(2, 2, cos);
                }
                _ => {
                    self.set(0, 0, cos);
                    self.set(0, 1, sin);
                    self.set(1, 0, -sin);
                    self.set(1, 1, cos);
                }
            }
            self.properties = 18;
        }
        /// `Matrix4f.rotateX/rotateY/rotateZ(ang)`, with the identity and
        /// pure-translation shortcuts.
        fn rotate_axis(&mut self, axis: usize, ang: f32) {
            if self.properties & IDENTITY != 0 {
                self.rotation_axis(axis, ang);
                return;
            }
            if self.properties & TRANSLATION != 0 {
                let (x, y, z) = (self.get(3, 0), self.get(3, 1), self.get(3, 2));
                self.rotation_axis(axis, ang);
                self.set_translation(x, y, z);
                return;
            }
            let sin = sin(ang);
            let cos = cos_from_sin(sin, ang);
            let s = *self;
            match axis {
                0 => {
                    for row in 0..4 {
                        let (l1, l2) = (s.get(1, row), s.get(2, row));
                        self.set(2, row, l1 * -sin + l2 * cos);
                        self.set(1, row, l1 * cos + l2 * sin);
                    }
                }
                1 => {
                    for row in 0..4 {
                        let (l0, l2) = (s.get(0, row), s.get(2, row));
                        self.set(0, row, l0 * cos + l2 * -sin);
                        self.set(2, row, l0 * sin + l2 * cos);
                    }
                }
                _ => {
                    // rotateTowardsXY(sin, cos).
                    let (dx, dy) = (sin, cos);
                    for row in 0..4 {
                        let (l0, l1) = (s.get(0, row), s.get(1, row));
                        self.set(0, row, l0 * dy + l1 * dx);
                        self.set(1, row, l0 * -dx + l1 * dy);
                    }
                }
            }
            self.properties &= !(PERSPECTIVE | IDENTITY | TRANSLATION);
        }
        /// `Matrix4f.scale(x, y, z)`, with the identity shortcut (`scaling`).
        pub fn scale(&mut self, x: f32, y: f32, z: f32) {
            let one = [x, y, z].iter().all(|v| v.abs() == 1.0);
            if self.properties & IDENTITY != 0 {
                self.set(0, 0, x);
                self.set(1, 1, y);
                self.set(2, 2, z);
                self.properties = AFFINE | if one { ORTHONORMAL } else { 0 };
                return;
            }
            for (column, v) in [x, y, z].into_iter().enumerate() {
                for row in 0..4 {
                    self.set(column, row, self.get(column, row) * v);
                }
            }
            self.properties &=
                !(PERSPECTIVE | IDENTITY | TRANSLATION | if one { 0 } else { ORTHONORMAL });
        }
        pub fn rotate_x(&mut self, ang: f32) {
            self.rotate_axis(0, ang);
        }
        pub fn rotate_y(&mut self, ang: f32) {
            self.rotate_axis(1, ang);
        }
        pub fn rotate_z(&mut self, ang: f32) {
            // rotateTowardsXY also short-cuts on identity, but rotateZ has
            // already taken that branch.
            self.rotate_axis(2, ang);
        }
        /// `Matrix4f.rotation(Quaternionfc)`.
        pub fn rotation(&mut self, q: Quat) {
            let (w2, x2, y2, z2) = (q.w * q.w, q.x * q.x, q.y * q.y, q.z * q.z);
            let zw = q.z * q.w;
            let dzw = zw + zw;
            let xy = q.x * q.y;
            let dxy = xy + xy;
            let xz = q.x * q.z;
            let dxz = xz + xz;
            let yw = q.y * q.w;
            let dyw = yw + yw;
            let yz = q.y * q.z;
            let dyz = yz + yz;
            let xw = q.x * q.w;
            let dxw = xw + xw;
            self.reset_unless_identity();
            self.set(0, 0, w2 + x2 - z2 - y2);
            self.set(0, 1, dxy + dzw);
            self.set(0, 2, dxz - dyw);
            self.set(1, 0, -dzw + dxy);
            self.set(1, 1, y2 - z2 + w2 - x2);
            self.set(1, 2, dyz + dxw);
            self.set(2, 0, dyw + dxz);
            self.set(2, 1, dyz - dxw);
            self.set(2, 2, z2 - y2 - x2 + w2);
            self.properties = AFFINE | ORTHONORMAL;
        }
        /// `Matrix4f.rotate(Quaternionfc)` of an affine matrix: `rotation` from the identity,
        /// `rotateTranslation` from a pure translation (the rotation, the translation kept),
        /// `rotateAffine` otherwise.
        pub fn rotate(&mut self, q: Quat) {
            if self.properties & IDENTITY != 0 {
                self.rotation(q);
            } else if self.properties & TRANSLATION != 0 {
                let translation = [self.get(3, 0), self.get(3, 1), self.get(3, 2)];
                self.m = Self::IDENTITY.m;
                self.rotation(q);
                for (row, v) in translation.into_iter().enumerate() {
                    self.set(3, row, v);
                }
            } else {
                self.rotate_affine(q);
            }
        }
        /// `Matrix4f.transformPosition(x, y, z, dest)` (`Vector3f.mulPosition`: unchanged by
        /// the identity, translated by a pure translation, else JOML's `Math.fma` unfused).
        pub fn transform_position(&self, v: [f32; 3]) -> [f32; 3] {
            let [x, y, z] = v;
            if self.properties & IDENTITY != 0 {
                return v;
            }
            if self.properties & TRANSLATION != 0 {
                return [x + self.get(3, 0), y + self.get(3, 1), z + self.get(3, 2)];
            }
            std::array::from_fn(|row| {
                self.get(0, row) * x
                    + (self.get(1, row) * y + (self.get(2, row) * z + self.get(3, row)))
            })
        }
        /// An affine matrix of no more specific kind, as JOML's property flags have it.
        pub fn affine(m: glam::Mat4) -> Self {
            Self {
                m: m.to_cols_array(),
                properties: AFFINE,
            }
        }
        /// `Matrix4f.mulAffine(Matrix4fc, Matrix4f)` in JOML's unfused right-to-left order.
        pub fn mul_affine(&self, b: &Self) -> Self {
            let mut out = [0.0f32; 16];
            for col in 0..3 {
                for row in 0..3 {
                    out[col * 4 + row] = self.get(0, row) * b.get(col, 0)
                        + (self.get(1, row) * b.get(col, 1) + self.get(2, row) * b.get(col, 2));
                }
            }
            for row in 0..3 {
                out[12 + row] = self.get(0, row) * b.get(3, 0)
                    + (self.get(1, row) * b.get(3, 1)
                        + (self.get(2, row) * b.get(3, 2) + self.get(3, row)));
            }
            out[15] = 1.0;
            Self {
                m: out,
                properties: AFFINE,
            }
        }
        /// `Matrix4f.rotate(Quaternionfc)` of an affine matrix that is neither the identity nor
        /// a pure translation (`rotateAffine`).
        pub fn rotate_affine(&mut self, q: Quat) {
            debug_assert!(
                self.properties & AFFINE != 0 && self.properties & (IDENTITY | TRANSLATION) == 0
            );
            let (w2, x2, y2, z2) = (q.w * q.w, q.x * q.x, q.y * q.y, q.z * q.z);
            let zw = q.z * q.w;
            let dzw = zw + zw;
            let xy = q.x * q.y;
            let dxy = xy + xy;
            let xz = q.x * q.z;
            let dxz = xz + xz;
            let yw = q.y * q.w;
            let dyw = yw + yw;
            let yz = q.y * q.z;
            let dyz = yz + yz;
            let xw = q.x * q.w;
            let dxw = xw + xw;
            let r = [
                [w2 + x2 - z2 - y2, dxy + dzw, dxz - dyw],
                [-dzw + dxy, y2 - z2 + w2 - x2, dyz + dxw],
                [dyw + dxz, dyz - dxw, z2 - y2 - x2 + w2],
            ];
            let s = *self;
            for (column, r) in r.iter().enumerate() {
                for row in 0..3 {
                    let v = s.get(0, row) * r[0] + s.get(1, row) * r[1] + s.get(2, row) * r[2];
                    self.set(column, row, v);
                }
                self.set(column, 3, 0.0);
            }
            self.properties &= !(PERSPECTIVE | IDENTITY | TRANSLATION);
        }
        /// `Vector4f.mul(Matrix4fc)` of an affine matrix (`mulAffine`, JOML's `Math.fma`
        /// unfused): the matrix times `v`, `w` kept.
        pub fn transform_affine(&self, v: [f32; 4]) -> [f32; 3] {
            debug_assert!(self.properties & AFFINE != 0);
            let [x, y, z, w] = v;
            std::array::from_fn(|row| {
                self.get(0, row) * x
                    + (self.get(1, row) * y + (self.get(2, row) * z + self.get(3, row) * w))
            })
        }
        /// `Matrix4f.invert()` of a rotation and translation (`invertOrthonormal`; with the
        /// identity and pure-translation shortcuts). `None` for any other matrix, which JOML
        /// inverts by other code paths.
        pub fn invert_orthonormal(&self) -> Option<Self> {
            if self.properties & IDENTITY != 0 {
                return Some(Self::IDENTITY);
            }
            let mut out = *self;
            if self.properties & TRANSLATION != 0 {
                for row in 0..3 {
                    out.set(3, row, -self.get(3, row));
                }
                out.properties = AFFINE | TRANSLATION | ORTHONORMAL;
                return Some(out);
            }
            if self.properties & ORTHONORMAL == 0 {
                return None;
            }
            let g = |c, r| self.get(c, r);
            let t = |c| -(g(c, 0) * g(3, 0) + g(c, 1) * g(3, 1) + g(c, 2) * g(3, 2));
            out.m = [
                g(0, 0),
                g(1, 0),
                g(2, 0),
                0.0,
                g(0, 1),
                g(1, 1),
                g(2, 1),
                0.0,
                g(0, 2),
                g(1, 2),
                g(2, 2),
                0.0,
                t(0),
                t(1),
                t(2),
                1.0,
            ];
            out.properties = AFFINE | ORTHONORMAL;
            Some(out)
        }
        pub fn to_glam(self) -> glam::Mat4 {
            glam::Mat4::from_cols_array(&self.m)
        }
    }
}
use joml::{Mat4, Quat};

/// `Camera.getViewRotationMatrix` to the last bit: `Matrix4f.rotation` of the conjugate of
/// `setRotation`'s quaternion, and its JOML `invert()`. `view_matrix` turns the same quaternion
/// into a matrix by glam's formula, which differs in the last bits (1.0 for 0.99999994, 6e-8 for
/// 0).
pub fn view_rotation_matrix(yaw: f32, pitch: f32) -> (glam::Mat4, glam::Mat4) {
    view_rotation_matrix_rolled(yaw, pitch, 0.0)
}

/// [`view_rotation_matrix`] with a roll about the view direction, in degrees (`Camera.setRotation`
/// with NeoForge's `roll`). Keeps `roll == 0.0` bit-identical to vanilla's `+0.0` z-angle.
pub fn view_rotation_matrix_rolled(yaw: f32, pitch: f32, roll: f32) -> (glam::Mat4, glam::Mat4) {
    let q = rotation_with_roll(yaw, pitch, roll);
    let mut m = Mat4::IDENTITY;
    m.rotation(Quat {
        x: -q.x,
        y: -q.y,
        z: -q.z,
        w: q.w,
    });
    let inverse = m.invert_orthonormal().expect("a rotation is orthonormal");
    (m.to_glam(), inverse.to_glam())
}

/// Camera-relative world view used by terrain, sky and the first-person hand.
#[sunrise_mod_api_macros::replaceable]
pub fn view_matrix(yaw: f32, pitch: f32) -> glam::Mat4 {
    view_matrix_rolled(yaw, pitch, 0.0)
}

/// `GameRenderer.projectPointToScreen`: world point `at` seen from a camera at `position` facing
/// `yaw`/`pitch` through column-major `projection` (the frame's reversed-Z perspective), as
/// normalised device x and y (-1 to 1 across the screen) and depth z. The view is the camera's
/// rotation only, without bob or spin, as vanilla's. None for a point at or behind the camera and
/// for a non-finite `at`, where vanilla returns a mirrored or NaN point.
pub fn project_point(
    projection: &[f32; 16],
    position: [f64; 3],
    yaw: f32,
    pitch: f32,
    at: [f64; 3],
) -> Option<[f32; 3]> {
    project_point_rolled(projection, position, yaw, pitch, 0.0, at)
}

/// [`project_point`] with a roll about the view direction, in degrees.
pub fn project_point_rolled(
    projection: &[f32; 16],
    position: [f64; 3],
    yaw: f32,
    pitch: f32,
    roll: f32,
    at: [f64; 3],
) -> Option<[f32; 3]> {
    if !at.iter().all(|v| v.is_finite()) {
        return None;
    }
    // Vanilla subtracts in doubles, then transforms the offset as floats.
    let offset = glam::Vec3::new(
        (at[0] - position[0]) as f32,
        (at[1] - position[1]) as f32,
        (at[2] - position[2]) as f32,
    );
    let view = if roll == 0.0 {
        view_matrix(yaw, pitch)
    } else {
        view_matrix_rolled(yaw, pitch, roll)
    };
    let clip = glam::Mat4::from_cols_array(projection) * view * offset.extend(1.0);
    if clip.w.is_nan() || clip.w <= 0.0 {
        return None;
    }
    let ndc = [clip.x / clip.w, clip.y / clip.w, clip.z / clip.w];
    ndc.iter().all(|v| v.is_finite()).then_some(ndc)
}

/// `Camera.setRotation` with NeoForge roll, in degrees (`ViewportEvent.ComputeCameraAngles`).
pub fn rotation_with_roll(yaw: f32, pitch: f32, roll: f32) -> Quat {
    let angle_z = if roll == 0.0 { 0.0 } else { -roll * DEG_TO_RAD };
    Quat::rotation_yxz(PI - yaw * DEG_TO_RAD, -pitch * DEG_TO_RAD, angle_z)
}

/// `view_matrix` with a roll about the view direction, in degrees.
pub fn view_matrix_rolled(yaw: f32, pitch: f32, roll: f32) -> glam::Mat4 {
    // Vanilla uses the conjugate of Camera.rotation, not a world-up look-at.
    let q = rotation_with_roll(yaw, pitch, roll);
    glam::Mat4::from_quat(glam::Quat::from_xyzw(-q.x, -q.y, -q.z, q.w))
}
/// `AbstractClientPlayer.getFieldOfViewModifier`'s product before `Mth.lerp(effectScale, 1, f)`:
/// the raw modifier `ComputeFovModifierEvent` carries. None while scoping with a spyglass in
/// first person, where vanilla returns `0.1` before that lerp (and before the event).
pub fn raw_field_of_view_modifier(p: &FovPlayer, first_person: bool) -> Option<f32> {
    let mut modifier = 1.0f32;
    if p.flying {
        modifier *= 1.1;
    }
    let walking_speed = p.walking_speed;
    if walking_speed != 0.0 {
        let speed_factor = p.movement_speed as f32 / walking_speed;
        modifier *= (speed_factor + 1.0) / 2.0;
    }
    if let Some((item, ticks)) = p.using {
        match item {
            UseItem::Bow => {
                let scale = java_min(ticks as f32 / 20.0, 1.0);
                modifier *= 1.0 - scale * scale * 0.15;
            }
            // isScoping: using a spyglass.
            UseItem::Spyglass if first_person => return None,
            _ => {}
        }
    }
    Some(modifier)
}

/// `AbstractClientPlayer.getFieldOfViewModifier(firstPerson, effectScale)`.
pub fn field_of_view_modifier(p: &FovPlayer, first_person: bool, effect_scale: f32) -> f32 {
    raw_field_of_view_modifier(p, first_person).map_or(0.1, |raw| lerp(effect_scale, 1.0, raw))
}

/// The fluid `Camera.getFluidInCamera` reports, as the field of view sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CameraFluid {
    #[default]
    None,
    Water,
    Lava,
    PowderSnow,
}

/// `Camera.NearPlane`: the near plane's centre and its half extents toward
/// the camera's left and up, relative to the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NearPlane {
    pub forward: [f64; 3],
    pub left: [f64; 3],
    pub up: [f64; 3],
}
impl NearPlane {
    /// `forward`, `getTopLeft`, `getTopRight`, `getBottomLeft`, `getBottomRight`:
    /// the points `Camera.getFluidInCamera` looks for lava and powder snow at.
    pub fn points(&self) -> [[f64; 3]; 5] {
        let corner = |up: f64, left: f64| -> [f64; 3] {
            std::array::from_fn(|i| self.forward[i] + self.up[i] * up + self.left[i] * left)
        };
        [
            self.forward,
            corner(1.0, 1.0),
            corner(1.0, -1.0),
            corner(-1.0, 1.0),
            corner(-1.0, -1.0),
        ]
    }
    /// `getPointOnPlane(x, y)`: `forward + up * y - left * x`.
    pub fn point_on_plane(&self, x: f32, y: f32) -> [f64; 3] {
        std::array::from_fn(|i| {
            self.forward[i] + self.up[i] * f64::from(y) - self.left[i] * f64::from(x)
        })
    }
}

/// `Camera`: the eye-height and field-of-view easing and the view placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub initialized: bool,
    pub eye_height: f32,
    pub eye_height_old: f32,
    pub fov_modifier: f32,
    pub old_fov_modifier: f32,
    pub position: [f64; 3],
    /// Where the camera was placed before a mod's `camera_position` overrides: streaming and
    /// the level-entry readiness follow this, never the render-only `position`.
    pub placed_position: [f64; 3],
    pub x_rot: f32,
    pub y_rot: f32,
    pub roll: f32,
    pub rotation: Quat,
    pub forwards: [f32; 3],
    pub up: [f32; 3],
    pub left: [f32; 3],
    pub detached: bool,
    /// The entity's eye and view rotation (yaw, pitch), where a detached camera backs off
    /// from.
    pub eye: [f64; 3],
    pub eye_rotation: [f32; 2],
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            initialized: false,
            eye_height: 0.0,
            eye_height_old: 0.0,
            fov_modifier: 0.0,
            old_fov_modifier: 0.0,
            position: [0.0; 3],
            placed_position: [0.0; 3],
            x_rot: 0.0,
            y_rot: 0.0,
            roll: 0.0,
            rotation: Quat::IDENTITY,
            forwards: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            left: [-1.0, 0.0, 0.0],
            detached: false,
            eye: [0.0; 3],
            eye_rotation: [0.0; 2],
        }
    }
}
impl Camera {
    /// `Camera.tick`: the eye height eases halfway toward the entity's
    /// (with a level and an entity), then `tickFov` with the camera entity's
    /// field-of-view modifier target (1 when it is not a player).
    pub fn tick(&mut self, entity_eye_height: Option<f32>, target_fov_modifier: f32) {
        if let Some(eye) = entity_eye_height {
            self.eye_height_old = self.eye_height;
            self.eye_height += (eye - self.eye_height) * 0.5;
        }
        self.old_fov_modifier = self.fov_modifier;
        self.fov_modifier += (target_fov_modifier - self.fov_modifier) * 0.5;
        self.fov_modifier = clamp(self.fov_modifier, 0.1, 1.5);
    }
    /// `Camera.setRotation`.
    pub fn set_rotation(&mut self, y_rot: f32, x_rot: f32) {
        self.set_rotation_with_roll(y_rot, x_rot, 0.0);
    }
    /// `Camera.setRotation` with NeoForge's `roll` (in degrees; `rotationYXZ(PI - yaw, -pitch, -roll)`).
    pub fn set_rotation_with_roll(&mut self, y_rot: f32, x_rot: f32, roll: f32) {
        self.x_rot = x_rot;
        self.y_rot = y_rot;
        self.roll = roll;
        self.rotation = rotation_with_roll(y_rot, x_rot, roll);
        self.forwards = self.rotation.transform([0.0, 0.0, -1.0]);
        self.up = self.rotation.transform([0.0, 1.0, 0.0]);
        self.left = self.rotation.transform([-1.0, 0.0, 0.0]);
    }
    /// `Camera.getNearPlane(fov)` for a `width` x `height` window, at the
    /// projection's `zNear` of 0.05.
    pub fn near_plane(&self, fov: f32, width: f32, height: f32) -> NearPlane {
        let z_near = f64::from(0.05f32);
        let aspect = f64::from(width) / f64::from(height);
        let plane_height = sunrise_core::jmath::tan(f64::from(fov * DEG_TO_RAD) / 2.0) * z_near;
        let scale = |v: [f32; 3], by: f64| v.map(|c| f64::from(c) * by);
        NearPlane {
            forward: scale(self.forwards, z_near),
            left: scale(self.left, plane_height * aspect),
            up: scale(self.up, plane_height),
        }
    }
    /// `Camera.move(forwards, up, right)`.
    pub fn move_by(&mut self, forwards: f32, up: f32, right: f32) {
        let offset = self.rotation.transform([right, up, -forwards]);
        self.position = [
            self.position[0] + f64::from(offset[0]),
            self.position[1] + f64::from(offset[1]),
            self.position[2] + f64::from(offset[2]),
        ];
    }
    /// The entity's interpolated eye and look, independent of the final render camera.
    pub fn align_eye(&mut self, entity: &CameraEntity, partial: f32) {
        let p = f64::from(partial);
        self.eye = [
            lerp_d(p, entity.previous[0], entity.position[0]),
            lerp_d(p, entity.previous[1], entity.position[1])
                + f64::from(lerp(partial, self.eye_height_old, self.eye_height)),
            lerp_d(p, entity.previous[2], entity.position[2]),
        ];
        self.eye_rotation = [entity.view_y_rot(partial), entity.view_x_rot(partial)];
    }

    /// `Camera.alignWithEntity` (not riding a minecart with the new
    /// movement): the entity's view rotation and interpolated eye, then a
    /// detached camera backed off along the view (`getMaxZoom`) or a sleeping
    /// one turned to the bed. `clip` is `Level.clip` with
    /// `ClipContext.Block.VISUAL` and no fluids; it returns the hit location
    /// unless the ray misses.
    pub fn align_with_entity(
        &mut self,
        entity: &CameraEntity,
        partial: f32,
        camera_type: CameraType,
        clip: impl FnMut([f64; 3], [f64; 3]) -> Option<[f64; 3]>,
    ) {
        self.align_with_entity_distance(entity, partial, camera_type, |distance| distance, clip);
    }
    /// Adjust the detached distance after entity/mount scaling, before collision clipping.
    /// Only finite overrides in 0..=1024 blocks are accepted (within the game's ray budget).
    /// First-person views do not call `adjust_distance`.
    pub fn align_with_entity_distance(
        &mut self,
        entity: &CameraEntity,
        partial: f32,
        camera_type: CameraType,
        adjust_distance: impl FnOnce(f32) -> f32,
        clip: impl FnMut([f64; 3], [f64; 3]) -> Option<[f64; 3]>,
    ) {
        self.align_with_entity_angles(
            entity,
            partial,
            camera_type,
            (entity.view_y_rot(partial), entity.view_x_rot(partial), 0.0),
            adjust_distance,
            clip,
        );
    }
    /// `Camera.alignWithEntity` with angles `(yaw, pitch, roll)` supplied at NeoForge's
    /// `ViewportEvent.ComputeCameraAngles` moment — before `setRotation`, third-person mirroring
    /// (`yaw + 180`, `-pitch`, `-roll`), detached back-off, and sleeping bed rotation.
    pub fn align_with_entity_angles(
        &mut self,
        entity: &CameraEntity,
        partial: f32,
        camera_type: CameraType,
        (yaw, pitch, roll): (f32, f32, f32),
        adjust_distance: impl FnOnce(f32) -> f32,
        mut clip: impl FnMut([f64; 3], [f64; 3]) -> Option<[f64; 3]>,
    ) {
        self.set_rotation_with_roll(yaw, pitch, roll);
        self.align_eye(entity, partial);
        self.position = self.eye;
        self.detached = !camera_type.is_first_person();
        if self.detached {
            if camera_type.is_mirrored() {
                self.set_rotation_with_roll(self.y_rot + 180.0, -self.x_rot, -self.roll);
            }
            let (mut distance, mut scale) = (4.0f32, 1.0f32);
            if let Some(living) = &entity.living {
                scale = living.scale;
                distance = living.camera_distance as f32;
            }
            let (mut mount_scale, mut mount_distance) = (scale, distance);
            if let Some((s, d)) = entity.mount {
                mount_scale = s;
                mount_distance = d as f32;
            }
            let vanilla = java_max(scale * distance, mount_scale * mount_distance);
            let adjusted = adjust_distance(vanilla);
            let distance = if adjusted.is_finite() && (0.0..=1024.0).contains(&adjusted) {
                adjusted
            } else {
                vanilla
            };
            let zoom = self.max_zoom(distance, &mut clip);
            self.move_by(-zoom, 0.0, 0.0);
        } else if let Some(LivingCamera {
            sleeping: Some(bed),
            ..
        }) = entity.living
        {
            self.set_rotation(bed.map_or(0.0, |yaw| yaw - 180.0), 0.0);
            self.move_by(0.0, 0.15, 0.0);
        }
        self.initialized = true;
    }
    /// `Camera.getMaxZoom`: eight rays from the corners of a 0.2 cube about
    /// the eye, shortening the distance to the nearest visual-shape hit.
    pub fn max_zoom(
        &self,
        mut distance: f32,
        clip: &mut impl FnMut([f64; 3], [f64; 3]) -> Option<[f64; 3]>,
    ) -> f32 {
        let at = self.position;
        for i in 0..8 {
            let offset = [
                ((i & 1) * 2 - 1) as f32,
                ((i >> 1 & 1) * 2 - 1) as f32,
                ((i >> 2 & 1) * 2 - 1) as f32,
            ];
            let from: [f64; 3] = std::array::from_fn(|a| at[a] + f64::from(offset[a] * 0.1));
            let back = f64::from(-distance);
            let to: [f64; 3] =
                std::array::from_fn(|a| from[a] + f64::from(self.forwards[a]) * back);
            if let Some(hit) = clip(from, to) {
                let d = [hit[0] - at[0], hit[1] - at[1], hit[2] - at[2]];
                let dist_sq = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) as f32;
                if dist_sq < distance * distance {
                    distance = dist_sq.sqrt();
                }
            }
        }
        distance
    }
    /// `Camera.calculateFov`: the option scaled by the eased modifier, then
    /// the death and fluid effects.
    pub fn calculate_fov(
        &self,
        partial: f32,
        fov_option: i32,
        effect_scale: f32,
        dead: Option<i32>,
        fluid: CameraFluid,
        panoramic: bool,
    ) -> f32 {
        if panoramic {
            return 90.0;
        }
        let fov = fov_option as f32 * lerp(partial, self.old_fov_modifier, self.fov_modifier);
        modify_fov_based_on_death_or_fluid(partial, fov, effect_scale, dead, fluid)
    }
    /// `Camera.calculateHudFov`.
    pub fn calculate_hud_fov(
        &self,
        partial: f32,
        effect_scale: f32,
        dead: Option<i32>,
        fluid: CameraFluid,
    ) -> f32 {
        modify_fov_based_on_death_or_fluid(partial, 70.0, effect_scale, dead, fluid)
    }
}
/// `Camera.update`: the far plane covers four times the render distance and
/// the whole cloud range (`depthFar`).
pub fn depth_far(render_distance_chunks: i32, cloud_range_chunks: i32) -> f32 {
    (render_distance_chunks as f32 * 16.0 * 4.0).max((cloud_range_chunks * 16) as f32)
}

/// `Projection.getMatrix` for a perspective set up by `setupPerspective`:
/// JOML's `Matrix4f.setPerspective(fov * DEG_TO_RAD, width / height, near,
/// far, zZeroToOne)`, column-major. Vanilla's `Projection` passes its planes
/// reversed (`near` is `zFar`) for reversed depth.
pub fn perspective(
    fov_degrees: f32,
    width: f32,
    height: f32,
    near: f32,
    far: f32,
    zero_to_one: bool,
) -> [f32; 16] {
    let h = sunrise_core::jmath::tan(f64::from(fov_degrees * DEG_TO_RAD * 0.5)) as f32;
    let aspect = width / height;
    let mut m = [0.0f32; 16];
    m[0] = 1.0 / (h * aspect);
    m[5] = 1.0 / h;
    let e = 1.0e-6f32;
    if far > 0.0 && far.is_infinite() {
        m[10] = e - 1.0;
        m[14] = (e - if zero_to_one { 1.0 } else { 2.0 }) * near;
    } else if near > 0.0 && near.is_infinite() {
        m[10] = (if zero_to_one { 0.0 } else { 1.0 }) - e;
        m[14] = ((if zero_to_one { 1.0 } else { 2.0 }) - e) * far;
    } else {
        m[10] = (if zero_to_one { far } else { far + near }) / (near - far);
        m[14] = (if zero_to_one { far } else { far + far }) * near / (near - far);
    }
    m[11] = -1.0;
    m
}
/// `Frustum.getNearPlaneBounds`: the box around the four corners of the
/// culling frustum's NDC `z = -1` plane, relative to the camera (the caller
/// moves it to the eye). The culling projection is `Camera.update`'s
/// `setPerspective(fov, aspect, 0.05, depthFar, zZeroToOne)`.
pub fn near_plane_bounds(
    fov_degrees: f32,
    width: f32,
    height: f32,
    yaw: f32,
    pitch: f32,
    far: f32,
) -> ([f64; 3], [f64; 3]) {
    let projection =
        glam::Mat4::from_cols_array(&perspective(fov_degrees, width, height, 0.05, far, true));
    // Frustum.calculateFrustum: projection * modelView.
    let clip_to_world = (projection * view_matrix(yaw, pitch)).inverse();
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let p = clip_to_world * glam::Vec4::new(x, y, -1.0, 1.0);
        let p = [p.x / p.w, p.y / p.w, p.z / p.w];
        for k in 0..3 {
            min[k] = min[k].min(f64::from(p[k]));
            max[k] = max[k].max(f64::from(p[k]));
        }
    }
    (min, max)
}
/// `GameRenderer.render3dHud`: the first-person hand and held items are
/// drawn with their own projection at `hudFov` (70 degrees, changed only by
/// death and fluids), never the FOV option or the sprint, speed and bow
/// modifier.
pub fn hand_projection(hud_fov: f32, width: f32, height: f32, far: f32) -> [f32; 16] {
    perspective(hud_fov, width, height, far, 0.05, true)
}
/// `Camera.modifyFovBasedOnDeathOrFluid`. `dead` is the camera entity's
/// `deathTime` while it is a living entity that is dead or dying.
pub fn modify_fov_based_on_death_or_fluid(
    partial: f32,
    mut fov: f32,
    effect_scale: f32,
    dead: Option<i32>,
    fluid: CameraFluid,
) -> f32 {
    if let Some(death_time) = dead {
        let duration = java_min(death_time as f32 + partial, 20.0);
        fov /= (1.0 - 500.0 / (duration + 500.0)) * 2.0 + 1.0;
    }
    if matches!(fluid, CameraFluid::Lava | CameraFluid::Water) {
        fov *= lerp(effect_scale, 1.0, 0.857_142_87);
    }
    fov
}

/// `ClientAvatarState`: the walked distance, view bob and cloak of a player.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AvatarBob {
    pub walk_dist: f32,
    pub walk_dist_o: f32,
    pub bob: f32,
    pub bob_o: f32,
    /// `xCloak`, `yCloak`, `zCloak`: a point trailing the player, which the cape leans from.
    pub cloak: [f64; 3],
    pub cloak_o: [f64; 3],
}
impl AvatarBob {
    /// `ClientAvatarState.tick` (at the start of `AbstractClientPlayer.tick`)
    /// with the player's position then: `moveCloak` eases the cloak a quarter
    /// of the way there, or puts it there from 10 blocks away.
    pub fn tick(&mut self, position: [f64; 3]) {
        self.walk_dist_o = self.walk_dist;
        self.cloak_o = self.cloak;
        for i in 0..3 {
            let d = position[i] - self.cloak[i];
            if d > 10.0 || d < -10.0 {
                self.cloak[i] = position[i];
                self.cloak_o[i] = position[i];
            } else {
                self.cloak[i] += d * 0.25;
            }
        }
    }
    /// `AvatarRenderer.extractCapeState`: the cape's `capeFlap`, `capeLean`
    /// and `capeLean2` (degrees) for the player at `position` (interpolated)
    /// with its body turned `body_yaw` and its `fallFlyingScale`.
    pub fn cape(
        &self,
        partial: f32,
        position: [f64; 3],
        body_yaw: f32,
        fall_flying_scale: f32,
    ) -> [f32; 3] {
        let d: [f64; 3] = std::array::from_fn(|i| {
            lerp_d(f64::from(partial), self.cloak_o[i], self.cloak[i]) - position[i]
        });
        let rad = f64::from(body_yaw * DEG_TO_RAD);
        let (sin, cos) = (f64::from(trig::sin(rad)), f64::from(-trig::cos(rad)));
        let mut flap = (d[1] as f32 * 10.0).clamp(-6.0, 32.0);
        let lean = ((d[0] * sin + d[2] * cos) as f32 * 100.0 * (1.0 - fall_flying_scale))
            .clamp(0.0, 150.0);
        let lean2 = ((d[0] * cos - d[2] * sin) as f32 * 100.0).clamp(-20.0, 20.0);
        let bob = self.interpolated_bob(partial);
        let walk = lerp(partial, self.walk_dist_o, self.walk_dist);
        flap += trig::sin(f64::from(walk * 6.0)) * 32.0 * bob;
        [flap, lean, lean2]
    }
    /// `LocalPlayer.move`: `addWalkedDistance(Mth.length(dx, dz) * 0.6F)`
    /// with the float horizontal displacement.
    pub fn add_walked(&mut self, dx: f32, dz: f32) {
        self.walk_dist += (dx * dx + dz * dz).sqrt() * 0.6;
    }
    /// `AbstractClientPlayer.updateBob`: toward the horizontal speed (at most
    /// 0.1) on the ground, toward 0 otherwise.
    pub fn update_bob(&mut self, on_ground_alive_not_swimming: bool, velocity: [f64; 3]) {
        let target = if on_ground_alive_not_swimming {
            java_min(
                0.1,
                (velocity[0] * velocity[0] + velocity[2] * velocity[2]).sqrt() as f32,
            )
        } else {
            0.0
        };
        self.bob_o = self.bob;
        self.bob += (target - self.bob) * 0.4;
    }
    /// `ClientAvatarState.resetBob` (`AbstractClientPlayer.rideTick`).
    pub fn reset_bob(&mut self) {
        self.bob_o = self.bob;
        self.bob = 0.0;
    }
    pub fn interpolated_bob(&self, partial: f32) -> f32 {
        lerp(partial, self.bob_o, self.bob)
    }
    pub fn backwards_interpolated_walk_distance(&self, partial: f32) -> f32 {
        let wda = self.walk_dist - self.walk_dist_o;
        -(self.walk_dist + wda * partial)
    }
}

/// The parts of `CameraRenderState.entityRenderState` the bobbing reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BobState {
    /// A living camera entity: `hurtTime - partial`, `hurtDuration`,
    /// `getHurtDir()`, and `deathTime + partial` when dead or dying.
    pub living: Option<HurtBob>,
    /// A player camera entity: the backwards-interpolated walk distance and
    /// the interpolated bob.
    pub player: Option<(f32, f32)>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HurtBob {
    pub hurt_time: f32,
    pub hurt_duration: i32,
    pub hurt_dir: f32,
    pub death_time: Option<f32>,
}
/// `PoseStack.rotateDegrees` on the pose with an `Axis` (0 `XP`, 1 `YP`, 2 `ZP`).
pub fn rotate_degrees(m: &mut Mat4, axis: usize, degrees: f32) {
    let ang = degrees * DEG_TO_RAD;
    match axis {
        0 => m.rotate_x(ang),
        1 => m.rotate_y(ang),
        _ => m.rotate_z(ang),
    }
}
/// `GameRenderer.bobHurt`.
pub fn bob_hurt(state: &BobState, damage_tilt_strength: f64, pose: &mut Mat4) {
    let Some(living) = state.living else { return };
    let mut hurt = living.hurt_time;
    if let Some(death_time) = living.death_time {
        let duration = java_min(death_time, 20.0);
        rotate_degrees(pose, 2, 40.0 - 8000.0 / (duration + 200.0));
    }
    if hurt < 0.0 {
        return;
    }
    hurt /= living.hurt_duration as f32;
    hurt = trig::sin(f64::from(hurt * hurt * hurt * hurt * PI));
    let rr = living.hurt_dir;
    rotate_degrees(pose, 1, -rr);
    let tilt = (f64::from(-hurt) * 14.0 * damage_tilt_strength) as f32;
    rotate_degrees(pose, 2, tilt);
    rotate_degrees(pose, 1, rr);
}
/// `GameRenderer.bobView`.
pub fn bob_view(state: &BobState, pose: &mut Mat4) {
    let Some((walk, bob)) = state.player else {
        return;
    };
    pose.translate(
        trig::sin(f64::from(walk * PI)) * bob * 0.5,
        -(trig::cos(f64::from(walk * PI)) * bob).abs(),
        0.0,
    );
    rotate_degrees(pose, 2, trig::sin(f64::from(walk * PI)) * bob * 3.0);
    rotate_degrees(
        pose,
        0,
        (trig::cos(f64::from(walk * PI - 0.2)) * bob).abs() * 5.0,
    );
}
/// The bob matrix `GameRenderer.renderLevel` multiplies into the projection
/// (and `renderItemInHand` into the hand's pose).
pub fn bob_matrix(state: &BobState, damage_tilt_strength: f64, bob_view_option: bool) -> Mat4 {
    let mut pose = Mat4::IDENTITY;
    bob_hurt(state, damage_tilt_strength, &mut pose);
    if bob_view_option {
        bob_view(state, &mut pose);
    }
    pose
}

/// The spinning effect `GameRenderer.renderLevel` multiplies into the level's
/// projection after the bob: a skew about a tilted axis, from a portal's or
/// Nausea's intensity (already scaled by Distortion Effects) and the angle in
/// degrees. The first-person hand keeps its own projection without it.
pub fn spinning_effect_matrix(intensity: f32, angle_degrees: f32) -> glam::Mat4 {
    if intensity <= 0.0 {
        return glam::Mat4::IDENTITY;
    }
    let mut skew = 5.0 / (intensity * intensity + 5.0) - intensity * 0.04;
    skew *= skew;
    let axis = glam::Vec3::new(
        0.0,
        std::f32::consts::SQRT_2 / 2.0,
        std::f32::consts::SQRT_2 / 2.0,
    );
    let angle = angle_degrees * DEG_TO_RAD;
    glam::Mat4::from_axis_angle(axis, angle)
        * glam::Mat4::from_scale(glam::Vec3::new(1.0 / skew, 1.0, 1.0))
        * glam::Mat4::from_axis_angle(axis, -angle)
}

#[cfg(test)]
mod tests {
    /// A synthetic camera at (0, 64, 0) facing south (yaw 0, +Z), 70° FOV, 16:9, near 0.05, far
    /// 512, reversed Z as the game builds it.
    fn project(at: [f64; 3]) -> Option<[f32; 3]> {
        let projection =
            glam::Mat4::perspective_rh(70f32.to_radians(), 16.0 / 9.0, 512.0, 0.05).to_cols_array();
        super::project_point(&projection, [0.0, 64.0, 0.0], 0.0, 0.0, at)
    }

    #[test]
    fn projected_points_ahead_land_on_screen_and_behind_or_nan_are_none() {
        let ahead = project([0.0, 64.0, 10.0]).unwrap();
        assert!(ahead[0].abs() < 1e-6 && ahead[1].abs() < 1e-6, "{ahead:?}");
        // Reversed Z: depth near / distance, 0.05 / 10.
        assert!((ahead[2] - 0.005).abs() < 1e-4, "{ahead:?}");
        // Up and to the camera's right (west, -X, when facing south) at the edge of the view.
        let corner = (35f64.to_radians()).tan() * 10.0;
        let edge = project([-corner * 16.0 / 9.0, 64.0 + corner, 10.0]).unwrap();
        assert!(
            (edge[0] - 1.0).abs() < 1e-4 && (edge[1] - 1.0).abs() < 1e-4,
            "{edge:?}"
        );
        let near = project([0.0, 64.0, 0.05]).unwrap();
        assert!((near[2] - 1.0).abs() < 1e-5, "{near:?}");
        assert_eq!(project([f64::MAX, 64.0, 10.0]), None);
        assert_eq!(project([0.0, 64.0, -10.0]), None);
        assert_eq!(project([0.0, 64.0, 0.0]), None);
        assert_eq!(project([f64::NAN, 64.0, 10.0]), None);
        assert_eq!(project([0.0, f64::INFINITY, 10.0]), None);
    }
    #[test]
    fn near_plane_bounds_sit_half_a_near_distance_ahead_of_the_eye() {
        // Facing +Z (yaw 0): the zero-to-one culling projection puts NDC
        // z = -1 at -far * near / (2 far - near) in front of the camera.
        let far = 512.0;
        let (min, max) = super::near_plane_bounds(70.0, 1600.0, 900.0, 0.0, 0.0, far);
        let depth = f64::from(far * 0.05 / (2.0 * far - 0.05));
        assert!((min[2] - depth).abs() < 1e-4 && (max[2] - depth).abs() < 1e-4);
        let half_height = depth * 35.0f64.to_radians().tan();
        assert!((max[1] - half_height).abs() < 1e-4 && (min[1] + half_height).abs() < 1e-4);
        let half_width = half_height * 1600.0 / 900.0;
        assert!((max[0] - half_width).abs() < 1e-3 && (min[0] + half_width).abs() < 1e-3);
    }
    /// `moveCloak` trails a quarter of the way, jumps from 10 blocks away; the
    /// cape leans back as the player walks forward (south, body yaw 0).
    #[test]
    fn the_cloak_trails_the_player() {
        let mut a = super::AvatarBob::default();
        a.tick([0.0, 64.0, 0.0]);
        assert_eq!((a.cloak, a.cloak_o), ([0.0, 64.0, 0.0], [0.0, 64.0, 0.0]));
        a.tick([0.0, 64.0, 1.0]);
        assert_eq!(a.cloak, [0.0, 64.0, 0.25]);
        let [flap, lean, lean2] = a.cape(1.0, [0.0, 64.0, 1.0], 0.0, 0.0);
        assert_eq!((flap, lean, lean2), (0.0, 75.0, 0.0));
        // Falling lifts the flap, at most 32 degrees.
        assert_eq!(a.cape(1.0, [0.0, 60.0, 0.25], 0.0, 0.0)[0], 32.0);
        // Gliding takes the lean away.
        assert_eq!(a.cape(1.0, [0.0, 64.0, 1.0], 0.0, 1.0)[1], 0.0);
    }

    #[test]
    fn depth_far_covers_the_cloud_range() {
        // renderDistance 2 with the default Cloud Distance (128 chunks).
        assert_eq!(super::depth_far(2, 128), 2048.0);
        assert_eq!(super::depth_far(32, 2), 2048.0);
        assert_eq!(super::depth_far(12, 12), 768.0);
    }
    use super::*;
    #[test]
    fn view_keeps_yaw_at_vertical_pitch_limits() {
        for yaw in [0.0, 45.0, 90.0, 180.0, -90.0, 360.0] {
            for pitch in [-90.0, -89.999, 0.0, 89.999, 90.0] {
                let view = view_matrix(yaw, pitch);
                let mut camera = Camera::default();
                camera.set_rotation(yaw, pitch);
                for (world, expected) in [
                    (camera.forwards, -glam::Vec3::Z),
                    (camera.up, glam::Vec3::Y),
                    (camera.left, -glam::Vec3::X),
                ] {
                    let got = view.transform_vector3(glam::Vec3::from(world));
                    assert!(
                        (got - expected).length() < 1e-5,
                        "yaw={yaw}, pitch={pitch}: {got:?} != {expected:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn camera_eases_eye_height_and_fov() {
        let mut camera = Camera::default();
        camera.tick(Some(1.62), 1.0);
        assert_eq!(camera.eye_height, 0.81);
        assert_eq!(camera.fov_modifier, 0.5);
        camera.tick(None, 1.0);
        assert_eq!(camera.eye_height, 0.81);
        assert_eq!(camera.fov_modifier, 0.75);
    }
    #[test]
    fn rotation_faces_south_at_zero_yaw() {
        let mut camera = Camera::default();
        camera.set_rotation(0.0, 0.0);
        assert!((camera.forwards[2] - 1.0).abs() < 1e-6);
        camera.set_rotation(90.0, 0.0);
        assert!((camera.forwards[0] + 1.0).abs() < 1e-6);
    }
    #[test]
    fn detached_camera_backs_off_until_blocked() {
        let entity = CameraEntity {
            position: [0.0; 3],
            previous: [0.0; 3],
            x_rot: 0.0,
            y_rot: 0.0,
            x_rot_o: 0.0,
            y_rot_o: 0.0,
            eye_height: 1.62,
            living: None,
            mount: None,
        };
        let mut camera = Camera::default();
        camera.align_with_entity(&entity, 1.0, CameraType::ThirdPersonBack, |_, _| None);
        assert!((camera.position[2] + 4.0).abs() < 1e-6);
        camera.align_with_entity(&entity, 1.0, CameraType::ThirdPersonBack, |_, _| {
            Some([0.0, 0.0, -1.0])
        });
        assert!(camera.position[2] > -1.1 && camera.position[2] < -0.9);
    }
    #[test]
    fn detached_distance_override_keeps_collision_clipping() {
        let entity = CameraEntity {
            position: [0.0; 3],
            previous: [0.0; 3],
            x_rot: 0.0,
            y_rot: 0.0,
            x_rot_o: 0.0,
            y_rot_o: 0.0,
            eye_height: 1.62,
            living: Some(LivingCamera {
                scale: 2.0,
                camera_distance: 3.0,
                sleeping: None,
                dead: None,
            }),
            mount: Some((3.0, 4.0)),
        };
        for kind in [CameraType::ThirdPersonBack, CameraType::ThirdPersonFront] {
            let mut camera = Camera::default();
            let mut calls = 0;
            let mut rays = 0;
            let z = if kind.is_mirrored() { 1.0 } else { -1.0 };
            camera.align_with_entity_distance(
                &entity,
                1.0,
                kind,
                |distance| {
                    assert_eq!(distance, 12.0);
                    8.0
                },
                |_, _| None,
            );
            assert!((camera.position[2] - z * 8.0).abs() < 1e-5);
            camera.align_with_entity_distance(
                &entity,
                1.0,
                kind,
                |distance| {
                    calls += 1;
                    assert_eq!(distance, 12.0);
                    8.0
                },
                |_, _| {
                    rays += 1;
                    Some([0.0, 0.0, z])
                },
            );
            assert_eq!((calls, rays), (1, 8));
            assert!((camera.position[2] - z).abs() < 0.2);
        }
    }
    #[test]
    fn detached_distance_rejects_invalid_values_and_accepts_zero() {
        let entity = CameraEntity {
            position: [0.0; 3],
            previous: [0.0; 3],
            x_rot: 0.0,
            y_rot: 0.0,
            x_rot_o: 0.0,
            y_rot_o: 0.0,
            eye_height: 1.62,
            living: None,
            mount: None,
        };
        for distance in [f32::NAN, f32::INFINITY, -1.0, 1025.0] {
            let mut camera = Camera::default();
            camera.align_with_entity_distance(
                &entity,
                1.0,
                CameraType::ThirdPersonBack,
                |_| distance,
                |_, _| None,
            );
            assert!((camera.position[2] + 4.0).abs() < 1e-6);
        }
        let mut camera = Camera::default();
        camera.align_with_entity_distance(
            &entity,
            1.0,
            CameraType::ThirdPersonBack,
            |_| 0.0,
            |_, _| None,
        );
        assert_eq!(camera.position, camera.eye);
        camera.align_with_entity_distance(
            &entity,
            1.0,
            CameraType::FirstPerson,
            |_| panic!("first person has no detached distance"),
            |_, _| None,
        );
        assert_eq!(camera.position, camera.eye);
    }
    #[test]
    fn align_with_entity_angles_mirrors_roll_in_third_person_front_and_backs_off_along_new_yaw() {
        let entity = CameraEntity {
            position: [0.0; 3],
            previous: [0.0; 3],
            x_rot: 0.0,
            y_rot: 0.0,
            x_rot_o: 0.0,
            y_rot_o: 0.0,
            eye_height: 1.62,
            living: None,
            mount: None,
        };
        let mut camera = Camera::default();
        camera.align_with_entity_angles(
            &entity,
            1.0,
            CameraType::ThirdPersonBack,
            (90.0, 0.0, 15.0),
            |d| d,
            |_, _| None,
        );
        assert_eq!((camera.y_rot, camera.x_rot, camera.roll), (90.0, 0.0, 15.0));
        // Yaw 90 faces -X, so backing off moves +4 along +X.
        assert!((camera.position[0] - 4.0).abs() < 1e-5);
        camera.align_with_entity_angles(
            &entity,
            1.0,
            CameraType::ThirdPersonFront,
            (90.0, 20.0, 15.0),
            |d| d,
            |_, _| None,
        );
        assert_eq!(
            (camera.y_rot, camera.x_rot, camera.roll),
            (270.0, -20.0, -15.0)
        );
        let view = view_matrix_rolled(camera.y_rot, camera.x_rot, camera.roll);
        for (world, expected) in [
            (camera.forwards, -glam::Vec3::Z),
            (camera.up, glam::Vec3::Y),
            (camera.left, -glam::Vec3::X),
        ] {
            let got = view.transform_vector3(glam::Vec3::from(world));
            assert!((got - expected).length() < 1e-5, "{got:?} != {expected:?}");
        }
    }
    #[test]
    fn fov_modifier_combines_speed_flight_and_bow() {
        let mut p = FovPlayer {
            flying: false,
            walking_speed: 0.1,
            movement_speed: 0.13,
            using: None,
        };
        assert_eq!(
            field_of_view_modifier(&p, true, 1.0),
            (0.13f32 / 0.1 + 1.0) / 2.0
        );
        assert_eq!(field_of_view_modifier(&p, true, 0.0), 1.0);
        p.using = Some((UseItem::Spyglass, 3));
        assert_eq!(field_of_view_modifier(&p, true, 1.0), 0.1);
        assert_ne!(field_of_view_modifier(&p, false, 1.0), 0.1);
    }
}
