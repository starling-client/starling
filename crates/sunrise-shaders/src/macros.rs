//! The macros a pack's programs and property files start with (spec 1.5, 1.5.1): `MC_VERSION`,
//! `MC_*`, the render stages, the biome categories and the renderer's. Defining and expanding
//! them is `sunrise-glsl`'s preprocessor's ([`sunrise_glsl::Macros`]), which this wraps.

pub(crate) use sunrise_glsl::macros::lex;

/// The feature flags of `iris.features.*` (spec 3.2) Sunrise has. A pack that requires one of
/// them loads; one it lacks is noted. Sunrise defines no `IRIS_FEATURE_<name>` macro for them
/// (spec OQ-2, docs/cleanroom/README.md): that macro family names another loader, so packs take
/// the path they take without it. `BLOCK_EMISSION_ATTRIBUTE`: terrain's
/// `at_midBlock.w` is the light its block gives. `SEPARATE_HARDWARE_SAMPLERS` (ShaderDoc,
/// "Separate Hardware Shadow Samplers"): `shadowtex0`/`shadowtex1` stay plain depth samplers when
/// hardware filtering is on, `shadowtex0HW`/`shadowtex1HW` compare; Sunrise picks a comparison
/// sampler by the declared type (`sampler2DShadow`), so they always are. `HIGHER_SHADOWCOLOR`:
/// `shadowcolor2` to `shadowcolor7` beside the first two. `FADE_VARIABLE`: terrain vertex
/// programs can read the section's visibility transition as `mc_chunkFade`.
const FEATURES: [&str; 9] = [
    "BLOCK_EMISSION_ATTRIBUTE",
    "COMPUTE_SHADERS",
    "CUSTOM_IMAGES",
    "ENTITY_TRANSLUCENT",
    "FADE_VARIABLE",
    "HIGHER_SHADOWCOLOR",
    "REVERSED_CULLING",
    "SEPARATE_HARDWARE_SAMPLERS",
    "SSBO",
];

/// Whether `name` is one of [`FEATURES`], in any case.
fn is_feature(name: &str) -> bool {
    FEATURES.iter().any(|f| f.eq_ignore_ascii_case(name))
}

/// `MC_VERSION` of the version Sunrise plays: 26.3 → 260300.
pub const MC_VERSION: u32 = mc_version(sunrise_core::version::VERSION_ID);

/// The documented rule, `major * 10000 + minor * 100 + patch` (spec 1.5.1, OQ-1): `1.21` → 12100,
/// `1.7.10` → 10710. Parts that are not digits count as 0.
pub const fn mc_version(id: &str) -> u32 {
    let bytes = id.as_bytes();
    let (mut parts, mut part, mut i) = ([0u32; 3], 0, 0);
    while i < bytes.len() && part < 3 {
        match bytes[i] {
            b'.' => part += 1,
            b @ b'0'..=b'9' => parts[part] = parts[part] * 10 + (b - b'0') as u32,
            _ => break,
        }
        i += 1;
    }
    parts[0] * 10000 + parts[1] * 100 + parts[2]
}

/// The macros at a point of a pack's file: `sunrise-glsl`'s, with the pack's own starting set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Macros(sunrise_glsl::Macros);

impl std::ops::Deref for Macros {
    type Target = sunrise_glsl::Macros;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Macros {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// The macro expansion work one file of a pack may spend ([`Macros::for_file`]), in the units
/// of [`sunrise_glsl::Macros::bound`] (for each token made, read or copied: 1, its bytes and the
/// names it hides): a stage file with its includes, a `.properties` file, the option values of
/// `shaders.properties`' sizes. As many as the tokens of work a core shader's directives may
/// spend ([`sunrise_glsl::MAX_SCANNED`]). A line of code is charged up to twice: telling whether
/// a call on it goes on past it expands it too ([`sunrise_glsl::Macros::join_call`]). A line of
/// expansions that reached the bound held 325 MiB at its peak (a release build); the ten packs
/// measured for #4135 spend at most 74,004 in a stage file and 104 in a `.properties` file, with
/// their default options.
pub const MAX_EXPANSION_WORK: usize = sunrise_glsl::MAX_SCANNED;

/// `MC_RENDER_STAGE_*` in the order the stages draw (spec 5.4); each is its index (OQ-7).
pub const RENDER_STAGES: [&str; 24] = [
    "NONE",
    "SKY",
    "SUNSET",
    "CUSTOM_SKY",
    "SUN",
    "MOON",
    "STARS",
    "VOID",
    "TERRAIN_SOLID",
    "TERRAIN_CUTOUT_MIPPED",
    "TERRAIN_CUTOUT",
    "ENTITIES",
    "BLOCK_ENTITIES",
    "DESTROY",
    "OUTLINE",
    "DEBUG",
    "HAND_SOLID",
    "TERRAIN_TRANSLUCENT",
    "TRIPWIRE",
    "PARTICLES",
    "CLOUDS",
    "RAIN_SNOW",
    "WORLD_BORDER",
    "HAND_TRANSLUCENT",
];

/// `DH_BLOCK_*`, each its index: Distant Sunrises' block materials.
pub const DH_BLOCK_MATERIALS: [&str; 16] = [
    "UNKNOWN",
    "LEAVES",
    "STONE",
    "WOOD",
    "METAL",
    "DIRT",
    "LAVA",
    "DEEPSLATE",
    "SNOW",
    "SAND",
    "TERRACOTTA",
    "NETHER_STONE",
    "WATER",
    "GRASS",
    "AIR",
    "ILLUMINATED",
];

/// The value of `MC_RENDER_STAGE_<name>` (OQ-7: its index in `RENDER_STAGES`).
pub fn render_stage(name: &str) -> u32 {
    RENDER_STAGES
        .iter()
        .position(|s| *s == name)
        .unwrap_or_else(|| panic!("no render stage {name}")) as u32
}

impl Macros {
    /// These macros, for one file of the pack: with [`MAX_EXPANSION_WORK`] to spend, whatever
    /// was spent before. Past it an expansion stops, named at its line by whoever reads the file.
    pub(crate) fn for_file(&self) -> Self {
        let mut m = self.clone();
        m.0.bound(MAX_EXPANSION_WORK);
        m
    }

    /// The standard macros that need no renderer (spec 1.5.1): `MC_VERSION`, this system's
    /// `MC_OS_*`, the GL and GLSL versions Iris reports, the render stages and the host
    /// settings at their defaults. The GPU vendor and family arrive with the renderer
    /// (`define_renderer`).
    pub fn standard() -> Self {
        let mut m = Self::default();
        m.define("MC_VERSION", MC_VERSION.to_string());
        let os = match std::env::consts::OS {
            "windows" => "MC_OS_WINDOWS",
            "macos" => "MC_OS_MAC",
            "linux" => "MC_OS_LINUX",
            _ => "MC_OS_OTHER",
        };
        m.define(os, "");
        // What Iris 1.11.7 reports on 26.3, whatever the pack's `#version` (observed with a probe
        // pack): its GL context's 3.3. Packs then take the paths they take there; the
        // translation itself reads GLSL up to 4.60.
        m.define("MC_GL_VERSION", "330");
        m.define("MC_GLSL_VERSION", "330");
        for (i, stage) in RENDER_STAGES.iter().enumerate() {
            m.define(format!("MC_RENDER_STAGE_{stage}"), i.to_string());
        }
        m.define("MC_RENDER_QUALITY", "1.0");
        m.define("MC_SHADOW_QUALITY", "1.0");
        m.define("MC_HAND_DEPTH", "0.125");
        m.define("MAX_COLOR_BUFFERS", "16");
        // The normal and specular maps are on (OptiFine's shaders.txt, H: their options, on by
        // default; Iris 1.11.7 defines both too, with or without a resource pack's maps).
        m.define("MC_NORMAL_MAP", "");
        m.define("MC_SPECULAR_MAP", "");
        // No `IS_IRIS`, `IRIS_VERSION` or `IRIS_HAS_*`: Sunrise is not that loader and does not
        // claim its identity (spec OQ-2, docs/cleanroom/README.md). Packs take their other path.
        // Distant Sunrises' block materials, which Iris defines with or without the mod (packs
        // compare ids with them unguarded: Exposa's `blockID == DH_BLOCK_LEAVES`), numbered as
        // the mod's `EDhApiBlockMaterial` numbers them.
        for (i, material) in DH_BLOCK_MATERIALS.iter().enumerate() {
            m.define(format!("DH_BLOCK_{material}"), i.to_string());
        }
        // The biome categories and precipitations the `biome_category` and
        // `biome_precipitation` uniforms compare with, as custom uniforms see them (Iris's
        // replacements define them for the stage files too: Sildur's Enhanced Default reads
        // `biome_category == CAT_THE_END`). `BIOME_*` needs the world's registry.
        for (i, category) in crate::biomes::CATEGORIES.iter().enumerate() {
            m.define(format!("CAT_{category}"), i.to_string());
        }
        for (i, precipitation) in ["NONE", "RAIN", "SNOW"].iter().enumerate() {
            m.define(format!("PPT_{precipitation}"), i.to_string());
        }
        m
    }

    /// The macros the renderer adds: the mipmap levels terrain is drawn with (`MC_MIPMAP_LEVEL`,
    /// Iris's: it follows the Mipmap Levels option), and `OTHER` as the GL vendor and renderer
    /// (OptiFine's shaders.txt, E and F: one of each is defined). Sunrise draws through wgpu, not
    /// a GL driver, so a pack's paths for one driver (an NVIDIA extension it takes behind
    /// `MC_GL_VENDOR_NVIDIA`) are not there to take.
    pub fn define_renderer(&mut self, mipmap_levels: u32) {
        self.define("MC_GL_VENDOR_OTHER", "");
        self.define("MC_GL_RENDERER_OTHER", "");
        self.define("MC_MIPMAP_LEVEL", mipmap_levels.to_string());
    }

    /// The `MC_GL_KHR_shader_subgroup_*` macros of a device with subgroup operations
    /// (`wgpu::Features::SUBGROUP`): the GL extensions the translator then takes.
    pub fn define_subgroups(&mut self) {
        for ext in [
            "basic",
            "vote",
            "arithmetic",
            "ballot",
            "shuffle",
            "shuffle_relative",
        ] {
            self.define(format!("MC_GL_KHR_shader_subgroup_{ext}"), "");
        }
    }

    /// A note for each feature `iris.features.required` names that Sunrise lacks; a name matches
    /// in any case. Optional features need nothing: no macro is defined for them (see `FEATURES`).
    pub fn feature_notes(properties: &crate::Properties) -> Vec<String> {
        properties
            .get("iris.features.required")
            .unwrap_or_default()
            .split_whitespace()
            .filter(|f| !is_feature(f))
            .map(|f| format!("the pack requires `{f}`, which is not supported yet"))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether `fsh` translates with `m`: a program that declares what it reads only under
    /// a macro's condition translates when the condition holds.
    fn translates(fsh: &str, m: &Macros) -> Result<(), String> {
        let vsh = "#version 330\nvoid main() { gl_Position = vec4(0.0); }\n";
        let pack = crate::Pack::from_zip_bytes(
            "P.zip",
            crate::pack::tests::zip(&[
                ("shaders/composite.vsh", vsh),
                ("shaders/composite.fsh", fsh),
            ]),
        )
        .unwrap();
        crate::translate_composite(
            &crate::preprocess(&pack, "composite.vsh", m),
            &crate::preprocess(&pack, "composite.fsh", m),
            crate::Target::Buffers,
            naga::front::glsl::InterfaceLimits::WGPU_DEFAULT,
        )
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
    }

    /// Packs see GL and GLSL 3.3, and no other loader's identity (spec OQ-2): a declaration made
    /// only under that condition is seen.
    #[test]
    fn programs_see_gl_33_and_no_other_loader() {
        let fsh = "#version 330\nout vec4 c;\n\
                   #if MC_GL_VERSION == 330 && MC_GLSL_VERSION == 330 && !defined IS_IRIS \\\n\
                   && !defined IRIS_VERSION && !defined IRIS_HAS_TRANSLUCENCY_SORTING\n\
                   const float v = 1.0;\n#endif\n\
                   void main() { c = vec4(v); }\n";
        translates(fsh, &Macros::standard()).unwrap();
    }

    /// With the renderer's macros a pack sees one GL vendor and renderer, `OTHER`, and the
    /// mipmap levels terrain is drawn with.
    #[test]
    fn programs_see_the_renderers_vendor_and_mipmap_levels() {
        let fsh = "#version 330\nout vec4 c;\n\
                   #if defined MC_GL_VENDOR_OTHER && defined MC_GL_RENDERER_OTHER && MC_MIPMAP_LEVEL == 2\n\
                   const float v = 1.0;\n#endif\n\
                   #if defined MC_GL_VENDOR_NVIDIA || defined MC_GL_RENDERER_GEFORCE\nconst float v = 2.0;\n#endif\n\
                   void main() { c = vec4(v); }\n";
        let mut m = Macros::standard();
        m.define_renderer(2);
        translates(fsh, &m).unwrap();
    }

    /// Packs take their normal- and specular-map branches, as they do with the maps on: a
    /// declaration made only there is seen.
    #[test]
    fn programs_take_the_normal_and_specular_map_branches() {
        let fsh = "#version 330\nout vec4 c;\n\
                   #ifdef MC_NORMAL_MAP\nconst float bump = 1.0;\n#endif\n\
                   #ifdef MC_SPECULAR_MAP\nconst float shine = 2.0;\n#endif\n\
                   void main() { c = vec4(bump, shine, 0.0, 1.0); }\n";
        translates(fsh, &Macros::standard()).unwrap();
    }

    /// A required feature Sunrise has loads without a note, one it lacks gets a note, whatever the
    /// case it is written in; no `IRIS_FEATURE_*` macro is defined either way (spec OQ-2).
    #[test]
    fn required_features_are_noted_and_never_defined() {
        let properties = crate::Properties::parse(
            "iris.features.optional = COMPUTE_SHADERS ssbo NOT_A_FEATURE\n\
             iris.features.required = BLOCK_EMISSION_ATTRIBUTE HIGHER_SHADOWCOLOR custom_images \
             Not_A_Feature\n",
            &Macros::standard(),
        );
        assert_eq!(
            Macros::feature_notes(&properties),
            ["the pack requires `Not_A_Feature`, which is not supported yet"]
        );
        let mut m = Macros::standard();
        m.define_renderer(4);
        let iris: Vec<_> = m.names().filter(|name| name.contains("IRIS")).collect();
        assert!(iris.is_empty(), "{iris:?}");
    }

    #[test]
    fn versions_follow_the_documented_rule() {
        assert_eq!(mc_version("1.21"), 12100);
        assert_eq!(mc_version("1.21.0"), 12100);
        assert_eq!(mc_version("1.7.10"), 10710);
        assert_eq!(mc_version("26.3"), 260300);
        assert_eq!(MC_VERSION, 260300);
        assert_eq!(Macros::standard().get("MC_VERSION"), Some("260300"));
    }

    #[test]
    fn distant_sunrises_block_materials_are_defined_without_the_mod() {
        let m = Macros::standard();
        assert_eq!(m.get("DH_BLOCK_UNKNOWN"), Some("0"));
        assert_eq!(m.get("DH_BLOCK_LEAVES"), Some("1"));
        assert_eq!(m.get("DH_BLOCK_WATER"), Some("12"));
        assert_eq!(m.get("DH_BLOCK_ILLUMINATED"), Some("15"));
        assert_eq!(m.get("DISTANT_SUNRISES"), None);
    }

    #[test]
    fn biome_categories_and_precipitations_are_macros_as_custom_uniforms_see_them() {
        let m = Macros::standard();
        let constants = crate::biomes::constants(&[]);
        for (name, value) in &constants {
            assert_eq!(m.get(name), Some(value.to_string().as_str()), "{name}");
        }
        assert_eq!(m.get("CAT_THE_END"), Some("8"));
        assert_eq!(m.get("PPT_SNOW"), Some("2"));
    }
}
