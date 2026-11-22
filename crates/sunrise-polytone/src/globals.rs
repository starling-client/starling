//! Polytone's globals (wiki "Extra Shader Uniforms"): the `PolyGlobals` block a pack's post and
//! core shaders may read, and the built-in `polytone_globals.glsl` for a pack that imports it
//! without shipping it. Written from the public wiki's member names and their meaning.
//!
//! The built-in file and its helpers follow the wiki. Polytone 26.3 (6.8.6, the only build for
//! 26.3) has no such file, so a pack importing it loads here and not in that Java build; a pack
//! declaring the block itself reads the same values in both (its pixels within the spread a
//! control pack without the block shows between the two). Polytone 6.11 on 26.2 has the file
//! (`#moj_import`), and gives the same `PolyPlayerBlockPos`, `PolyPlayerOffset`, `PolySunAngle`,
//! `PolyDeltaTime` and projection form in the same scene. Its `polyPositionFromDepth` takes the
//! depth as OpenGL's (`depth * 2 - 1`), which on 26.3's reverse-Z 0 to 1 depth buffer gives
//! other positions; this one follows the wiki's contract (the position from the screen uv and the
//! depth buffer's value) and gives the geometric hit. Its `PolyDayTime` read 6001 where this
//! gives 6000 (the sun angle 6000's in both); that difference is not matched.

/// The block's name.
pub const BLOCK: &str = "PolyGlobals";
/// Its std140 size: two `mat4`, three floats, an `ivec3` and a `vec3`.
pub const BLOCK_SIZE: usize = 176;
/// The include's file name; `<polytone_globals.glsl>` reads it from `shaders/include/`.
pub const INCLUDE_FILE: &str = "polytone_globals.glsl";

/// `polytone_globals.glsl` as the wiki describes it: the block and its two helpers (see the module
/// notes: no counterpart in Polytone 26.3). 26.3's reverse-Z projection keeps depth from 0 to 1,
/// so `polyPositionFromDepth` takes a depth read from the depth buffer as the clip-space z.
pub const GLSL: &str = "#ifndef POLYTONE_GLOBALS_GLSL
#define POLYTONE_GLOBALS_GLSL
layout(std140) uniform PolyGlobals {
    mat4 PolyProjMat;
    mat4 PolyModelViewMat;
    float PolySunAngle;
    float PolyDayTime;
    float PolyDeltaTime;
    ivec3 PolyPlayerBlockPos;
    vec3 PolyPlayerOffset;
};
vec3 polyPlayerPos() {
    return vec3(PolyPlayerBlockPos) - PolyPlayerOffset;
}
vec3 polyPositionFromDepth(vec2 uv, float depth) {
    vec4 view = inverse(PolyProjMat) * vec4(uv * 2.0 - 1.0, depth, 1.0);
    return (inverse(PolyModelViewMat) * vec4(view.xyz / view.w, 1.0)).xyz;
}
#endif
";

/// A `PolyGlobals` member's GLSL type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Member {
    Mat4,
    Float,
    IVec3,
    Vec3,
}

/// The block's members in order, each with its std140 offset: what [`Globals::std140`] writes,
/// and what a shader's declaration of the block must match (a leading part of it, at least).
pub const LAYOUT: [(&str, Member, usize); 7] = [
    ("PolyProjMat", Member::Mat4, 0),
    ("PolyModelViewMat", Member::Mat4, 64),
    ("PolySunAngle", Member::Float, 128),
    ("PolyDayTime", Member::Float, 132),
    ("PolyDeltaTime", Member::Float, 136),
    ("PolyPlayerBlockPos", Member::IVec3, 144),
    ("PolyPlayerOffset", Member::Vec3, 160),
];

/// Whether an include id (`ns:shaders/include/<path>`) names the built-in file.
pub fn is_include(id: &str) -> bool {
    id.strip_suffix(INCLUDE_FILE)
        .is_some_and(|rest| rest.ends_with(":shaders/include/"))
}

/// What `PolyGlobals` holds this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Globals {
    /// The world's projection and model-view (the camera's turn), column major.
    pub projection: [f32; 16],
    pub model_view: [f32; 16],
    /// The sun's angle in radians, 0 at the horizon.
    pub sun_angle: f32,
    /// The time of day in ticks (0 to 24000), and the game ticks this frame advanced.
    pub day_time: f32,
    pub delta_time: f32,
    /// The player's feet: `PolyPlayerBlockPos`, and `PolyPlayerOffset` as vanilla's
    /// `CameraOffset` is (the block position less the position, so `polyPlayerPos()` is the
    /// block position less the offset).
    pub player: [f64; 3],
}

impl Globals {
    /// The std140 block: each member written at its [`LAYOUT`] offset.
    pub fn std140(&self) -> [u8; BLOCK_SIZE] {
        let mut out = [0u8; BLOCK_SIZE];
        let mut put = |member: usize, words: &[[u8; 4]]| {
            let at = LAYOUT[member].2;
            for (i, word) in words.iter().enumerate() {
                out[at + 4 * i..at + 4 * i + 4].copy_from_slice(word);
            }
        };
        let block = self.player.map(f64::floor);
        put(0, &self.projection.map(f32::to_le_bytes));
        put(1, &self.model_view.map(f32::to_le_bytes));
        put(2, &[self.sun_angle.to_le_bytes()]);
        put(3, &[self.day_time.to_le_bytes()]);
        put(4, &[self.delta_time.to_le_bytes()]);
        // A float saturates into the int, NaN at 0.
        put(5, &block.map(|b| (b as i32).to_le_bytes()));
        put(
            6,
            &std::array::from_fn::<_, 3, _>(|i| ((block[i] - self.player[i]) as f32).to_le_bytes()),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The block's std140 layout: matrices at 0 and 64, the three floats from 128, the block
    /// position at 144 and the offset at 160 (the block position less the position).
    #[test]
    fn the_globals_follow_std140() {
        let mut projection = [0.0; 16];
        projection[0] = 2.0;
        let g = Globals {
            projection,
            sun_angle: 1.5,
            day_time: 6000.0,
            delta_time: 0.5,
            player: [10.25, -3.5, 7.0],
            ..Default::default()
        };
        let b = g.std140();
        let f = |at: usize| f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let i = |at: usize| i32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        assert_eq!(f(0), 2.0);
        assert_eq!((f(128), f(132), f(136), f(140)), (1.5, 6000.0, 0.5, 0.0));
        assert_eq!((i(144), i(148), i(152)), (10, -4, 7));
        assert_eq!((f(160), f(164), f(168)), (-0.25, -0.5, 0.0));
        // A position past an int's range saturates.
        let far = Globals {
            player: [1e300, f64::NAN, -1e300],
            ..Default::default()
        }
        .std140();
        let i = |at: usize| i32::from_le_bytes(far[at..at + 4].try_into().unwrap());
        assert_eq!((i(144), i(148), i(152)), (i32::MAX, 0, i32::MIN));
    }

    /// The writer follows [`LAYOUT`]: each member, given its own value, is found at its LAYOUT
    /// offset, in the order the wiki lists them.
    #[test]
    fn the_writer_follows_the_layout() {
        let names: Vec<&str> = LAYOUT.iter().map(|(name, ..)| *name).collect();
        assert_eq!(
            names,
            [
                "PolyProjMat",
                "PolyModelViewMat",
                "PolySunAngle",
                "PolyDayTime",
                "PolyDeltaTime",
                "PolyPlayerBlockPos",
                "PolyPlayerOffset"
            ]
        );
        let mut projection = [0.0; 16];
        (projection[0], projection[15]) = (11.0, 1.0);
        let mut model_view = [0.0; 16];
        (model_view[0], model_view[15]) = (12.0, 2.0);
        let bytes = Globals {
            projection,
            model_view,
            sun_angle: 3.0,
            day_time: 4.0,
            delta_time: 5.0,
            player: [6.5, 7.5, 8.5],
        }
        .std140();
        let word = |at: usize| <[u8; 4]>::try_from(&bytes[at..at + 4]).unwrap();
        let float = |at| f32::from_le_bytes(word(at));
        let int = |at| i32::from_le_bytes(word(at));
        // Each member at its std140 offset (literal, not LAYOUT's), its first and last word.
        assert_eq!((float(0), float(60)), (11.0, 1.0));
        assert_eq!((float(64), float(124)), (12.0, 2.0));
        assert_eq!((float(128), float(132), float(136)), (3.0, 4.0, 5.0));
        assert_eq!((int(144), int(152)), (6, 8));
        assert_eq!((float(160), float(168)), (-0.5, -0.5));
        let offsets: Vec<usize> = LAYOUT.iter().map(|(.., at)| *at).collect();
        assert_eq!(offsets, [0, 64, 128, 132, 136, 144, 160]);
    }

    #[test]
    fn the_include_is_named_in_any_namespace() {
        assert!(is_include(
            "minecraft:shaders/include/polytone_globals.glsl"
        ));
        assert!(is_include("polytone:shaders/include/polytone_globals.glsl"));
        assert!(!is_include("minecraft:shaders/include/fog.glsl"));
        assert!(!is_include("minecraft:shaders/core/polytone_globals.glsl"));
    }
}
