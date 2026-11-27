//! Camera-relative frustum and view clipping structures.

use glam::{DVec3, Mat4, Vec3, Vec4};

/// The maximum number of alternate world views supported simultaneously.
pub const MAX_WORLD_VIEWS: usize = 8;

/// Camera-relative clip planes (`Frustum`).
#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    planes: [Vec4; 6],
}

impl Frustum {
    /// Construct a frustum from six camera-relative clip planes.
    #[inline]
    pub const fn from_planes(planes: [Vec4; 6]) -> Self {
        Self { planes }
    }

    /// The six camera-relative clip planes.
    #[inline]
    pub const fn planes(&self) -> &[Vec4; 6] {
        &self.planes
    }

    /// Whether the camera-relative box `min..max` is at least partly inside.
    /// Uses `>= 0.0` (matching JOML `FrustumIntersection.intersectAab`) so a
    /// non-finite plane test (`0.0 * inf = NaN`) rejects rather than passing.
    pub fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        self.planes.iter().all(|p| {
            let sign = p.truncate().cmpge(Vec3::ZERO);
            let v = Vec3::select(sign, max, min);
            p.truncate().dot(v) + p.w >= 0.0
        })
    }

    /// Whether a camera-relative sphere is at least partly inside.
    pub fn intersects_sphere(&self, center: Vec3, radius: f32) -> bool {
        if !center.is_finite() || !radius.is_finite() || radius < 0.0 {
            return true;
        }
        !self
            .planes
            .iter()
            .any(|plane| plane.truncate().dot(center) + plane.w < -radius)
    }

    /// Depth clip planes alone: X/Y projection jitter cannot change this test.
    pub fn intersects_depth(&self, min: Vec3, max: Vec3) -> bool {
        if !min.is_finite() || !max.is_finite() || !min.cmple(max).all() {
            return true;
        }
        !self.planes[4..].iter().any(|p| {
            let sign = p.truncate().cmpge(Vec3::ZERO);
            let v = Vec3::select(sign, max, min);
            p.truncate().dot(v) + p.w < 0.0
        })
    }

    /// Whether the camera-relative box `min..max` is entirely inside.
    pub fn contains(&self, min: Vec3, max: Vec3) -> bool {
        self.planes.iter().all(|p| {
            let sign = p.truncate().cmpge(Vec3::ZERO);
            let v = Vec3::select(sign, min, max);
            p.truncate().dot(v) + p.w >= 0.0
        })
    }

    /// Whether the camera-relative `point` is on or inside every plane.
    pub fn contains_point(&self, point: Vec3) -> bool {
        if !point.is_finite() {
            return true;
        }
        self.planes
            .iter()
            .all(|plane| plane.truncate().dot(point) + plane.w >= 0.0)
    }

    /// The same frustum seen from a camera moved `blocks` back along the view direction.
    pub fn pulled_back(&self, blocks: f32) -> Frustum {
        let sides: Vec3 = self.planes[..4].iter().map(|p| p.truncate()).sum();
        let depth = self.planes[4].truncate();
        let along = depth.dot(sides);
        if !along.is_finite() || along.abs() < 1.0e-3 {
            return *self;
        }
        let toward_sides = if along > 0.0 { depth } else { -depth };
        let axis = toward_sides.normalize_or_zero();
        let mut planes = self.planes;
        for plane in &mut planes {
            plane.w += blocks * plane.truncate().dot(axis);
        }
        Self::from_planes(planes)
    }

    /// Vanilla's `Frustum.offsetToFullyIncludeCameraCube`: pull back in 4-block steps until the
    /// camera's aligned `cube_size` cube is wholly inside the frustum.
    pub fn offset_to_fully_include_camera_cube(&self, camera: DVec3, cube_size: i32) -> Frustum {
        if cube_size <= 0 || !camera.is_finite() || !self.planes.iter().all(|p| p.is_finite()) {
            return *self;
        }
        let sides: Vec3 = self.planes[..4].iter().map(|p| p.truncate()).sum();
        let depth = self.planes[4].truncate();
        let along = depth.dot(sides);
        if !along.is_finite() || along.abs() < 1.0e-3 {
            return *self;
        }
        let size = f64::from(cube_size);
        let min = (DVec3::new(
            (camera.x / size).floor() * size,
            (camera.y / size).floor() * size,
            (camera.z / size).floor() * size,
        ) - camera)
            .as_vec3();
        let max = (DVec3::new(
            (camera.x / size).ceil() * size,
            (camera.y / size).ceil() * size,
            (camera.z / size).ceil() * size,
        ) - camera)
            .as_vec3();
        let mut out = *self;
        for _ in 0..256 {
            if out.contains(min, max) {
                break;
            }
            out = out.pulled_back(4.0);
        }
        out
    }

    /// Construct a frustum from a view-projection matrix (wgpu depth range 0..1).
    pub fn from_view_projection(view_proj: Mat4) -> Self {
        Self::from_planes(frustum_planes_from_view_proj(view_proj))
    }
}

/// Extract six normalized clip planes from a view-projection matrix (wgpu depth range 0..1).
pub fn frustum_planes_from_view_proj(m: Mat4) -> [Vec4; 6] {
    let r = |i| m.row(i);
    let planes = [
        r(3) + r(0),
        r(3) - r(0),
        r(3) + r(1),
        r(3) - r(1),
        r(2),
        r(3) - r(2),
    ];
    planes.map(|p| p / p.truncate().length())
}

/// The union of two frusta (the camera's and its water mirror's), for visibility testing.
#[derive(Clone, Copy, Debug)]
pub struct Frusta(pub Frustum, pub Option<Frustum>);

/// What visibility tests cull with: a frustum or a union of them.
pub trait Cull {
    fn intersects(&self, min: Vec3, max: Vec3) -> bool;
    fn contains(&self, min: Vec3, max: Vec3) -> bool;
    /// Whether a section is tested by the box around its geometry, not its whole 16 blocks.
    fn tight(&self) -> bool {
        true
    }
    /// Whether the eye's vertical render-distance clamp and section-tree culling apply.
    fn vertical_clamp(&self) -> bool {
        true
    }
    /// Camera-cube-offset frustum for outside-height section-tree culling, if applicable.
    fn offset_for_camera_cube(&self, _camera: DVec3, _cube_size: i32) -> Option<JoinedFrusta> {
        None
    }
}

impl Cull for Frustum {
    fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        Frustum::intersects(self, min, max)
    }
    fn contains(&self, min: Vec3, max: Vec3) -> bool {
        Frustum::contains(self, min, max)
    }
    fn offset_for_camera_cube(&self, camera: DVec3, cube_size: i32) -> Option<JoinedFrusta> {
        Some(JoinedFrusta::from(
            self.offset_to_fully_include_camera_cube(camera, cube_size),
        ))
    }
}

impl Cull for Frusta {
    fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        self.0.intersects(min, max) || self.1.is_some_and(|f| f.intersects(min, max))
    }
    fn contains(&self, min: Vec3, max: Vec3) -> bool {
        self.0.contains(min, max) || self.1.is_some_and(|f| f.contains(min, max))
    }
    fn offset_for_camera_cube(&self, camera: DVec3, cube_size: i32) -> Option<JoinedFrusta> {
        Some(JoinedFrusta::new(
            self.0
                .offset_to_fully_include_camera_cube(camera, cube_size),
            self.1
                .map(|f| f.offset_to_fully_include_camera_cube(camera, cube_size)),
        ))
    }
}

/// The camera's frustum joined to this frame's world views.
#[derive(Clone, Copy, Debug)]
pub struct JoinedFrusta {
    camera: Frustum,
    views: [Frustum; MAX_WORLD_VIEWS],
    len: usize,
}

impl JoinedFrusta {
    /// Join the camera's frustum to the first [`MAX_WORLD_VIEWS`] alternate views.
    pub fn new(camera: Frustum, views: impl IntoIterator<Item = Frustum>) -> Self {
        let mut joined = Self {
            camera,
            views: [camera; MAX_WORLD_VIEWS],
            len: 0,
        };
        for (slot, view) in joined.views.iter_mut().zip(views) {
            *slot = view;
            joined.len += 1;
        }
        joined
    }

    /// The same views with the camera's frustum `camera` (TAA's wider one).
    pub fn with_camera(self, camera: Frustum) -> Self {
        Self { camera, ..self }
    }

    fn views(&self) -> &[Frustum] {
        &self.views[..self.len]
    }

    pub fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        self.camera.intersects(min, max) || self.views().iter().any(|f| f.intersects(min, max))
    }

    pub fn intersects_sphere(&self, center: Vec3, radius: f32) -> bool {
        self.camera.intersects_sphere(center, radius)
            || self
                .views()
                .iter()
                .any(|f| f.intersects_sphere(center, radius))
    }

    pub fn intersects_depth(&self, min: Vec3, max: Vec3) -> bool {
        self.camera.intersects_depth(min, max)
            || self.views().iter().any(|f| f.intersects(min, max))
    }

    pub fn contains(&self, min: Vec3, max: Vec3) -> bool {
        self.camera.contains(min, max) || self.views().iter().any(|f| f.contains(min, max))
    }

    pub fn contains_point(&self, point: Vec3) -> bool {
        self.camera.contains_point(point) || self.views().iter().any(|f| f.contains_point(point))
    }

    pub fn pulled_back(&self, blocks: f32) -> Self {
        Self {
            camera: self.camera.pulled_back(blocks),
            views: self.views.map(|f| f.pulled_back(blocks)),
            len: self.len,
        }
    }

    pub fn offset_to_fully_include_camera_cube(&self, camera: DVec3, cube_size: i32) -> Self {
        Self {
            camera: self
                .camera
                .offset_to_fully_include_camera_cube(camera, cube_size),
            views: self
                .views
                .map(|f| f.offset_to_fully_include_camera_cube(camera, cube_size)),
            len: self.len,
        }
    }
}

impl From<Frustum> for JoinedFrusta {
    fn from(camera: Frustum) -> Self {
        Self::new(camera, [])
    }
}

impl Cull for JoinedFrusta {
    fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        JoinedFrusta::intersects(self, min, max)
    }
    fn contains(&self, min: Vec3, max: Vec3) -> bool {
        JoinedFrusta::contains(self, min, max)
    }
    fn offset_for_camera_cube(&self, camera: DVec3, cube_size: i32) -> Option<JoinedFrusta> {
        Some(self.offset_to_fully_include_camera_cube(camera, cube_size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOV: f32 = 70.0;
    const ASPECT: f32 = 16.0 / 9.0;
    const FAR: f32 = 512.0;
    const EPS: f32 = 0.01;

    fn world_projection(fov_degrees: f32, aspect: f32, far: f32) -> Mat4 {
        Mat4::perspective_rh(fov_degrees.to_radians(), aspect, far, 0.05)
    }

    fn looking_down_minus_z() -> Frustum {
        Frustum::from_view_projection(world_projection(FOV, ASPECT, FAR))
    }

    fn tangents() -> (f32, f32) {
        let up = (FOV / 2.0).to_radians().tan();
        (up * ASPECT, up)
    }

    #[test]
    fn a_point_is_inside_only_within_all_six_planes() {
        let view = looking_down_minus_z();
        let (tan_x, tan_y) = tangents();
        let depth = 10.0;
        let inside = [
            Vec3::new(0.0, 0.0, -depth),
            Vec3::new(depth * tan_x - EPS, 0.0, -depth),
            Vec3::new(-(depth * tan_x - EPS), 0.0, -depth),
            Vec3::new(0.0, depth * tan_y - EPS, -depth),
            Vec3::new(0.0, -(depth * tan_y - EPS), -depth),
            Vec3::new(0.0, 0.0, -0.06),
            Vec3::new(0.0, 0.0, -(FAR - 0.1)),
        ];
        let outside = [
            Vec3::new(depth * tan_x + EPS, 0.0, -depth),
            Vec3::new(-(depth * tan_x + EPS), 0.0, -depth),
            Vec3::new(0.0, depth * tan_y + EPS, -depth),
            Vec3::new(0.0, -(depth * tan_y + EPS), -depth),
            Vec3::new(0.0, 0.0, -0.04),
            Vec3::new(0.0, 0.0, -(FAR + 0.1)),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(0.0, 0.0, 0.0),
        ];
        for p in inside {
            assert!(view.contains_point(p), "{p} should be inside");
        }
        for p in outside {
            assert!(!view.contains_point(p), "{p} should be outside");
        }
        assert!(view.contains_point(Vec3::NAN), "an unknown point is kept");
    }

    #[test]
    fn pulling_the_camera_back_moves_every_plane() {
        let (tan_x, tan_y) = tangents();
        let back = looking_down_minus_z().pulled_back(3.0);
        let inside = [
            Vec3::new(0.0, 0.0, 2.9),
            Vec3::new(3.0 * tan_x - EPS, 0.0, 0.0),
            Vec3::new(-(3.0 * tan_x - EPS), 0.0, 0.0),
            Vec3::new(0.0, 3.0 * tan_y - EPS, 0.0),
            Vec3::new(0.0, -(3.0 * tan_y - EPS), 0.0),
            Vec3::new(0.0, 0.0, 3.0 - FAR + 0.1),
        ];
        let outside = [
            Vec3::new(0.0, 0.0, 3.0),
            Vec3::new(3.0 * tan_x + EPS, 0.0, 0.0),
            Vec3::new(-(3.0 * tan_x + EPS), 0.0, 0.0),
            Vec3::new(0.0, 3.0 * tan_y + EPS, 0.0),
            Vec3::new(0.0, -(3.0 * tan_y + EPS), 0.0),
            Vec3::new(0.0, 0.0, 3.0 - FAR - 0.1),
        ];
        for p in inside {
            assert!(back.contains_point(p), "{p} should be inside");
        }
        for p in outside {
            assert!(!back.contains_point(p), "{p} should be outside");
        }
        assert!(!looking_down_minus_z().contains_point(Vec3::new(0.0, 0.0, 2.9)));
        let same = looking_down_minus_z();
        let unmoved = same.pulled_back(0.0);
        for p in [
            Vec3::new(1.0, 2.0, -3.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(40.0, 0.0, -5.0),
        ] {
            assert_eq!(same.contains_point(p), unmoved.contains_point(p));
        }
    }

    #[test]
    fn the_offset_is_along_the_view_direction() {
        for (yaw, pitch) in [(0.3_f32, 0.0_f32), (2.0, -0.5), (-1.2, 0.9), (3.0, 1.2)] {
            let pose = Mat4::from_rotation_x(pitch) * Mat4::from_rotation_y(yaw);
            let view = Frustum::from_view_projection(world_projection(FOV, ASPECT, FAR) * pose);
            let forward = pose.inverse().transform_vector3(-Vec3::Z);
            let back = view.pulled_back(3.0);
            assert!(
                back.contains_point(forward * -2.9),
                "{yaw} {pitch}: 2.9 behind"
            );
            assert!(
                !back.contains_point(forward * -3.0),
                "{yaw} {pitch}: 3 behind"
            );
            assert!(
                !view.contains_point(forward * -2.9),
                "{yaw} {pitch}: unmoved"
            );
            assert!(back.contains_point(forward * 20.0) && view.contains_point(forward * 20.0));
        }
    }

    #[test]
    fn an_orthographic_frustum_stays_where_it_is() {
        let ortho =
            Frustum::from_view_projection(Mat4::orthographic_rh(-8.0, 8.0, -4.5, 4.5, 100.0, 0.05));
        let moved = ortho.pulled_back(3.0);
        for p in [
            Vec3::new(0.0, 0.0, 2.0),
            Vec3::new(0.0, 0.0, -9.0),
            Vec3::new(7.9, 4.4, -9.0),
            Vec3::new(8.1, 0.0, -9.0),
            Vec3::new(0.0, 0.0, -101.0),
        ] {
            assert_eq!(ortho.contains_point(p), moved.contains_point(p), "{p}");
        }
    }

    #[test]
    fn a_frustum_without_depth_planes_stays_where_it_is() {
        let mut flat = looking_down_minus_z();
        flat.planes[4] = glam::Vec4::W;
        flat.planes[5] = glam::Vec4::W;
        let moved = flat.pulled_back(3.0);
        for p in [
            Vec3::new(0.0, 0.0, 2.0),
            Vec3::new(0.0, 0.0, -9.0),
            Vec3::new(80.0, 0.0, -9.0),
        ] {
            assert_eq!(flat.contains_point(p), moved.contains_point(p));
        }
    }

    #[test]
    fn offset_to_fully_include_camera_cube_contains_aligned_cube() {
        let view = looking_down_minus_z();
        for camera in [
            DVec3::new(0.0, 400.0, 0.0),
            DVec3::new(0.5, 400.5, 0.5),
            DVec3::new(-3.25, 70.1, 11.9),
        ] {
            let offset = view.offset_to_fully_include_camera_cube(camera, 8);
            let min = (DVec3::new(
                (camera.x / 8.0).floor() * 8.0,
                (camera.y / 8.0).floor() * 8.0,
                (camera.z / 8.0).floor() * 8.0,
            ) - camera)
                .as_vec3();
            let max = (DVec3::new(
                (camera.x / 8.0).ceil() * 8.0,
                (camera.y / 8.0).ceil() * 8.0,
                (camera.z / 8.0).ceil() * 8.0,
            ) - camera)
                .as_vec3();
            assert!(
                offset.contains(min, max),
                "camera {camera}: cube {min}..{max} must be inside offset frustum"
            );
        }
        let all = Frustum::from_planes([Vec4::W; 6]);
        let same = all.offset_to_fully_include_camera_cube(DVec3::new(0.5, 400.5, 0.5), 8);
        assert_eq!(same.planes(), all.planes());
    }
}
