//! JOML matrix and quaternion math adapted for shader pack uniforms and view matrices.

use std::f32::consts::PI;
const DEG_TO_RAD: f32 = PI / 180.0;

const TAU: f32 = std::f32::consts::TAU;
const HALF_PI: f32 = std::f32::consts::FRAC_PI_2;

/// `Math.sin(float)`: `java.lang.Math.sin` narrowed, as HotSpot computes it on this
/// machine (`sunrise_core::jmath`), not Rust's own `sin`: JOML's view rotation must match
/// vanilla's to the last bit (#8464).
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
    /// An affine matrix of no more specific kind, as JOML's property flags have it.
    pub fn affine(m: glam::Mat4) -> Self {
        Self {
            m: m.to_cols_array(),
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

/// `Camera.getViewRotationMatrix` to the last bit: `Matrix4f.rotation` of the conjugate of
/// `setRotation`'s quaternion, and its JOML `invert()`. `view_matrix` turns the same quaternion
/// into a matrix by glam's formula, which differs in the last bits (1.0 for 0.99999994, 6e-8 for
/// 0).
pub fn view_rotation_matrix(yaw: f32, pitch: f32) -> (glam::Mat4, glam::Mat4) {
    let q = Quat::rotation_yxz(PI - yaw * DEG_TO_RAD, -pitch * DEG_TO_RAD, 0.0);
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
