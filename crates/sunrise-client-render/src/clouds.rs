//! Cloud visibility and flat/fancy cell geometry (`CloudRenderer`).
/// `EnvironmentAttributes.CLOUD_HEIGHT`'s default (the overworld's `visual/cloud_height`); the
/// layer itself follows [`Input::height`].
pub const CLOUD_HEIGHT: f32 = 192.33;

/// Whether the cloud color has any packed alpha (vanilla gates the clouds on it).
pub fn has_color(alpha: f32) -> bool {
    (alpha.clamp(0.0, 1.0) * 255.0) as u8 > 0
}

/// `LevelRenderer`: skip OFF clouds and colors with no packed alpha.
pub fn visible(mode: &str, alpha: f32) -> bool {
    mode != "false" && has_color(alpha)
}

/// `buildFlatCell`: bottom plane, in both windings for the culled cloud pipeline.
pub fn flat_cell(x: f32, z: f32, bottom: f32) -> [[[f32; 3]; 4]; 2] {
    let p = [
        [x + 12.0, bottom, z],
        [x + 12.0, bottom, z + 12.0],
        [x, bottom, z + 12.0],
        [x, bottom, z],
    ];
    [p, [p[3], p[2], p[1], p[0]]]
}

/// Direction order is DOWN, UP, NORTH, SOUTH, WEST, EAST; inward faces use bits 6..12.
pub fn face_mask(x: i32, z: i32, bottom: f32, empty: [bool; 4]) -> u16 {
    let mut mask = 0;
    if bottom <= 0.0 {
        mask |= 1 << 1;
    }
    if bottom + 4.0 >= 0.0 {
        mask |= 1;
    }
    for (i, visible) in [z > 0, z < 0, x > 0, x < 0].into_iter().enumerate() {
        if empty[i] && visible {
            mask |= 1 << (i + 2);
        }
    }
    if x.abs() <= 1 && z.abs() <= 1 {
        mask |= 63 << 6;
    }
    mask
}

/// Exterior faces first, then reversed interior faces, in vanilla emission order.
pub fn faces(mask: u16) -> impl Iterator<Item = (u8, bool)> {
    [1, 0, 2, 3, 4, 5]
        .into_iter()
        .filter(move |d| mask & (1 << d) != 0)
        .map(|d| (d, false))
        .chain(
            (0..6)
                .filter(move |d| mask & (1 << (d + 6)) != 0)
                .map(|d| (d, true)),
        )
}

/// `CloudRenderer.prepare`: the cell radius for the Cloud Distance option
/// (in chunks), independent of the render distance.
pub fn radius_cells(cloud_range_chunks: i32) -> i32 {
    ((cloud_range_chunks * 16) as f32 / 12.0).ceil() as i32
}

/// Cells the mesh's origin snaps to. The mesh is a window on the cloud pattern around the
/// camera's cell, so it used to be rebuilt, and the whole array re-uploaded, on every cell
/// crossed: 6.5 MB, about 11 ms at the default 128-chunk range, the worst frame of a flight.
/// Anchoring the origin to a coarser grid rebuilds it once every `SNAP_CELLS` cells instead,
/// and `Uniform::offset` carries the remaining shift. The ring of cells the shift can leave
/// uncovered at the far side lies past the cloud fog end, where the cloud fade has already
/// taken the alpha to 0 (`entity.wgsl`'s `fs_entity`), so the extra cells cost only the
/// vertices they add.
const SNAP_CELLS: i64 = 4;

#[derive(Clone)]
pub struct CellIter {
    radius: i32,
    ring: i32,
    x: i32,
    pending_z: Option<i32>,
}

impl CellIter {
    pub fn new(radius: i32) -> Self {
        Self {
            radius,
            ring: 0,
            x: 0,
            pending_z: None,
        }
    }
}

impl Iterator for CellIter {
    type Item = (i32, i32);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(z) = self.pending_z.take() {
            let x = self.x;
            self.x += 1;
            return Some((x, z));
        }
        let r2 = self.radius * self.radius;
        while self.ring <= 2 * self.radius {
            let min_abs = self.ring - self.radius;
            while self.x <= self.ring {
                if min_abs > 0 && self.x > -min_abs && self.x < min_abs {
                    self.x = min_abs;
                }
                let x = self.x;
                let z = self.ring - x.abs();
                if z <= self.radius && x * x + z * z <= r2 {
                    if z != 0 {
                        self.pending_z = Some(z);
                        return Some((x, -z));
                    } else {
                        self.x += 1;
                        return Some((x, 0));
                    }
                }
                self.x += 1;
            }
            self.ring += 1;
            self.x = -self.ring;
        }
        None
    }
}

/// CloudRenderer builds cells in expanding Manhattan rings, nearest first.
pub fn cells(radius: i32) -> CellIter {
    CellIter::new(radius)
}

/// Per-frame cloud inputs. Resources are immutable for the renderer's lifetime:
/// resource-pack reloads replace the renderer and its cache together.
pub struct Input<'a> {
    pub texture: &'a (u32, u32, Vec<bool>),
    pub uv: [f32; 2],
    pub camera: glam::DVec3,
    pub time: i64,
    pub partial_tick: f64,
    pub range: i32,
    pub fast: bool,
    pub color: [f32; 4],
    /// The layer's bottom: `EnvironmentAttributes.CLOUD_HEIGHT` probed at the camera this frame
    /// (`LevelExtractor`: `levelRenderState.cloudHeight`, a float), as the game samples it
    /// (`environment::Sky::cloud_height`).
    pub height: f32,
}

#[derive(Debug, PartialEq, Eq)]
struct MeshKey {
    cell_x: i64,
    cell_z: i64,
    radius: i32,
    fast: bool,
    // The two vertical face tests include both surfaces at the boundaries.
    top: bool,
    bottom: bool,
    // The camera's cell within the snapped origin's, for the 3x3 cells whose inward
    // faces are drawn. Only read while the camera is inside the layer.
    sub_x: i32,
    sub_z: i32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Uniform {
    offset: [f32; 4],
    colors: [[f32; 4]; 4],
}

/// How far the clouds have drifted, in ticks: the game time within one trip across the cloud
/// texture (`width` cells of 400 ticks), plus the partial tick (`CloudRenderer.prepare`).
pub fn drift(time: i64, partial_tick: f64, width: u32) -> f64 {
    time.rem_euclid(i64::from(width) * 400) as f64 + partial_tick
}

impl Input<'_> {
    /// The mesh around the camera's cell snapped to `snap` cells (1: vanilla's own mesh, which a
    /// pack's `core/clouds` shader reads).
    fn state(&self, snap: i64) -> (MeshKey, Uniform) {
        let (w, h, _) = self.texture;
        // CloudRenderer.prepare's float arithmetic: the drift in ticks is a float (the game time's
        // remainder plus the partial tick), times the float speed, then widened.
        let offset = (self.time % (i64::from(*w) * 400)) as f32 + self.partial_tick as f32;
        let mut x = self.camera.x + f64::from(offset * 0.030_000_001_f32);
        // Vanilla adds 3.96 as a double literal written from the float: 3.9600000381469727.
        let mut z = self.camera.z + 3.960_000_038_146_972_7;
        let (tw, th) = (f64::from(*w) * 12.0, f64::from(*h) * 12.0);
        x -= (x / tw).floor() * tw;
        z -= (z / th).floor() * th;
        let cell_x = (x / 12.0).floor() as i64 / snap * snap;
        let cell_z = (z / 12.0).floor() as i64 / snap * snap;
        // `prepare`'s bottomY is a float (the cloud height attribute's), widened to subtract
        // the camera's double y, then narrowed again.
        let bottom = (f64::from(self.height) - self.camera.y) as f32;
        // The inward faces of the 3x3 cells around the camera are only seen from inside the
        // layer; outside it they are back-facing, so the mesh need not follow the camera there.
        let inside_layer = bottom <= 0.0 && bottom + 4.0 >= 0.0;
        let (sub_x, sub_z) = if inside_layer {
            (
                (x / 12.0).floor() as i32 - cell_x as i32,
                (z / 12.0).floor() as i32 - cell_z as i32,
            )
        } else {
            (0, 0)
        };
        let key = MeshKey {
            cell_x,
            cell_z,
            // One more ring than the range, for the cells the snapped origin can uncover.
            radius: radius_cells(self.range) + if snap > 1 { snap as i32 } else { 0 },
            fast: self.fast,
            top: bottom <= 0.0,
            bottom: bottom + 4.0 >= 0.0,
            sub_x,
            sub_z,
        };
        let colors = [1.0, 0.7, 0.8, 0.9].map(|shade| {
            let [r, g, b, a] = self.color;
            [r * shade, g * shade, b * shade, a]
                .map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8 as f32 / 255.0)
        });
        (
            key,
            Uniform {
                offset: [
                    -(x - cell_x as f64 * 12.0) as f32,
                    bottom,
                    -(z - cell_z as f64 * 12.0) as f32,
                    0.0,
                ],
                colors,
            },
        )
    }
}

/// The cloud cells around a snapped origin, rebuilt when the key changes: once every
/// [`SNAP_CELLS`] cells the camera crosses, and every cell while the camera is inside the
/// layer, where the inward faces follow it. The renderer copies the vertices to the GPU on
/// a rebuild, which is why the origin snaps at all.
#[derive(Default)]
pub(crate) struct Mesh {
    key: Option<MeshKey>,
    cells: Vec<(i32, i32)>,
    cached_radius: Option<i32>,
    pub vertices: Vec<crate::mesher::Vertex>,
}

#[inline(always)]
fn push_quad(
    vertices: &mut Vec<crate::mesher::Vertex>,
    p: [[f32; 3]; 4],
    shade: u32,
    normal: [i8; 4],
    uv: [f32; 2],
) {
    for pos in p {
        vertices.push(crate::mesher::Vertex {
            normal,
            pos,
            uv,
            layer: 4,
            color: u32::MAX,
            light: shade,
            glint_uv: [0.0, 0.0],
        });
    }
}

#[inline(always)]
fn emit_face(
    vertices: &mut Vec<crate::mesher::Vertex>,
    direction: usize,
    inside: bool,
    x0: f32,
    z0: f32,
    uv: [f32; 2],
) {
    let x1 = x0 + 12.0;
    let z1 = z0 + 12.0;
    const Y0: f32 = 0.0;
    const Y1: f32 = 4.0;
    let (p, shade, normal) = match direction {
        0 => (
            [[x1, Y0, z0], [x1, Y0, z1], [x0, Y0, z1], [x0, Y0, z0]],
            1,
            [0, -127, 0, 0],
        ),
        1 => (
            [[x0, Y1, z0], [x0, Y1, z1], [x1, Y1, z1], [x1, Y1, z0]],
            0,
            [0, 127, 0, 0],
        ),
        2 => (
            [[x0, Y0, z0], [x0, Y1, z0], [x1, Y1, z0], [x1, Y0, z0]],
            2,
            [0, 0, -127, 0],
        ),
        3 => (
            [[x1, Y0, z1], [x1, Y1, z1], [x0, Y1, z1], [x0, Y0, z1]],
            2,
            [0, 0, 127, 0],
        ),
        4 => (
            [[x0, Y0, z1], [x0, Y1, z1], [x0, Y1, z0], [x0, Y0, z0]],
            3,
            [-127, 0, 0, 0],
        ),
        _ => (
            [[x1, Y0, z0], [x1, Y1, z0], [x1, Y1, z1], [x1, Y0, z1]],
            3,
            [127, 0, 0, 0],
        ),
    };
    if inside {
        push_quad(
            vertices,
            [p[3], p[2], p[1], p[0]],
            shade,
            normal.map(|c| -c),
            uv,
        );
    } else {
        push_quad(vertices, p, shade, normal, uv);
    }
}

#[inline(always)]
fn emit_faces(
    vertices: &mut Vec<crate::mesher::Vertex>,
    mask: u16,
    x0: f32,
    z0: f32,
    uv: [f32; 2],
) {
    for d in [1, 0, 2, 3, 4, 5] {
        if mask & (1 << d) != 0 {
            emit_face(vertices, d, false, x0, z0, uv);
        }
    }
    if mask >> 6 != 0 {
        for d in 0..6 {
            if mask & (1 << (d + 6)) != 0 {
                emit_face(vertices, d, true, x0, z0, uv);
            }
        }
    }
}

#[inline(always)]
fn emit_flat_cell(vertices: &mut Vec<crate::mesher::Vertex>, x0: f32, z0: f32, uv: [f32; 2]) {
    let x1 = x0 + 12.0;
    let z1 = z0 + 12.0;
    let p = [[x1, 0.0, z0], [x1, 0.0, z1], [x0, 0.0, z1], [x0, 0.0, z0]];
    push_quad(vertices, p, 0, [0; 4], uv);
    push_quad(vertices, [p[3], p[2], p[1], p[0]], 0, [0; 4], uv);
}

impl Mesh {
    /// Returns true only when vertices must be uploaded. Sub-cell motion and
    /// color changes update only the uniform, as in CloudRenderer.prepare.
    pub fn prepare(&mut self, input: &Input<'_>) -> (bool, Uniform) {
        let (key, uniform) = input.state(SNAP_CELLS);
        if self.key.as_ref() == Some(&key) {
            return (false, uniform);
        }
        if self.cached_radius != Some(key.radius) {
            self.cells = self::cells(key.radius).collect();
            self.cached_radius = Some(key.radius);
        }
        self.vertices.clear();

        let (w, h, tex_cells) = input.texture;
        let (cell_x, cell_z) = (key.cell_x, key.cell_z);
        let bottom = uniform.offset[1];
        let uv = input.uv;

        let (w_i64, h_i64) = (i64::from(*w), i64::from(*h));
        let filled = |cx: i64, cz: i64| {
            tex_cells[(cz.rem_euclid(h_i64) * w_i64 + cx.rem_euclid(w_i64)) as usize]
        };

        for &(dx, dz) in &self.cells {
            let (dx_i64, dz_i64) = (i64::from(dx), i64::from(dz));
            let (cx, cz) = (cell_x + dx_i64, cell_z + dz_i64);
            if !filled(cx, cz) {
                continue;
            }
            let x0 = dx as f32 * 12.0;
            let z0 = dz as f32 * 12.0;

            if input.fast {
                emit_flat_cell(&mut self.vertices, x0, z0, uv);
                continue;
            }

            let mask = face_mask(
                dx - key.sub_x,
                dz - key.sub_z,
                bottom,
                [
                    !filled(cx, cz - 1),
                    !filled(cx, cz + 1),
                    !filled(cx - 1, cz),
                    !filled(cx + 1, cz),
                ],
            );
            emit_faces(&mut self.vertices, mask, x0, z0, uv);
        }

        self.key = Some(key);
        (true, uniform)
    }
}

/// The cloud texture's pixel vanilla 26.3's `CloudRenderer.prepare` asks for outside the image
/// (`NativeImage.getPixel`'s message: `(x, y) outside of image bounds (width, height)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutsideImage {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Display for OutsideImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            x,
            y,
            width,
            height,
        } = self;
        write!(f, "({x}, {y}) outside of image bounds ({width}, {height})")
    }
}

/// Where `CloudRenderer.prepare` would read past a cloud texture: it packs each filled cell's
/// north, east, south and west neighbours, rows first, and wraps east and west by the texture's
/// *height*. A texture no wider than tall is read inside; one taller than wide is read outside
/// for a filled cell in its first column (west) or its last (east), and vanilla's `getPixel`
/// throws there: the reload fails and every selected pack is dropped. The first such pixel, as
/// vanilla meets it, or none.
pub fn outside_image(texture: &(u32, u32, Vec<bool>)) -> Option<OutsideImage> {
    let (w, h, filled) = texture;
    let (wi, hi) = (*w as i32, *h as i32);
    for y in 0..hi {
        for x in 0..wi {
            if !filled[(y * wi + x) as usize] {
                continue;
            }
            for nx in [(x + 1).rem_euclid(hi), (x - 1).rem_euclid(hi)] {
                if nx >= wi {
                    return Some(OutsideImage {
                        x: nx,
                        y,
                        width: *w,
                        height: *h,
                    });
                }
            }
        }
    }
    None
}

/// Where the cloud texture is read from.
pub const TEXTURE: &str = "assets/minecraft/textures/environment/clouds.png";

/// A cloud texture vanilla 26.3 would read past ([`outside_image`]): the pixel, named as
/// `NativeImage.getPixel` names it, as the reload's error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused(pub OutsideImage);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{TEXTURE}: {}", self.0)
    }
}

impl std::error::Error for Refused {}

/// The cloud texture (`textures/environment/clouds.png`) of the resources at `root` as filled
/// cells (alpha 10 or more, `CloudRenderer.isCellEmpty`); one with no such file, or that doesn't
/// decode, has none. One vanilla would read past ([`outside_image`]) is refused: in vanilla
/// `CloudRenderer.prepare` throws, the reload fails and the selected packs are dropped, and the
/// caller's reload fails the same way.
pub fn load(
    root: &crate::resource_source::ResourceSource,
) -> Result<(u32, u32, Vec<bool>), Refused> {
    let texture = root
        .read(TEXTURE)
        .map_err(Into::into)
        .and_then(|bytes| crate::hud::decode_bytes(&bytes))
        .map(|(w, h, px)| {
            (
                w,
                h,
                px.as_chunks::<4>().0.iter().map(|p| p[3] >= 10).collect(),
            )
        })
        .unwrap_or((1, 1, vec![false]));
    match outside_image(&texture) {
        None => Ok(texture),
        Some(pixel) => Err(Refused(pixel)),
    }
}

/// What a pack's `core/clouds` shader reads (`CloudRenderer.prepare` and `buildMesh`): the faces
/// it reads from `CloudFaces` and its `CloudInfo` block; and its programs.
pub mod pack {
    use super::{Input, MeshKey};

    /// `encodeFace`'s flags beside the direction (`clouds.vsh`'s `FLAG_INSIDE_FACE`,
    /// `FLAG_USE_TOP_COLOR`), and `Direction.get3DDataValue`.
    const INSIDE_FACE: i32 = 16;
    const USE_TOP_COLOR: i32 = 32;
    const DOWN: i32 = 0;
    const UP: i32 = 1;
    const NORTH: i32 = 2;
    const SOUTH: i32 = 3;
    const WEST: i32 = 4;
    const EAST: i32 = 5;

    /// A pack's `core/clouds` as vanilla compiles it: for `RenderPipelines.CLOUDS` and
    /// `FLAT_CLOUDS`, and for `OIT_CLOUDS` and `OIT_FLAT_CLOUDS`, one program a stage
    /// (`OitStage`'s order). Vanilla requires them all: a pack whose shader can't make one
    /// fails the load (`hud::RequiredShaderRefused`).
    #[derive(Debug)]
    pub struct Programs {
        pub classic: sunrise_glsl::Translated,
        pub oit: Box<[sunrise_glsl::Translated; 3]>,
    }

    /// What [`faces`] depends on: the camera's cell and side of the layer, the range and the
    /// style, and the cloud texture. `CloudRenderer.prepare` builds its mesh again only when the
    /// cell, the side or the style change.
    #[derive(Debug, PartialEq, Eq)]
    pub struct FacesKey(MeshKey, usize, u32, u32);

    /// The key [`faces`] gives the same faces for.
    pub fn faces_key(input: &Input<'_>) -> FacesKey {
        let (w, h, filled) = input.texture;
        FacesKey(input.state(1).0, filled.as_ptr() as usize, *w, *h)
    }

    /// `CloudInfo` (std140: `CloudColor`, then `CloudOffset`, the camera's place in its cell
    /// negated with the layer's bottom relative to it, then `CellSize`), which follows the
    /// camera every frame. The colour goes through 8 bits a channel, as vanilla's ARGB int.
    pub fn info(input: &Input<'_>) -> [u8; 48] {
        let (_, uniform) = input.state(1);
        let mut info = [0f32; 12];
        info[..4].copy_from_slice(
            &input
                .color
                .map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8 as f32 / 255.0),
        );
        info[4..7].copy_from_slice(&uniform.offset[..3]);
        info[8..11].copy_from_slice(&[12.0, 4.0, 12.0]);
        let mut bytes = [0u8; 48];
        bytes.copy_from_slice(bytemuck::cast_slice(&info));
        bytes
    }

    /// `buildMesh`'s faces, three `R8_SINT` texels each (here one `i32` a texel): the cell's
    /// relative x and z halved, then the direction with its flags, the halves' lost bits in
    /// bits 7 (x) and 6 (z). Cells in `buildMesh`'s rings, nearest first.
    pub fn faces(input: &Input<'_>) -> Vec<i32> {
        let (key, uniform) = input.state(1);
        let (w, h, filled) = input.texture;
        let (w, h) = (*w as i32, *h as i32);
        let empty = |x: i32, y: i32| !filled[(y * w + x) as usize];
        // RelativeCameraPos: above the layer's top, below its bottom, or inside it.
        let bottom = uniform.offset[1];
        let above = bottom + 4.0 < 0.0;
        let below = bottom > 0.0;
        let fancy = !input.fast;
        let radius = key.radius;
        let mut faces = Vec::new();
        let mut encode = |x: i32, z: i32, direction: i32, flags: i32| {
            let dir_and_flags = direction | flags | (x & 1) << 7 | (z & 1) << 6;
            // `(byte)` casts, read back as signed 8-bit texels.
            faces.extend([(x >> 1) as i8, (z >> 1) as i8, dir_and_flags as i8].map(i32::from));
        };
        // tryBuildCell.
        let mut cell = |x: i32, z: i32| {
            let ix = (key.cell_x as i32 + x).rem_euclid(w);
            let iy = (key.cell_z as i32 + z).rem_euclid(h);
            if empty(ix, iy) {
                return;
            }
            if !fancy {
                encode(x, z, DOWN, USE_TOP_COLOR);
                return;
            }
            // The neighbours `CloudRenderer.prepare` packs into a cell, east and west wrapping
            // by the texture's height as vanilla's do; a texture where that leaves the image is
            // refused at load ([`super::outside_image`]).
            let north = empty(ix, (iy - 1).rem_euclid(h));
            let east = empty((ix + 1).rem_euclid(h), iy);
            let south = empty(ix, (iy + 1).rem_euclid(h));
            let west = empty((ix - 1).rem_euclid(h), iy);
            // buildExtrudedCell.
            if !below {
                encode(x, z, UP, 0);
            }
            if !above {
                encode(x, z, DOWN, 0);
            }
            if north && z > 0 {
                encode(x, z, NORTH, 0);
            }
            if south && z < 0 {
                encode(x, z, SOUTH, 0);
            }
            if west && x > 0 {
                encode(x, z, WEST, 0);
            }
            if east && x < 0 {
                encode(x, z, EAST, 0);
            }
            if x.abs() <= 1 && z.abs() <= 1 {
                for direction in [DOWN, UP, NORTH, SOUTH, WEST, EAST] {
                    encode(x, z, direction, INSIDE_FACE);
                }
            }
        };
        // buildMesh's rings, nearest first.
        for (x, z) in super::cells(radius) {
            cell(x, z);
        }
        faces
    }
}

#[cfg(test)]
#[path = "clouds_pack_tests.rs"]
mod pack_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_mesh_tracks_geometry_but_not_subcell_motion_or_color() {
        let texture = (8, 8, (0..64).map(|i| i % 3 != 0).collect());
        let mut input = Input {
            texture: &texture,
            uv: [0.5; 2],
            camera: glam::DVec3::new(1.0, 100.0, 1.0),
            time: 0,
            partial_tick: 0.0,
            range: 2,
            fast: false,
            color: [1.0; 4],
            height: CLOUD_HEIGHT,
        };
        let mut mesh = Mesh::default();
        assert!(mesh.prepare(&input).0);
        let vertices = bytemuck::cast_slice::<_, u8>(&mesh.vertices).to_vec();
        let pointer = mesh.vertices.as_ptr();
        for tick in 0..100 {
            input.time = tick;
            input.partial_tick = 0.5;
            input.camera.x = 2.0;
            input.camera.y = 101.0;
            input.color = [0.8, 0.7, 0.6, 0.5];
            let (rebuilt, uniform) = mesh.prepare(&input);
            assert!(!rebuilt);
            assert_eq!(mesh.vertices.as_ptr(), pointer);
            assert_eq!(bytemuck::cast_slice::<_, u8>(&mesh.vertices), vertices);
            assert_eq!(
                uniform.offset[1],
                (f64::from(192.33f32) - input.camera.y) as f32
            );
            assert_eq!(
                uniform.colors[1],
                [0.8 * 0.7, 0.7 * 0.7, 0.6 * 0.7, 0.5].map(|v| (v * 255.0) as u8 as f32 / 255.0)
            );
            for color in uniform.colors {
                assert_eq!(color[3], 127.0 / 255.0);
            }
        }
        // One block into the second snap cell: every case below moves the snapped origin, or
        // another part of the key, so each one rebuilds.
        let far_cell = SNAP_CELLS as f64 * 12.0 + 1.0;
        for (x, y, z, range, fast) in [
            (far_cell, 101.0, 1.0, 2, false),
            (far_cell, 101.0, -5.0, 2, false),
            // Level with the clouds' bottom, the float height vanilla subtracts from.
            (far_cell, f64::from(192.33f32), -5.0, 2, false),
            (far_cell, 196.34, -5.0, 2, false),
            (far_cell, 196.34, -5.0, 3, false),
            (far_cell, 196.34, -5.0, 3, true),
        ] {
            input.camera = glam::DVec3::new(x, y, z);
            input.range = range;
            input.fast = fast;
            assert!(mesh.prepare(&input).0);
            let mut fresh = Mesh::default();
            fresh.prepare(&input);
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&mesh.vertices),
                bytemuck::cast_slice::<_, u8>(&fresh.vertices)
            );
            assert!(!mesh.prepare(&input).0);
        }
        // Drift alone crosses the snapped origin, even with a stationary camera.
        input.camera = glam::DVec3::new(1.0, 101.0, 1.0);
        input.time = 0;
        input.partial_tick = 0.0;
        assert!(mesh.prepare(&input).0);
        assert!(!mesh.prepare(&input).0);
        // A higher layer, still above the camera, moves only the uniform: its bottom is 250 less
        // the camera's 101.
        input.height = 250.0;
        let (rebuilt, uniform) = mesh.prepare(&input);
        assert!(!rebuilt);
        assert_eq!(uniform.offset[1], 149.0);
        // 400 ticks of drift move the clouds 12 blocks: one whole snap cell.
        input.time = SNAP_CELLS * 400;
        assert!(mesh.prepare(&input).0);
        // Resource reload constructs a new renderer and cache, including empty packs.
        let mut reloaded = Mesh::default();
        let empty = (1, 1, vec![false]);
        input.texture = &empty;
        assert!(reloaded.prepare(&input).0);
        assert!(reloaded.vertices.is_empty());
        assert!(!reloaded.prepare(&input).0);
    }

    #[test]
    fn a_snap_cell_of_motion_keeps_the_mesh_at_the_same_world_cells() {
        let texture = (8, 8, (0..64).map(|i| i % 3 != 0).collect());
        let mut input = Input {
            texture: &texture,
            uv: [0.5; 2],
            camera: glam::DVec3::new(1.0, 100.0, 1.0),
            time: 0,
            partial_tick: 0.0,
            range: 2,
            fast: false,
            color: [1.0; 4],
            height: CLOUD_HEIGHT,
        };
        // One snap cell, in blocks.
        let cell = SNAP_CELLS as f64 * 12.0;
        let mut mesh = Mesh::default();
        let (rebuilt, uniform) = mesh.prepare(&input);
        assert!(rebuilt);
        // The shader draws a vertex at `camera + position + offset`, so the world cells the
        // cloud covers are `camera` plus this: the same ones anywhere inside a snap cell.
        assert_eq!(input.camera.x as f32 + uniform.offset[0], 0.0);
        for x in [1.5, cell * 0.15, cell * 0.5, cell * 0.75, cell - 1.5] {
            input.camera.x = x;
            let (rebuilt, uniform) = mesh.prepare(&input);
            assert!(!rebuilt, "rebuilt at {x}");
            assert_eq!(input.camera.x as f32 + uniform.offset[0], 0.0);
            // The trailing origin still covers the whole range past the camera.
            let far = mesh
                .vertices
                .iter()
                .map(|v| v.pos[0] + uniform.offset[0])
                .fold(f32::MIN, f32::max);
            assert!(far >= 32.0, "clouds end {far} blocks ahead at {x}");
        }
        // The next snap cell takes the origin with it.
        input.camera.x = cell;
        let (rebuilt, uniform) = mesh.prepare(&input);
        assert!(rebuilt);
        assert_eq!(input.camera.x as f32 + uniform.offset[0], cell as f32);
    }

    #[test]
    fn only_inside_the_layer_does_the_mesh_follow_the_camera_between_snap_cells() {
        let texture = (8, 8, (0..64).map(|i| i % 3 != 0).collect());
        let mut input = Input {
            texture: &texture,
            uv: [0.5; 2],
            camera: glam::DVec3::new(1.0, 101.0, 1.0),
            time: 0,
            partial_tick: 0.0,
            range: 2,
            fast: false,
            color: [1.0; 4],
            height: CLOUD_HEIGHT,
        };
        let mut mesh = Mesh::default();
        assert!(mesh.prepare(&input).0);
        // Below the layer, cells crossed inside one snap cell change nothing visible.
        for (x, z) in [(11.9, 11.9), (24.0, 12.0), (35.5, 23.9)] {
            input.camera.x = x;
            input.camera.z = z;
            assert!(
                !mesh.prepare(&input).0,
                "rebuilt at {x},{z} below the layer"
            );
        }
        // Inside it, the inward faces around the camera do move with its own cell.
        input.camera.y = 194.0;
        assert!(mesh.prepare(&input).0);
        input.camera.x = 12.0;
        assert!(mesh.prepare(&input).0);
        input.camera.y = 101.0;
        assert!(mesh.prepare(&input).0);
    }

    #[test]
    fn settings_and_packed_alpha_gate_clouds() {
        for mode in ["false", "fast", "true"] {
            assert_eq!(visible(mode, 1.0), mode != "false");
            assert!(!visible(mode, 0.0));
            assert!(!visible(mode, 0.5 / 255.0));
        }
        assert!(visible("fast", 1.0 / 255.0));
    }

    #[test]
    fn fast_cells_are_flat_at_the_bottom_and_visible_from_both_sides() {
        for bottom in [-10.0, 0.0, 10.0] {
            let [down, up] = flat_cell(-3.0, 7.0, bottom);
            assert_eq!(
                down,
                [
                    [9.0, bottom, 7.0],
                    [9.0, bottom, 19.0],
                    [-3.0, bottom, 19.0],
                    [-3.0, bottom, 7.0],
                ]
            );
            assert_eq!(up, [down[3], down[2], down[1], down[0]]);
            for (quad, sign) in [(down, -1.0), (up, 1.0)] {
                let [a, b, c] = [quad[0], quad[1], quad[2]].map(glam::Vec3::from_array);
                assert!((b - a).cross(c - a).y * sign > 0.0);
            }
        }
    }

    #[test]
    fn cells_are_nearest_first_and_unique() {
        let cells: Vec<_> = cells(2).collect();
        assert_eq!(cells[0], (0, 0));
        assert_eq!(cells.len(), 13);
        assert!(
            cells
                .windows(2)
                .all(|w| w[0].0.abs() + w[0].1.abs() <= w[1].0.abs() + w[1].1.abs())
        );
        assert_eq!(
            cells.iter().collect::<std::collections::HashSet<_>>().len(),
            cells.len()
        );
    }
    #[test]
    fn only_camera_facing_exterior_faces() {
        assert_eq!(face_mask(2, 2, 10.0, [true; 4]), 1 | 4 | 16);
        assert_eq!(face_mask(-2, -2, -5.0, [true; 4]), 2 | 8 | 32);
        assert_eq!(face_mask(2, 2, 0.0, [false; 4]), 3);
    }
    #[test]
    fn cloud_distance_sets_the_radius() {
        assert_eq!(radius_cells(2), 3);
        assert_eq!(radius_cells(12), 16);
        assert_eq!(radius_cells(128), 171);
    }
    #[test]
    fn connected_cells_have_no_exterior_walls() {
        assert_eq!(face_mask(2, 2, 10.0, [false; 4]), 1);
    }
    #[test]
    fn nearby_cells_keep_inward_faces_for_camera_inside_clouds() {
        assert_eq!(face_mask(0, 0, 0.0, [false; 4]), 3 | (63 << 6));
    }
}
