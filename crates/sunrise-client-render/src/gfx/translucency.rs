//! Vanilla 26.3 MeshData.SortState / VertexSorting.byDistance.
//! Keep vertex order intact; only the quad indices change as the camera moves.

/// MeshData.decodeQuadCentroids uses the opposite corners, not all four vertices.
pub fn quad_center(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| (a[i] + b[i]) / 2.0)
}

/// The side an axis-aligned quad faces: the terrain pipelines cull back faces, so a quad whose
/// plane the camera is behind draws no pixel. Water's surface has a back face beside its front
/// (`FluidState.shouldRenderBackwardUpFace`), so about half of a lake's quads are such.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Facing {
    axis: usize,
    /// The plane's coordinate on `axis`, section-relative.
    plane: f32,
    /// Whether the front (the counter-clockwise side) looks towards `+axis`.
    positive: bool,
}

/// How far a camera's coordinate can be behind a plane without the quad being culled by float
/// error in the projection.
const PLANE_MARGIN: f32 = 1.0 / 256.0;

impl Facing {
    /// The facing of the quad `corners` (in draw order, as `quad_center`'s), decided as the
    /// rasterizer decides it, per triangle by winding: a quad's triangles are 0 1 2 and 2 3 0,
    /// and each one's front is the side its counter-clockwise winding looks from. Only a quad
    /// whose four corners share one coordinate on an axis and whose two triangles wind the same
    /// way has one facing; any other (a sloped surface, a lopsided or bow-tie quad) is `None` and
    /// always drawn, as one of its triangles may face a camera the other does not.
    pub fn of(corners: &[[f32; 3]; 4]) -> Option<Self> {
        let axis = (0..3).find(|&a| corners.iter().all(|c| c[a] == corners[0][a]))?;
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        // The sign of a triangle's winding along `axis`: its edges' cross product there.
        let winding = |a: usize, b: usize, c: usize| {
            let (e1, e2) = (
                [corners[b][u] - corners[a][u], corners[b][v] - corners[a][v]],
                [corners[c][u] - corners[a][u], corners[c][v] - corners[a][v]],
            );
            e1[0] * e2[1] - e1[1] * e2[0]
        };
        let (first, second) = (winding(0, 1, 2), winding(2, 3, 0));
        (first != 0.0 && (first > 0.0) == (second > 0.0) && second != 0.0).then_some(Self {
            axis,
            plane: corners[0][axis],
            positive: first > 0.0,
        })
    }

    /// Whether a camera anywhere in `reach` (its lowest and highest coordinate on each axis) is
    /// behind the quad, or in its plane: it draws nothing from there.
    fn hidden_from(self, reach: &[[f32; 2]; 3]) -> bool {
        let [low, high] = reach[self.axis];
        if self.positive {
            high <= self.plane - PLANE_MARGIN
        } else {
            low >= self.plane + PLANE_MARGIN
        }
    }
}

#[derive(Default)]
pub struct Sorter {
    order: Vec<(u32, f32)>,
    indices: Vec<u32>,
}

impl Sorter {
    pub fn sort(&mut self, centers: &[[f32; 3]], camera: [f32; 3]) -> &[u32] {
        self.order.clear();
        self.order.extend(centers.iter().enumerate().map(|(i, c)| {
            let [x, y, z] = std::array::from_fn(|j| camera[j] - c[j]);
            (i as u32, x * x + y * y + z * z)
        }));
        // VertexSorting uses a stable merge sort, preserving mesh order for ties.
        self.order.sort_by(|a, b| b.1.total_cmp(&a.1));
        self.indices.clear();
        for &(quad, _) in &self.order {
            let v = quad * 4;
            self.indices.extend([v, v + 1, v + 2, v + 2, v + 3, v]);
        }
        &self.indices
    }
}

impl Sorter {
    /// `sort`'s whole order, then, when some quads face away from every camera in `reach`
    /// (`Facing::hidden_from`), the other quads again in that same order: the quads' count
    /// first. The whole order is what the section draws for any other camera, so it stays in
    /// full and exactly as `sort` left it.
    fn sort_facing(
        &mut self,
        centers: &[[f32; 3]],
        facings: &[Option<Facing>],
        camera: [f32; 3],
        reach: &[[f32; 2]; 3],
    ) -> (&[u32], usize) {
        self.sort(centers, camera);
        let hidden = |quad: u32| facings[quad as usize].is_some_and(|f| f.hidden_from(reach));
        let kept = self
            .order
            .iter()
            .filter(|&&(quad, _)| !hidden(quad))
            .count();
        if kept < self.order.len() {
            for &(quad, _) in &self.order {
                if !hidden(quad) {
                    let v = quad * 4;
                    self.indices.extend([v, v + 1, v + 2, v + 2, v + 3, v]);
                }
            }
        }
        (&self.indices, kept)
    }
}

/// The index words a section of `quads` translucent quads holds in the sorted index buffer: its
/// whole back to front order, then the part of it that faces the camera (`SortState::drawn`).
pub fn index_words(quads: u32) -> u64 {
    u64::from(quads) * 12
}

/// Which side of a section's 16 block cube the camera is on, in each axis: -1 below or before
/// it, 1 above or after it, 0 within its extent (`TranslucencyPointOfView`).
pub fn viewpoint(camera: [f64; 3], origin: [i32; 3]) -> [i32; 3] {
    std::array::from_fn(|i| {
        let block = camera[i].floor() as i32;
        ((block >> 4) - (origin[i] >> 4)).clamp(-1, 1)
    })
}

pub struct SortState {
    centers: Vec<[f32; 3]>,
    facings: Vec<Option<Facing>>,
    /// When the last order has the facing quads after it (`update`'s `cull_back`): the viewpoint
    /// of the eye it was made for (`viewpoint`) and how many quads a camera there draws.
    kept: Option<([i32; 3], usize)>,
    last: Option<([i32; 3], [i32; 3])>,
}

impl SortState {
    pub fn new(centers: Vec<[f32; 3]>) -> Self {
        Self {
            facings: vec![None; centers.len()],
            centers,
            kept: None,
            last: None,
        }
    }

    /// The state of a mesh's translucent quads, each as its four corners.
    pub fn of_quads(quads: impl Iterator<Item = [[f32; 3]; 4]>) -> Self {
        let (centers, facings) = quads
            .map(|q| (quad_center(q[0], q[2]), Facing::of(&q)))
            .unzip();
        Self {
            centers,
            facings,
            kept: None,
            last: None,
        }
    }

    /// Which quads of the section's `total` the camera's draw takes: the first quad of its
    /// index range and the count. The quads that face the eye (in `pov`, its `viewpoint`) come
    /// after the whole order when the last order was made for that viewpoint; otherwise, and
    /// whenever nothing faces away, the whole order draws. A section is re-sorted only now and
    /// then, so an eye that has moved to another viewpoint draws the whole order, which is the
    /// order it was sorted in, until the section is sorted again.
    pub fn drawn(&self, total: u32, cull_back: bool, pov: [i32; 3]) -> (u32, u32) {
        match self.kept {
            Some((made_for, kept)) if cull_back && made_for == pov && (kept as u32) < total => {
                (total, kept as u32)
            }
            _ => (0, total),
        }
    }

    pub fn is_sorted(&self) -> bool {
        self.last.is_some()
    }

    /// Large tiles cannot use the section viewpoint shortcut: the camera may
    /// cross a quad's plane while remaining on the same side of the first 16³.
    pub fn update_lod<'a>(
        &mut self,
        camera: [f64; 3],
        origin: [i32; 3],
        sorter: &'a mut Sorter,
    ) -> Option<&'a [u32]> {
        let block = camera.map(|v| v.floor() as i32);
        if self.last.is_some_and(|(last, _)| last == block) {
            return None;
        }
        self.last = Some((block, [0; 3]));
        self.kept = None;
        Some(sorter.sort(
            &self.centers,
            std::array::from_fn(|i| (camera[i] - f64::from(origin[i])) as f32),
        ))
    }

    /// TranslucencyPointOfView and ResortTransparencyTask: diagonal views keep
    /// their ordering until the point of view changes; aligned views also
    /// resort on crossing a block boundary. Sub-block movement needs no upload.
    ///
    /// The order is by distance from `camera`, as vanilla's. With `cull_back` the indices carry,
    /// after the whole order, the quads of it that face the eye (`drawn`): `eye` is where the
    /// projection looks from, which the view bob moves off `camera`. A quad is left out only when
    /// no eye of this viewpoint could see its front: a quad's plane is on one axis, and the eye
    /// must be off the section on that axis, on the quad's back side.
    pub fn update<'a>(
        &mut self,
        camera: [f64; 3],
        eye: [f64; 3],
        origin: [i32; 3],
        cull_back: bool,
        sorter: &'a mut Sorter,
    ) -> Option<&'a [u32]> {
        let block = camera.map(|v| v.floor() as i32);
        let pov = viewpoint(camera, origin);
        let eye_pov = viewpoint(eye, origin);
        if let Some((last_block, last_pov)) = self.last
            && last_pov == pov
            && (last_block == block || !pov.contains(&0))
            && self.kept.map(|(made_for, _)| made_for) == cull_back.then_some(eye_pov)
        {
            return None;
        }
        self.last = Some((block, pov));
        let relative = std::array::from_fn(|i| (camera[i] - f64::from(origin[i])) as f32);
        if !cull_back {
            self.kept = None;
            return Some(sorter.sort(&self.centers, relative));
        }
        // Every eye of the viewpoint, section-relative: beyond the section's edge on an axis
        // it is off, and on an axis it shares with the section, anywhere in it.
        let reach = eye_pov.map(|side| match side {
            1.. => [16.0, f32::INFINITY],
            0 => [0.0, 16.0],
            _ => [f32::NEG_INFINITY, 0.0],
        });
        let (indices, kept) = sorter.sort_facing(&self.centers, &self.facings, relative, &reach);
        self.kept = Some((eye_pov, kept));
        Some(indices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_distances_keep_mesh_order_and_triangle_winding() {
        let mut sorter = Sorter::default();
        assert_eq!(
            sorter.sort(
                &[[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [4.0, 0.0, 0.0]],
                [0.0; 3]
            ),
            &[8, 9, 10, 10, 11, 8, 0, 1, 2, 2, 3, 0, 4, 5, 6, 6, 7, 4]
        );
        assert!(sorter.sort(&[], [0.0; 3]).is_empty());
    }

    #[test]
    fn movement_reverses_faces_without_moving_vertices() {
        let centers = [[0.5, 0.5, 0.0], [0.5, 0.5, 1.0]];
        let mut sorter = Sorter::default();
        assert_eq!(sorter.sort(&centers, [0.5, 0.5, -2.0])[0], 4);
        assert_eq!(sorter.sort(&centers, [0.5, 0.5, 3.0])[0], 0);
    }

    #[test]
    fn cached_viewpoint_handles_negative_and_large_coordinates() {
        let origin = [-30_000_000, -64, 30_000_000];
        let mut state = SortState::new(vec![[0.5; 3]]);
        let mut sorter = Sorter::default();
        let camera = [-29_999_999.5, -63.5, 30_000_000.5];
        assert!(update_at(&mut state, camera, origin, false, &mut sorter).is_some());
        assert!(update_at(&mut state, camera, origin, false, &mut sorter).is_none());
        assert!(
            update_at(
                &mut state,
                [-29_999_999.1, -63.1, 30_000_000.9],
                origin,
                false,
                &mut sorter
            )
            .is_none()
        );
        assert!(
            update_at(
                &mut state,
                [-29_999_998.9, -63.1, 30_000_000.9],
                origin,
                false,
                &mut sorter
            )
            .is_some()
        );
    }
    #[test]
    fn large_tiles_resort_inside_the_same_section_viewpoint() {
        let mut state = SortState::new(vec![[32.0, 0.0, 0.0], [96.0, 0.0, 0.0]]);
        let mut sorter = Sorter::default();
        assert_eq!(
            state
                .update_lod([48.0, 20.0, 20.0], [0; 3], &mut sorter)
                .unwrap()[0],
            4
        );
        assert!(
            state
                .update_lod([48.5, 20.0, 20.0], [0; 3], &mut sorter)
                .is_none()
        );
        // Both views have positive x/y/z signs relative to the first section,
        // so the section-sized shortcut would incorrectly keep the old order.
        assert_eq!(
            state
                .update_lod([80.0, 20.0, 20.0], [0; 3], &mut sorter)
                .unwrap()[0],
            0
        );
    }

    /// A water block's surface: the front quad looking up, the back one looking down.
    fn surface(y: f32, front: bool) -> [[f32; 3]; 4] {
        let corners = [[0.0, y, 0.0], [0.0, y, 1.0], [1.0, y, 1.0], [1.0, y, 0.0]];
        if front {
            corners
        } else {
            [corners[0], corners[3], corners[2], corners[1]]
        }
    }

    fn pov(camera: [f64; 3], origin: [i32; 3]) -> [i32; 3] {
        viewpoint(camera, origin)
    }

    /// `update` for a camera whose eye is where it is (no view bob).
    fn update_at<'a>(
        state: &mut SortState,
        camera: [f64; 3],
        origin: [i32; 3],
        cull_back: bool,
        sorter: &'a mut Sorter,
    ) -> Option<&'a [u32]> {
        state.update(camera, camera, origin, cull_back, sorter)
    }

    /// How many quads the draw for an eye in `pov` takes.
    fn count(state: &SortState, total: u32, cull_back: bool, pov: [i32; 3]) -> u32 {
        state.drawn(total, cull_back, pov).1
    }

    /// The indices the draw for an eye in `pov` submits out of the section's `indices`, as the
    /// renderer takes them: `quads * 6` from its first quad's.
    fn submitted(state: &SortState, indices: &[u32], total: u32, pov: [i32; 3]) -> Vec<u32> {
        let (skip, quads) = state.drawn(total, true, pov);
        indices[skip as usize * 6..][..quads as usize * 6].to_vec()
    }

    #[test]
    fn a_surface_draws_its_front_to_a_camera_above_and_its_back_to_one_below() {
        let quads = [surface(8.0, true), surface(8.0, false)];
        let mut sorter = Sorter::default();
        let mut state = SortState::of_quads(quads.into_iter());
        // In another section, above: only quad 0 is drawn.
        let above = [8.5, 40.5, 8.5];
        let indices = update_at(&mut state, above, [0, 0, 0], true, &mut sorter)
            .unwrap()
            .to_vec();
        let up = pov(above, [0; 3]);
        assert_eq!(submitted(&state, &indices, 2, up), [0, 1, 2, 2, 3, 0]);
        assert_eq!(count(&state, 2, false, up), 2);
        // Below it: only quad 1.
        let below = [8.5, -20.5, 8.5];
        let indices = update_at(&mut state, below, [0, 0, 0], true, &mut sorter)
            .unwrap()
            .to_vec();
        assert_eq!(
            submitted(&state, &indices, 2, pov(below, [0; 3])),
            [4, 5, 6, 6, 7, 4]
        );
    }

    #[test]
    fn a_camera_in_the_sections_own_extent_sees_every_quad() {
        let mut sorter = Sorter::default();
        let mut state = SortState::of_quads([surface(8.5, true), surface(8.5, false)].into_iter());
        // At any height within the section's 16 blocks, above the plane or below.
        for y in [0.2, 8.4, 8.6, 15.9] {
            let camera = [8.5, y, 8.5];
            update_at(&mut state, camera, [0; 3], true, &mut sorter);
            assert_eq!(count(&state, 2, true, pov(camera, [0; 3])), 2, "at y {y}");
        }
    }

    /// A quad facing down at y 8 (A) and one facing -x at x 0, y 30 (B): from the first camera
    /// B is nearer, from the second A is.
    fn overlapping() -> [[[f32; 3]; 4]; 2] {
        let down = surface(8.0, false);
        let west = [
            [0.0, 30.0, 0.0],
            [0.0, 31.0, 0.0],
            [0.0, 31.0, 1.0],
            [0.0, 30.0, 1.0],
        ];
        [down, west]
    }

    #[test]
    fn an_eye_that_moved_to_another_viewpoint_draws_the_order_it_was_sorted_in() {
        let quads = overlapping();
        assert!(quads.iter().all(|q| Facing::of(q).is_some()));
        let mut sorter = Sorter::default();
        let mut state = SortState::of_quads(quads.into_iter());
        // Sorted from above and west of the section: A faces away, B faces it; B is nearer, so
        // the whole order is A then B.
        let first = [-48.0, 40.0, 8.0];
        let indices = update_at(&mut state, first, [0; 3], true, &mut sorter)
            .unwrap()
            .to_vec();
        let (a, b) = ([0, 1, 2, 2, 3, 0], [4, 5, 6, 6, 7, 4]);
        assert_eq!(submitted(&state, &indices, 2, pov(first, [0; 3])), b);
        // The eye went below before the section was sorted again: A faces it now, and what is
        // drawn is the order the sort made (A, B), not the facing quad first and the rest after.
        let second = [-48.0, -1.0, 8.0];
        assert_ne!(pov(first, [0; 3]), pov(second, [0; 3]));
        let whole = submitted(&state, &indices, 2, pov(second, [0; 3]));
        assert_eq!(whole, [a, b].concat());
        assert_eq!(
            whole,
            Sorter::default().sort(&[[0.5, 8.0, 0.5], [0.0, 30.5, 0.5]], [-48.0, 40.0, 8.0])
        );
        // Back at the first viewpoint, nothing was re-sorted: the facing quads draw again.
        assert_eq!(submitted(&state, &indices, 2, pov(first, [0; 3])), b);
    }

    #[test]
    fn the_facing_follows_the_rendered_eye_not_the_nominal_camera() {
        // Two quads at y 0.2, just inside the section's bottom: one facing up, one down.
        let quads = [surface(0.2, true), surface(0.2, false)];
        let mut sorter = Sorter::default();
        let mut state = SortState::of_quads(quads.iter().copied());
        // The camera is a hair below the section, so it sees only the back quad; the bob lifts the
        // eye into the section, above the plane, and the front quad faces it too.
        let camera = [8.5, -0.1, 8.5];
        let eye = [8.5, 0.4, 8.5];
        assert_ne!(pov(camera, [0; 3]), pov(eye, [0; 3]));
        let indices = state
            .update(camera, eye, [0; 3], true, &mut sorter)
            .unwrap()
            .to_vec();
        assert_eq!(count(&state, 2, true, pov(eye, [0; 3])), 2);
        assert_eq!(indices.len(), 12);
        // The same section drawn for an eye that stays with the camera takes the one quad.
        let mut state = SortState::of_quads(quads.iter().copied());
        let indices = update_at(&mut state, camera, [0; 3], true, &mut sorter)
            .unwrap()
            .to_vec();
        assert_eq!(
            submitted(&state, &indices, 2, pov(camera, [0; 3])),
            [4, 5, 6, 6, 7, 4]
        );
    }

    #[test]
    fn an_eye_crossing_a_section_plane_while_the_camera_does_not_is_sorted_for() {
        let quads = [surface(0.2, true), surface(0.2, false)];
        let mut sorter = Sorter::default();
        let mut state = SortState::of_quads(quads.into_iter());
        let camera = [8.5, -0.1, 8.5];
        let (below, inside) = ([8.5, -0.05, 8.5], [8.5, 0.4, 8.5]);
        assert!(
            state
                .update(camera, below, [0; 3], true, &mut sorter)
                .is_some()
        );
        // Same camera, same block: nothing to sort while the eye stays in its viewpoint.
        assert!(
            state
                .update(camera, below, [0; 3], true, &mut sorter)
                .is_none()
        );
        // The eye crossed: the order is made again for it, though the camera did not move.
        let indices = state
            .update(camera, inside, [0; 3], true, &mut sorter)
            .unwrap()
            .to_vec();
        assert_eq!(count(&state, 2, true, pov(inside, [0; 3])), 2);
        assert_eq!(indices.len(), 12);
        assert!(
            state
                .update(camera, inside, [0; 3], true, &mut sorter)
                .is_none()
        );
    }

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    /// Whether the rasterizer draws triangle `a b c` for a camera at `eye`: its winding, as the
    /// geometric normal's side of the eye, counter-clockwise from the front.
    fn triangle_faces(a: [f32; 3], b: [f32; 3], c: [f32; 3], eye: [f32; 3]) -> bool {
        let (u, v, w) = (sub(b, a), sub(c, a), sub(eye, a));
        let normal = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        normal[0] * w[0] + normal[1] * w[1] + normal[2] * w[2] > 0.0
    }

    /// Whether either of a quad's triangles (0 1 2 and 2 3 0) faces `eye`.
    fn quad_faces(q: &[[f32; 3]; 4], eye: [f32; 3]) -> bool {
        triangle_faces(q[0], q[1], q[2], eye) || triangle_faces(q[2], q[3], q[0], eye)
    }

    /// The quads kept draw in exactly the order the whole list had them: back to front by
    /// distance, ties in mesh order.
    #[test]
    fn the_kept_quads_keep_the_full_back_to_front_order() {
        let mut quads = Vec::new();
        for n in 0..40 {
            let y = 1.0 + (n % 7) as f32 * 0.5;
            let (x, z) = ((n % 5) as f32, (n / 5 % 4) as f32);
            let top = [
                [x, y, z],
                [x, y, z + 1.0],
                [x + 1.0, y, z + 1.0],
                [x + 1.0, y, z],
            ];
            quads.push(if n % 2 == 0 {
                top
            } else {
                [top[0], top[3], top[2], top[1]]
            });
        }
        let centers: Vec<[f32; 3]> = quads.iter().map(|q| quad_center(q[0], q[2])).collect();
        let camera = [-3.5, 40.5, 2.5];
        let full = Sorter::default().sort(&centers, [-3.5, 40.5, 2.5]).to_vec();
        let mut sorter = Sorter::default();
        let mut state = SortState::of_quads(quads.iter().copied());
        let order = update_at(&mut state, camera, [0; 3], true, &mut sorter)
            .unwrap()
            .to_vec();
        let total = quads.len() as u32;
        assert_eq!(order[..full.len()], full);
        let in_prefix = submitted(&state, &order, total, pov(camera, [0; 3]));
        assert!(!in_prefix.is_empty() && in_prefix.len() < full.len());
        let expected: Vec<u32> = full
            .chunks(6)
            .filter(|quad| in_prefix.chunks(6).any(|kept| kept == *quad))
            .flatten()
            .copied()
            .collect();
        assert_eq!(in_prefix, expected);
    }

    #[test]
    fn a_quad_whose_triangles_wind_opposite_ways_is_never_left_out() {
        // A bow-tie in one plane: triangle 0 1 2 winds one way, 2 3 0 the other.
        let bow = [
            [0.0, 3.0, 0.0],
            [1.0, 3.0, 1.0],
            [1.0, 3.0, 0.0],
            [0.0, 3.0, 1.0],
        ];
        assert!(
            triangle_faces(bow[0], bow[1], bow[2], [0.5, 9.0, 0.5])
                != triangle_faces(bow[2], bow[3], bow[0], [0.5, 9.0, 0.5])
        );
        assert_eq!(Facing::of(&bow), None);
        // A zero-area triangle has no winding to decide by.
        let flat = [
            [0.0, 3.0, 0.0],
            [1.0, 3.0, 0.0],
            [2.0, 3.0, 0.0],
            [0.0, 3.0, 1.0],
        ];
        assert_eq!(Facing::of(&flat), None);
    }

    #[test]
    fn a_sloped_quad_stays_while_either_triangle_faces_the_camera() {
        // A flowing surface's top: three corners at 0.5 and one at 0.9, so its triangles lie
        // in different planes (0 1 2 rises towards corner 2, 2 3 0 is level).
        let top = [
            [0.0, 0.5, 0.0],
            [0.0, 0.5, 1.0],
            [1.0, 0.9, 1.0],
            [1.0, 0.5, 0.0],
        ];
        assert_eq!(Facing::of(&top), None);
        // Grazing from the side, a little below the level triangle's plane but above the
        // raised one: only triangle 0 1 2 faces the camera.
        let eye = [-5.0, 0.49, 0.5];
        assert!(triangle_faces(top[0], top[1], top[2], eye));
        assert!(!triangle_faces(top[2], top[3], top[0], eye));
        let mut sorter = Sorter::default();
        let mut state = SortState::of_quads([top].into_iter());
        let camera = [-4.5, 0.49, 0.5];
        update_at(&mut state, camera, [0; 3], true, &mut sorter);
        assert_eq!(count(&state, 1, true, pov(camera, [0; 3])), 1);
    }

    /// Quads on planes at both sides of the section and at the block edges, both windings, from
    /// cameras all around: a quad left out faces no camera of the viewpoint, which is judged here
    /// by each triangle's winding at sample points of the viewpoint's reach, whatever the quad's
    /// position in the section.
    #[test]
    fn left_out_quads_face_away_from_every_camera_of_the_viewpoint() {
        let mut quads = Vec::new();
        for plane in [0.0, 0.44, 3.5, 7.89, 9.56, 15.99, 16.0] {
            let faces = [
                [
                    [0.0, plane, 0.0],
                    [0.0, plane, 1.0],
                    [1.0, plane, 1.0],
                    [1.0, plane, 0.0],
                ],
                [
                    [plane, 0.0, 0.0],
                    [plane, 1.0, 0.0],
                    [plane, 1.0, 1.0],
                    [plane, 0.0, 1.0],
                ],
                [
                    [0.0, 0.0, plane],
                    [1.0, 0.0, plane],
                    [1.0, 1.0, plane],
                    [0.0, 1.0, plane],
                ],
            ];
            for face in faces {
                quads.push(face);
                // The same quad with the winding reversed, as water's back face is.
                quads.push([face[0], face[3], face[2], face[1]]);
            }
        }
        assert!(quads.iter().all(|q| Facing::of(q).is_some()));
        let mut sorter = Sorter::default();
        let origin = [32, -16, 64];
        let mut left_out = 0;
        for x in [-9.5_f64, 3.2, 20.5, 31.9] {
            for y in [-30.2_f64, -16.0, -5.5, 2.5, 40.0] {
                for z in [-20.5_f64, 70.4, 80.5, 100.0] {
                    let camera = [
                        f64::from(origin[0]) + x,
                        f64::from(origin[1]) + y,
                        f64::from(origin[2]) + z,
                    ];
                    let mut state = SortState::of_quads(quads.iter().copied());
                    let indices = update_at(&mut state, camera, origin, true, &mut sorter)
                        .unwrap()
                        .to_vec();
                    let side = pov(camera, origin);
                    let total = quads.len() as u32;
                    let kept = submitted(&state, &indices, total, side);
                    // Cameras of the viewpoint, section-relative: three samples an axis.
                    let eyes: Vec<[f32; 3]> = (0..27)
                        .map(|n| {
                            [0, 1, 2].map(|i| {
                                let [low, high] = match side[i] {
                                    1.. => [16.0, 40.0],
                                    0 => [0.0, 16.0],
                                    _ => [-24.0, -0.001],
                                };
                                low + (high - low) * ((n / 3_usize.pow(i as u32)) % 3) as f32 / 2.0
                            })
                        })
                        .collect();
                    for quad in indices[..total as usize * 6]
                        .chunks(6)
                        .filter(|quad| !kept.chunks(6).any(|k| k == *quad))
                    {
                        left_out += 1;
                        let q = &quads[(quad[0] / 4) as usize];
                        assert!(
                            eyes.iter().all(|&eye| !quad_faces(q, eye)),
                            "{q:?} is left out of the viewpoint {side:?} but faces a camera"
                        );
                    }
                }
            }
        }
        assert!(left_out > 0);
    }
}
