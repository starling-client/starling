//! A GPU particle type's own core shader (wiki "GPU Particles", the 1.21.11 format): the
//! `shader` and `uniforms` it names, and the `polytone:gpu_particle.glsl` block of its knobs the
//! shader imports, as Sunrise lays it out.
use crate::expr::Program;
use crate::particles::{CustomParticle, Gpu, RenderType, program};
use serde_json::Value as Json;

/// A GPU particle's own core shader (`shader`, `ns:name`, `minecraft` when it names no
/// namespace) and its `uniforms`: each a block of its own holding one float, set every frame
/// from its expression.
#[derive(Debug)]
pub struct ParticleShader {
    pub namespace: String,
    pub name: String,
    pub uniforms: Vec<(String, Program)>,
    /// The uniforms whose expression doesn't compile, and why.
    pub refused: Vec<(String, String)>,
}

impl ParticleShader {
    /// The shader's id as the pack names it (`ns:name`).
    pub fn id(&self) -> String {
        format!("{}:{}", self.namespace, self.name)
    }

    /// The folder its stages are in, as a core shader's relative includes start from
    /// (`ns:shaders/core/` and the folders of `name`).
    pub fn stage_dir(&self) -> String {
        let folders = self.name.rsplit_once('/').map_or("", |(dir, _)| dir);
        let folders = if folders.is_empty() {
            String::new()
        } else {
            format!("{folders}/")
        };
        format!("{}:shaders/core/{folders}", self.namespace)
    }

    /// The pack file of one of its stages (`vsh` or `fsh`).
    pub fn stage_path(&self, extension: &str) -> String {
        format!(
            "assets/{}/shaders/core/{}.{extension}",
            self.namespace, self.name
        )
    }
}

impl CustomParticle {
    /// A GPU particle's own `shader`; `None` for any other particle, or one without it.
    pub fn gpu_shader(&self) -> Option<ParticleShader> {
        self.gpu.as_ref()?;
        let shader = self.render.get("shader")?.as_str()?;
        let (namespace, name) = shader.split_once(':').unwrap_or(("minecraft", shader));
        let mut uniforms = Vec::new();
        let mut refused = Vec::new();
        for (uniform, expression) in self
            .render
            .get("uniforms")
            .and_then(Json::as_object)
            .into_iter()
            .flatten()
        {
            let compiled = match expression {
                Json::String(_) | Json::Number(_) => program(expression),
                _ => Err(format!("{expression} is not an expression")),
            };
            match compiled {
                Ok(compiled) => uniforms.push((uniform.clone(), compiled)),
                Err(e) => refused.push((uniform.clone(), e)),
            }
        }
        Some(ParticleShader {
            namespace: namespace.to_owned(),
            name: name.to_owned(),
            uniforms,
            refused,
        })
    }
}

impl Gpu {
    /// The pack file of `texture` (`ns:textures/particle/rain.png`, `minecraft` when it names
    /// no namespace).
    pub fn texture_path(&self) -> String {
        let (namespace, path) = self
            .texture
            .split_once(':')
            .unwrap_or(("minecraft", &self.texture));
        format!("assets/{namespace}/{path}")
    }
}

/// The include a GPU particle's shader imports (`#moj_import <polytone:gpu_particle.glsl>`):
/// the type's knobs as one std140 block, laid out as [`gpu_block`] writes it.
pub const GPU_PARTICLE_GLSL: &str = "layout(std140) uniform PolytoneGpuParticle {\n\
    vec3 Origin;\n    float Time;\n\
    float Gravity;\n    float Drag;\n    float Sway;\n    float Spin;\n\
    float SizeEnd;\n    float Aspect;\n    float AlphaCutoff;\n    int Frames;\n\
    vec2 Fade;\n    int Billboard;\n    int RandomSprite;\n\
    int UseColorEnd;\n    int KillBelowHeightmap;\n    float CameraY;\n    float HzPad;\n\
    vec4 ColorEnd;\n\
    vec4 HeightmapInfo;\n\
};\n";

/// The include's id as a core shader names it.
pub const GPU_PARTICLE_INCLUDE: &str = "polytone:shaders/include/gpu_particle.glsl";

/// The block's name in [`GPU_PARTICLE_GLSL`], and its size.
pub const GPU_BLOCK: &str = "PolytoneGpuParticle";
pub const GPU_BLOCK_SIZE: usize = 112;

/// The heightmap a GPU particle's shader samples (`Sampler1`): `size` × `size` columns from
/// the world column `origin` (x, z), heights above `min_y`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeightmapArea {
    pub origin: [i32; 2],
    pub size: u32,
    pub min_y: i32,
}

/// The heightmap's heights on the CPU, for the GPU particle types drawn as quads that die under
/// it (`kill_below_heightmap`), and the snapshot a renderer uploads for the shaders that sample
/// it. Its heights always fill its area ([`Surface::new`]).
#[derive(Clone, Debug)]
pub struct Surface {
    area: HeightmapArea,
    /// `size` × `size` heights above `min_y`, x along a row and z down the rows.
    heights: std::sync::Arc<[u16]>,
}

/// Why heights can't make a [`Surface`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceSizeError {
    /// An area of no columns (`size` 0): nothing for a column to read.
    NoColumns,
    /// `heights` heights for an area of `size` × `size` columns.
    WrongCount { size: u32, heights: usize },
}

impl std::fmt::Display for SurfaceSizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::NoColumns => write!(f, "a heightmap area needs at least one column a side"),
            Self::WrongCount { size, heights } => write!(
                f,
                "a heightmap of {size} by {size} columns needs {} heights, not {heights}",
                u64::from(size).pow(2)
            ),
        }
    }
}

impl std::error::Error for SurfaceSizeError {}

impl Surface {
    /// `heights` over `area`: refused unless the area has columns and they are `size` × `size`.
    pub fn new(
        area: HeightmapArea,
        heights: std::sync::Arc<[u16]>,
    ) -> Result<Self, SurfaceSizeError> {
        if area.size == 0 {
            return Err(SurfaceSizeError::NoColumns);
        }
        if (area.size as usize).checked_pow(2) != Some(heights.len()) {
            return Err(SurfaceSizeError::WrongCount {
                size: area.size,
                heights: heights.len(),
            });
        }
        Ok(Self { area, heights })
    }

    pub fn area(&self) -> HeightmapArea {
        self.area
    }

    /// The heights, x along a row and z down the rows.
    pub fn heights(&self) -> &std::sync::Arc<[u16]> {
        &self.heights
    }

    /// Whether `at` is under the motion-blocking surface of its column; a column outside the
    /// area (any finite or infinite place) takes its nearest edge, as the shaders' clamped
    /// sampler does.
    pub fn covers(&self, at: [f64; 3]) -> bool {
        let size = self.area.size;
        // In floats: a place far outside saturates no integer, and NaN takes the first column.
        let last = f64::from(size - 1);
        let column =
            |v: f64, origin: i32| (v.floor() - f64::from(origin)).clamp(0.0, last) as usize;
        let (x, z) = (
            column(at[0], self.area.origin[0]),
            column(at[2], self.area.origin[1]),
        );
        let top = i64::from(self.area.min_y) + i64::from(self.heights[z * size as usize + x]);
        at[1] < top as f64
    }
}

/// What a frame adds to a type's knobs in its block.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlockFrame {
    /// 0 facing the camera, 1 around Y, 2 flat, 3 along its velocity.
    pub billboard: i32,
    pub random_sprite: bool,
    /// Where the records' positions start from, relative to the camera.
    pub origin: [f32; 3],
    /// The records' clock.
    pub time: f32,
    pub camera: [f64; 3],
    /// The heightmap `Sampler1` holds, placed relative to the camera in the block.
    pub heightmap: Option<HeightmapArea>,
}

/// The `polytone:gpu_particle.glsl` block for `gpu` drawn as `render_type` in `frame`.
pub fn gpu_block(gpu: &Gpu, render_type: RenderType, frame: &BlockFrame) -> [u8; GPU_BLOCK_SIZE] {
    let BlockFrame {
        billboard,
        random_sprite,
        origin,
        time,
        camera,
        heightmap,
    } = *frame;
    let mut floats = [0f32; GPU_BLOCK_SIZE / 4];
    let int = |v: i32| f32::from_bits(v as u32);
    floats[..3].copy_from_slice(&origin);
    floats[3] = time;
    floats[4] = gpu.gravity as f32;
    // Drag is `-ln(friction)`: none for a friction of 1 (or out of range).
    floats[5] = if gpu.friction > 0.0 && gpu.friction < 1.0 {
        -(gpu.friction.ln()) as f32
    } else {
        0.0
    };
    floats[6] = gpu.sway as f32;
    floats[7] = gpu.spin as f32;
    floats[8] = gpu.size_end.map_or(-1.0, |s| s as f32);
    floats[9] = gpu.aspect as f32;
    // Wiki "GPU Particles": the two translucent types blend, anything else is alpha cutout.
    floats[10] = match render_type {
        RenderType::Translucent | RenderType::AdditiveTranslucent => 0.0,
        _ => 0.1,
    };
    floats[11] = int(gpu.frames.max(1) as i32);
    floats[12] = gpu.fade[0] as f32;
    floats[13] = gpu.fade[1] as f32;
    floats[14] = int(billboard);
    floats[15] = int(i32::from(random_sprite));
    floats[16] = int(i32::from(gpu.color_end.is_some()));
    floats[17] = int(i32::from(gpu.kill_below_heightmap));
    floats[18] = camera[1] as f32;
    if let Some(end) = gpu.color_end {
        let channel = |shift: u32| ((end >> shift) & 0xff) as f32 / 255.0;
        floats[20..24].copy_from_slice(&[channel(16), channel(8), channel(0), channel(24)]);
    }
    if let Some(h) = heightmap {
        floats[24] = (f64::from(h.origin[0]) - camera[0]) as f32;
        floats[25] = (f64::from(h.origin[1]) - camera[2]) as f32;
        floats[26] = h.size as f32;
        floats[27] = h.min_y as f32;
    }
    let mut bytes = [0u8; GPU_BLOCK_SIZE];
    for (out, float) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(floats) {
        *out = float.to_le_bytes();
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn gpu(gravity: f64, friction: f64) -> Gpu {
        Gpu {
            texture: String::new(),
            frames: 1,
            gravity,
            friction,
            size_end: None,
            color_end: None,
            fade: [0.0; 2],
            sway: 0.0,
            spin: 0.0,
            aspect: 1.0,
            area: None,
            kill_below_heightmap: false,
        }
    }

    /// Heights that don't fill their area are refused when the surface is made, with the
    /// reason as a value, so its quads never check their lengths: an empty area, too few
    /// heights and too many.
    #[test]
    fn a_surface_is_refused_unless_its_heights_fill_its_area() {
        let area = |size| HeightmapArea {
            origin: [0, 0],
            size,
            min_y: 0,
        };
        let surface = |size, n: usize| Surface::new(area(size), vec![0u16; n].into());
        assert_eq!(surface(4, 16).unwrap().heights().len(), 16);
        assert_eq!(surface(4, 16).unwrap().area(), area(4));
        assert_eq!(surface(0, 0).unwrap_err(), SurfaceSizeError::NoColumns);
        assert_eq!(surface(0, 1).unwrap_err(), SurfaceSizeError::NoColumns);
        let wrong = |heights| SurfaceSizeError::WrongCount { size: 4, heights };
        assert_eq!(surface(4, 15).unwrap_err(), wrong(15));
        assert_eq!(surface(4, 17).unwrap_err(), wrong(17));
    }

    /// Rain dies at the surface of its own column (wiki "GPU Particles": the motion-blocking
    /// heightmap), not where the sky is shaded: in front of a hill it falls to the grass.
    #[test]
    fn the_surface_covers_what_is_below_its_column() {
        // A 4x4 area from (10, 20) above y -64: ground at y 64, the column (12, 21) a hill
        // up to 80.
        let mut heights = vec![128u16; 16];
        heights[4 + 2] = 144;
        let area = HeightmapArea {
            origin: [10, 20],
            size: 4,
            min_y: -64,
        };
        let surface = Surface::new(area, heights.into()).unwrap();
        assert!(!surface.covers([11.5, 64.0, 20.5]));
        assert!(surface.covers([11.5, 63.9, 20.5]));
        assert!(surface.covers([12.5, 79.0, 21.5]));
        assert!(!surface.covers([12.5, 80.5, 21.5]));
        // Beside the hill the drop still falls.
        assert!(!surface.covers([13.5, 70.0, 21.5]));
    }

    /// A place far outside the area takes the edge on its own side, however far: no
    /// overflow, and not the opposite edge.
    #[test]
    fn far_places_take_the_nearest_edge() {
        // Ground at y 10 along the first column (x 0), y 50 along the last (x 3), y 30 between.
        let heights: Vec<u16> = (0..16)
            .map(|i| match i % 4 {
                0 => 10,
                3 => 50,
                _ => 30,
            })
            .collect();
        let area = HeightmapArea {
            origin: [-2, -2],
            size: 4,
            min_y: 0,
        };
        let surface = Surface::new(area, heights.into()).unwrap();
        for x in [
            f64::MIN,
            -1.0e300,
            f64::NEG_INFINITY,
            f64::from(i32::MIN) - 5.0,
        ] {
            assert!(surface.covers([x, 9.0, 0.0]), "{x}");
            assert!(!surface.covers([x, 11.0, 0.0]), "{x}");
        }
        for x in [f64::MAX, 1.0e300, f64::INFINITY, f64::from(i32::MAX) + 5.0] {
            assert!(surface.covers([x, 49.0, 0.0]), "{x}");
            assert!(!surface.covers([x, 51.0, 0.0]), "{x}");
        }
        // The same along z, the rows.
        assert!(surface.covers([0.0, 29.0, f64::MAX]));
        assert!(surface.covers([0.0, 29.0, f64::MIN]));
    }

    /// The block as floats, as the shader reads it.
    fn floats(block: &[u8; GPU_BLOCK_SIZE]) -> Vec<f32> {
        block
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect()
    }

    /// Wiki "GPU Particles": `shader` names a core shader (`minecraft` without a namespace) and
    /// each of `uniforms` is an expression; one that doesn't compile is refused by name.
    #[test]
    fn a_gpu_particle_names_its_own_shader_and_uniforms() {
        let parse = |extra: Json| {
            let mut json = json!({"type": "gpu", "texture": "a:textures/particle/b.png"});
            json.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            CustomParticle::parse(&json).unwrap()
        };
        let shader = parse(json!({
            "shader": "my_pack:rain",
            "uniforms": {"Brightness": "0.5", "Wind": 2, "Bad": "1 +", "Flag": true}
        }))
        .gpu_shader()
        .unwrap();
        assert_eq!(shader.id(), "my_pack:rain");
        assert_eq!(
            shader.stage_path("fsh"),
            "assets/my_pack/shaders/core/rain.fsh"
        );
        let mut names: Vec<&str> = shader.uniforms.iter().map(|(n, _)| n.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["Brightness", "Wind"]);
        let mut refused: Vec<&str> = shader.refused.iter().map(|(n, _)| n.as_str()).collect();
        refused.sort_unstable();
        assert_eq!(refused, ["Bad", "Flag"]);
        assert_eq!(shader.stage_dir(), "my_pack:shaders/core/");
        let plain = parse(json!({"shader": "rain"})).gpu_shader().unwrap();
        assert_eq!(plain.id(), "minecraft:rain");
        let nested = parse(json!({"shader": "my_pack:weather/storm/rain"})).gpu_shader();
        let nested = nested.unwrap();
        assert_eq!(nested.stage_dir(), "my_pack:shaders/core/weather/storm/");
        assert_eq!(
            nested.stage_path("vsh"),
            "assets/my_pack/shaders/core/weather/storm/rain.vsh"
        );
        assert!(parse(json!({})).gpu_shader().is_none());
        // A custom (CPU) particle's `shader` is not a GPU particle's.
        let custom = CustomParticle::parse(&json!({"shader": "my_pack:rain"})).unwrap();
        assert!(custom.gpu_shader().is_none());
        let texture = parse(json!({})).gpu.unwrap().texture_path();
        assert_eq!(texture, "assets/a/textures/particle/b.png");
    }

    /// The `polytone:gpu_particle.glsl` block as its std140 layout reads it, each knob at its
    /// offset: drag `-ln(friction)`, `SizeEnd`, the ints as their bits, `ColorEnd` from ARGB, and
    /// `HeightmapInfo` the heightmap's corner relative to the camera, its size and bottom.
    #[test]
    fn the_shader_block_lays_out_the_types_knobs() {
        let knobs = Gpu {
            frames: 4,
            size_end: Some(0.2),
            color_end: Some(0x80FF_0000),
            fade: [0.25, 0.5],
            sway: 0.25,
            spin: 0.125,
            aspect: 2.0,
            kill_below_heightmap: true,
            ..gpu(0.04, 0.25)
        };
        let heightmap = HeightmapArea {
            origin: [-54, -68],
            size: 128,
            min_y: -64,
        };
        let block = gpu_block(
            &knobs,
            RenderType::Opaque,
            &BlockFrame {
                billboard: 3,
                random_sprite: true,
                origin: [-0.5, -0.25, -0.75],
                time: 12.5,
                camera: [10.5, 70.25, -3.75],
                heightmap: Some(heightmap),
            },
        );
        let f = floats(&block);
        let int = |i: usize| f[i].to_bits() as i32;
        assert_eq!(f[..5], [-0.5, -0.25, -0.75, 12.5, 0.04]);
        assert_eq!(f[5], 1.386_294_4);
        assert_eq!(f[6..11], [0.25, 0.125, 0.2, 2.0, 0.1]);
        assert_eq!(int(11), 4);
        assert_eq!(f[12..14], [0.25, 0.5]);
        assert_eq!([int(14), int(15), int(16), int(17)], [3, 1, 1, 1]);
        assert_eq!(f[18], 70.25);
        assert_eq!(f[20..24], [1.0, 0.0, 0.0, 128.0 / 255.0]);
        assert_eq!(f[24..28], [-64.5, -64.25, 128.0, -64.0]);
        // No end colour, size or heightmap: their defaults.
        let plain = gpu_block(&gpu(0.0, 1.0), RenderType::Opaque, &BlockFrame::default());
        let f = floats(&plain);
        assert_eq!([f[5], f[8], f[16].to_bits() as f32], [0.0, -1.0, 0.0]);
        assert_eq!(f[20..28], [0.0; 8]);
    }

    /// Wiki "GPU Particles": only the non-translucent types are alpha cutout. A translucent
    /// type's faint records (a storm's rain at alpha 0.05) must not fall under the cutoff a
    /// custom shader discards below.
    #[test]
    fn only_cutout_types_have_an_alpha_cutoff() {
        let cutoff = |render_type| {
            let block = gpu_block(&gpu(0.0, 1.0), render_type, &BlockFrame::default());
            floats(&block)[10]
        };
        assert_eq!(cutoff(RenderType::Translucent), 0.0);
        assert_eq!(cutoff(RenderType::AdditiveTranslucent), 0.0);
        assert_eq!(cutoff(RenderType::Opaque), 0.1);
    }
}
