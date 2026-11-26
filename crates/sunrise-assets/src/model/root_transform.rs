//! NeoForge's root `transform` of a model (`NeoForgeModelProperties.TRANSFORM`), read as
//! `TransformationHelper.Deserializer` reads it and composed into the baking state as
//! `UnbakedElementsHelper.composeRootTransformIntoModelState` does.
use crate::{
    json::*,
    math::{Mat4, cos_from_sin, joml_sin, quaternion_mul, quaternion_xyz},
};
use serde_json::value::RawValue;

/// A model's root transform, in the opposing-corner frame its JSON describes. The model closest
/// to the leaf that names one wins, `"identity"` included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RootTransform(Mat4);

impl RootTransform {
    /// `TransformationHelper.Deserializer.deserialize`: `"identity"`, a 3x4 row-major matrix
    /// (bare, or as an object's only `matrix` key), or an object of `translation`, `rotation`
    /// (or `left_rotation`), `scale`, `right_rotation` (or `post-rotation`) and `origin`, which
    /// defaults to the opposing corner.
    pub(crate) fn decode(raw: &RawValue) -> Result<Self> {
        if raw.get().starts_with('"') {
            return match string(raw)?.as_str() {
                "identity" => Ok(Self(Mat4::IDENTITY)),
                other => Err(error(&format!("TRSR: unknown default string: {other}"))),
            };
        }
        if raw.get().starts_with('[') {
            return Ok(Self(matrix(raw)?));
        }
        let obj = object(raw)?;
        if let Some(raw) = obj.get("matrix") {
            if obj.len() > 1 {
                return Err(error("TRSR: can't combine matrix and other keys"));
            }
            return Ok(Self(matrix(raw)?));
        }
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        let mut take = |key: &str| {
            let raw = obj.get(key).copied()?;
            keys.retain(|k| *k != key);
            Some(raw)
        };
        let translation = take("translation").map(vector::<3>).transpose()?;
        let left = match take("rotation") {
            Some(raw) => Some(raw),
            None => take("left_rotation"),
        }
        .map(rotation)
        .transpose()?;
        let scale = take("scale")
            .map(|raw| {
                if raw.get().starts_with('[') {
                    vector::<3>(raw)
                } else {
                    float(raw).map(|s| [s; 3])
                }
            })
            .transpose()?;
        let right = match take("right_rotation") {
            Some(raw) => Some(raw),
            None => take("post-rotation"),
        }
        .map(rotation)
        .transpose()?;
        let origin = take("origin").map(origin).transpose()?.unwrap_or([1.0; 3]);
        if !keys.is_empty() {
            return Err(error(&format!("TRSR: unexpected keys {}", keys.join(", "))));
        }
        // `Transformation.compose`: translation, left rotation, scale, right rotation.
        let mut m = Mat4::IDENTITY;
        if let Some(translation) = translation {
            m = Mat4::translation(translation);
        }
        if let Some(left) = left {
            m = m.rotate(left);
        }
        if let Some(scale) = scale {
            m = m.scale(scale);
        }
        if let Some(right) = right {
            m = m.rotate(right);
        }
        Ok(Self(apply_origin(&m, origin)))
    }

    /// The matrix the baker composes after a model's state
    /// (`composeRootTransformIntoModelState`): `None` for the identity, which leaves the state as
    /// it was, otherwise the transform moved to the block-centred frame the face baker uses.
    pub fn block_centred(&self) -> Option<Mat4> {
        (!is_identity(&self.0)).then(|| apply_origin(&self.0, [-0.5; 3]))
    }
}

/// `Transformation.isIdentity`: `Matrix4f.equals` against the identity, bit for bit.
fn is_identity(m: &Mat4) -> bool {
    m.0.iter()
        .flatten()
        .zip(Mat4::IDENTITY.0.iter().flatten())
        .all(|(a, b)| a.to_bits() == b.to_bits())
}

/// `ITransformationExtension.applyOrigin`: `T(origin) * m * T(-origin)`.
fn apply_origin(m: &Mat4, origin: [f32; 3]) -> Mat4 {
    if is_identity(m) {
        return Mat4::IDENTITY;
    }
    Mat4::translation(origin)
        .mul(m)
        .mul(&Mat4::translation(origin.map(|v| -v)))
}

/// `parseMatrix`: three rows of four numbers over the identity's last row.
fn matrix(raw: &RawValue) -> Result<Mat4> {
    let rows = array(raw)?;
    if rows.len() != 3 {
        return Err(error("Matrix: expected an array of length 3"));
    }
    let mut values = [
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    for (row, raw) in rows.into_iter().enumerate() {
        values[row * 4..row * 4 + 4].copy_from_slice(&vector::<4>(raw)?);
    }
    Ok(Mat4::from_row_major(values))
}

/// `parseRotation`: a list of single-axis objects multiplied in order, three XYZ degrees, a raw
/// quaternion, or one single-axis object.
fn rotation(raw: &RawValue) -> Result<[f32; 4]> {
    if raw.get().starts_with('{') {
        return axis_rotation(raw);
    }
    let values = array(raw)?;
    let first = values
        .first()
        .ok_or_else(|| error("Rotation: empty array"))?;
    if first.get().starts_with('{') {
        let mut q = [0.0, 0.0, 0.0, 1.0];
        for value in values {
            q = quaternion_mul(q, axis_rotation(value)?);
        }
        return Ok(q);
    }
    if values.len() == 3 {
        // `quatFromXYZ` in degrees: `(float) Math.PI / 180`.
        let factor = std::f32::consts::PI / 180.0;
        let [x, y, z] = vector::<3>(raw)?;
        return Ok(quaternion_xyz(x * factor, y * factor, z * factor));
    }
    vector::<4>(raw)
}

/// `parseAxisRotation`: `{"x": degrees}`, `{"y": ...}` or `{"z": ...}`, as `Axis.XP`, `YP` or
/// `ZP`'s `rotationDegrees`.
fn axis_rotation(raw: &RawValue) -> Result<[f32; 4]> {
    let obj = object(raw)?;
    let mut entries = obj.iter();
    let (Some((axis, degrees)), None) = (entries.next(), entries.next()) else {
        return Err(error("Axis rotation: expected single axis object"));
    };
    let angle = float(degrees)? * (std::f64::consts::PI / 180.0) as f32;
    let half = angle * 0.5;
    let sin = joml_sin(half);
    let cos = cos_from_sin(sin, half);
    match axis.as_str() {
        "x" => Ok([sin, 0.0, 0.0, cos]),
        "y" => Ok([0.0, sin, 0.0, cos]),
        "z" => Ok([0.0, 0.0, sin, cos]),
        _ => Err(error(&format!(
            "Axis rotation: expected single axis key, got: {axis}"
        ))),
    }
}

/// `parseOrigin`: three numbers, or `center`, `corner` or `opposing-corner`.
fn origin(raw: &RawValue) -> Result<[f32; 3]> {
    if raw.get().starts_with('[') {
        return vector::<3>(raw);
    }
    match primitive_string(raw)?.as_str() {
        "center" => Ok([0.5; 3]),
        "corner" => Ok([0.0; 3]),
        "opposing-corner" => Ok([1.0; 3]),
        _ => Err(error(
            "Origin: expected one of 'center', 'corner', 'opposing-corner'",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(json: &str) -> Result<RootTransform> {
        RootTransform::decode(serde_json::from_str(json).unwrap())
    }
    fn point(m: &Mat4, p: [f32; 3]) -> [f32; 3] {
        m.transform_position(p)
    }

    #[test]
    fn identity_and_an_empty_object_leave_the_state_alone() {
        assert_eq!(decode(r#""identity""#).unwrap().block_centred(), None);
        assert_eq!(decode("{}").unwrap().block_centred(), None);
        assert!(decode(r#""flipped""#).is_err());
    }

    /// A translation needs no origin: one block unit up stays one block unit up in the
    /// block-centred frame.
    #[test]
    fn a_translation_moves_every_vertex() {
        let m = decode(r#"{"translation":[0,0.25,0]}"#)
            .unwrap()
            .block_centred()
            .unwrap();
        assert_eq!(point(&m, [-0.5, -0.5, -0.5]), [-0.5, -0.25, -0.5]);
    }

    /// The default origin is the opposing corner (1, 1, 1): a half scale keeps that corner and
    /// pulls the block's own corner (0, 0, 0) to (0.5, 0.5, 0.5); `center` keeps the middle.
    #[test]
    fn scale_is_about_the_origin() {
        let corner = decode(r#"{"scale":0.5}"#).unwrap().block_centred().unwrap();
        // Block-centred: (0,0,0) is (-0.5,-0.5,-0.5) and (1,1,1) is (0.5,0.5,0.5).
        assert_eq!(point(&corner, [-0.5; 3]), [0.0; 3]);
        assert_eq!(point(&corner, [0.5; 3]), [0.5; 3]);
        let centre = decode(r#"{"scale":[0.5,0.5,0.5],"origin":"center"}"#)
            .unwrap()
            .block_centred()
            .unwrap();
        assert_eq!(point(&centre, [0.5; 3]), [0.25; 3]);
        assert_eq!(point(&centre, [0.0; 3]), [0.0; 3]);
    }

    /// A quarter turn about Y, written as an axis object, a list of them, or before or after
    /// the scale, is the same transform, and it turns +X into -Z about the centre.
    #[test]
    fn rotation_spellings_agree() {
        let turn = decode(r#"{"rotation":{"y":90},"origin":"center"}"#).unwrap();
        for json in [
            r#"{"rotation":[{"y":90}],"origin":"center"}"#,
            r#"{"left_rotation":{"y":90},"origin":"center"}"#,
            r#"{"right_rotation":{"y":90},"origin":"center"}"#,
            r#"{"post-rotation":{"y":90},"origin":"center"}"#,
        ] {
            assert_eq!(decode(json).unwrap(), turn, "{json}");
        }
        // Three numbers are XYZ degrees: a quarter turn about Y, the same quaternion bit for bit.
        let xyz = decode(r#"{"rotation":[0,90,0],"origin":"center"}"#).unwrap();
        assert_eq!(xyz, turn);
        // JOML 1.10.9 (`Matrix4f.rotate`, `applyOrigin` twice, `transformPosition`) gives this
        // point for both spellings, bit for bit.
        let m = turn.block_centred().unwrap();
        assert_eq!(point(&m, [0.5, 0.0, 0.0]), [0.0, 0.0, -0.49999994]);
        assert_eq!(
            point(&xyz.block_centred().unwrap(), [0.5, 0.0, 0.0]),
            [0.0, 0.0, -0.49999994]
        );
    }

    #[test]
    fn a_matrix_is_three_rows_of_four() {
        let m = decode("[[1,0,0,0.25],[0,1,0,0],[0,0,1,0]]").unwrap();
        let wrapped = decode(r#"{"matrix":[[1,0,0,0.25],[0,1,0,0],[0,0,1,0]]}"#).unwrap();
        assert_eq!(m, wrapped);
        assert_eq!(
            point(&m.block_centred().unwrap(), [0.0; 3]),
            [0.25, 0.0, 0.0]
        );
        assert!(decode("[[1,0,0,0],[0,1,0,0]]").is_err());
        assert!(decode(r#"{"matrix":[[1,0,0,0],[0,1,0,0],[0,0,1,0]],"scale":2}"#).is_err());
    }

    #[test]
    fn unknown_or_doubled_keys_are_refused() {
        assert!(decode(r#"{"translate":[0,0,0]}"#).is_err());
        assert!(decode(r#"{"rotation":{"x":90},"left_rotation":{"x":90}}"#).is_err());
        assert!(decode(r#"{"right_rotation":{"x":90},"post-rotation":{"x":90}}"#).is_err());
        assert!(decode(r#"{"rotation":{"w":90}}"#).is_err());
        assert!(decode(r#"{"rotation":{"x":90,"y":90}}"#).is_err());
        assert!(decode(r#"{"origin":"middle"}"#).is_err());
    }
}
