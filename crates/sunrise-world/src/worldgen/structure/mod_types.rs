//! Structure types a mod registers (vanilla's `StructureType` with its own
//! `Structure.findGenerationPoint` and `StructurePiece`s), handed over once at
//! startup and looked up by `StructureRegistry::new` before `Unsupported`. The
//! game keeps the piece mechanisms it has: a mod type's own pieces are legacy
//! pieces behind an adapter (`Piece::Legacy`), so the start cache, the saving,
//! the cross-chunk placement and the beardifier treat them like a fortress
//! piece's, and the pieces it assembles from template pools through
//! [`StructureInputs::assemble_jigsaw`] are jigsaw pieces (`Piece::Pool`), saved
//! and loaded as `minecraft:jigsaw` pieces are. Runs on the worldgen worker
//! threads, so a type keeps no mutable state and its contexts carry no `Host`.
use super::jigsaw::{Jigsaw, JigsawConfig, PoolElement, PoolPiece, Pools, assemble, attach};
use super::legacy::LegacyPiece;
use super::start::{Piece, StartContext};
use crate::worldgen::block_feature::FeatureBlockBindings;
use mod_api::JigsawSpace;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use sunrise_core::nbt::{Compound, Tag};
use sunrise_core::{
    position::ChunkPos,
    random::{LegacyRandom, RandomSource, WorldgenRandom},
};

/// What a structure type's start builds from: the bundle's templates and
/// processor lists, and its template pools for a jigsaw assembly.
pub struct StructureInputs<'a> {
    pub resources: &'a super::StructureResources,
    pools: &'a Pools,
    bindings: &'a FeatureBlockBindings<'a>,
}
impl<'a> StructureInputs<'a> {
    pub fn new(
        resources: &'a super::StructureResources,
        pools: &'a Pools,
        bindings: &'a FeatureBlockBindings<'a>,
    ) -> Self {
        Self {
            resources,
            pools,
            bindings,
        }
    }
    /// The bundle's template pool `name`: `None` for one it lacks or one left
    /// out because an element failed to decode.
    pub fn pool(&self, name: &str) -> Option<Arc<super::jigsaw::Pool>> {
        self.pools.get(name, self.resources, self.bindings)
    }
    /// `JigsawPlacement.addPieces` from the bundle's template pools with the
    /// type's own decoded settings, as a NeoForge structure type calls it from
    /// its own `findGenerationPoint`: [`assemble`] with `config`'s depth and
    /// reach as given. `JigsawStructure`'s codec bounds (`size` 0 to 20,
    /// `max_distance_from_center` 1 to 128 across) belong to the
    /// `minecraft:jigsaw` type and are not applied here; every placement check
    /// of the assembly itself is. The pieces are jigsaw pieces, saved and
    /// loaded as `minecraft:jigsaw` pieces are, so the type's own loader never
    /// sees them.
    pub fn assemble_jigsaw(
        &self,
        config: &JigsawConfig<'_>,
        mut context: &mut dyn StartContext,
        random: &mut impl RandomSource,
        position: [i32; 3],
    ) -> Option<([i32; 3], Vec<Piece>)> {
        let (position, pieces) = assemble(
            config,
            &mut context,
            self.pools,
            self.resources,
            self.bindings,
            random,
            position,
        )?;
        Some((position, pieces.into_iter().map(Piece::Pool).collect()))
    }
    /// One candidate on one jigsaw of `source`, as a structure type laying
    /// its own jigsaw out calls it: [`attach`] with the bundle's pools looked
    /// up by name, without aliases.
    #[allow(clippy::too_many_arguments)]
    pub fn attach(
        &self,
        source: &mut PoolPiece,
        jigsaw: &Jigsaw<'_>,
        target: &Arc<PoolElement>,
        space: &mut JigsawSpace,
        use_expansion_hack: bool,
        mut context: &mut dyn StartContext,
        random: &mut impl RandomSource,
    ) -> Option<PoolPiece> {
        attach(
            source,
            jigsaw,
            target,
            space,
            use_expansion_hack,
            &|name| self.pool(name),
            self.resources,
            &mut None,
            &mut context,
            random,
        )
    }
}

/// A structure type's engine side: vanilla's `StructureType` bound to the
/// pieces' `StructurePieceType` loader, as the executable's adapter sees it.
/// `Send + Sync` because neighbouring chunks build at once on worker threads;
/// the same seed gives the same pieces.
pub trait ModStructureType: Send + Sync {
    /// Prepare a definition once at load. The game owns the JSON-to-NBT boundary.
    fn prepare_config(&self, config: &serde_json::Value) -> Result<Arc<Tag>, String>;
    /// `Structure.findGenerationPoint` with `generatePieces`: the start
    /// position and at least one piece for `chunk`, or none for "no start
    /// here". The placement checks stay the game's own: the set's spacing, the
    /// structure's biomes and its `terrain_adaptation`. A type that lays its
    /// start out from template pools calls [`StructureInputs::assemble_jigsaw`].
    fn find_generation_point(
        &self,
        context: &mut dyn StartContext,
        inputs: &StructureInputs<'_>,
        chunk: ChunkPos,
        // Converted once and retained by the loaded structure definition.
        config: &Arc<Tag>,
        // Seeded as vanilla seeds it for the chunk (`setLargeFeatureSeed`).
        random: &mut WorldgenRandom<LegacyRandom>,
    ) -> Option<([i32; 3], Vec<Piece>)>;
    /// The `StructurePieceType` loader (`StructurePieceType.load`): a saved
    /// piece back exactly as it was saved. There is no rebuild-from-seed
    /// fallback: every piece type has its loader.
    fn load_piece(
        &self,
        registry: &crate::registry::BlockRegistry,
        tag: &Compound,
    ) -> Option<Box<dyn LegacyPiece>>;
}

/// The installed types, `modid:tower` → type. Set once, before any world opens
/// (the executable hands the mods' registrations over), and read by
/// `StructureRegistry::new` while decoding a bundle's structures.
pub type ModStructureTypes = BTreeMap<String, Arc<dyn ModStructureType>>;

static MOD_TYPES: OnceLock<ModStructureTypes> = OnceLock::new();

#[cfg(test)]
thread_local! {
    /// The types a test installs, in place of the process-wide table.
    static TEST_TYPES: std::cell::Cell<Option<&'static ModStructureTypes>> =
        const { std::cell::Cell::new(None) };
}

/// Install the process-wide table once; a second installation keeps the first.
pub fn install_mod_structure_types(types: ModStructureTypes) -> Result<(), ModStructureTypes> {
    MOD_TYPES.set(types)
}

/// The installed structure types: empty until the executable installs the
/// mods', and a test's own while one is installed.
pub fn mod_structure_types() -> &'static ModStructureTypes {
    #[cfg(test)]
    if let Some(types) = TEST_TYPES.get() {
        return types;
    }
    MOD_TYPES.get().unwrap_or(&EMPTY)
}

static EMPTY: ModStructureTypes = BTreeMap::new();

#[cfg(test)]
/// Runs `with` over a test's own types, as the registry reads them.
pub fn with_test_types<T>(types: &ModStructureTypes, with: impl FnOnce() -> T) -> T {
    let leaked: &'static ModStructureTypes = Box::leak(Box::new(types.clone()));
    let prev = TEST_TYPES.replace(Some(leaked));
    let out = with();
    TEST_TYPES.set(prev);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::structure::legacy::{PieceContext, Result};
    use crate::worldgen::structure::start::StructureKind;
    use serde_json::json;
    use std::collections::BTreeSet;
    use sunrise_core::bounding_box::BoundingBox;

    /// A type whose every piece is its box: positions it got, in a tag.
    struct Boxed(Vec<[i32; 3]>);

    impl ModStructureType for Boxed {
        fn prepare_config(
            &self,
            config: &serde_json::Value,
        ) -> std::result::Result<Arc<Tag>, String> {
            sunrise_core::json_nbt::json_to_tag(config)
                .map(Arc::new)
                .map_err(|error| error.to_string())
        }
        fn find_generation_point(
            &self,
            _context: &mut dyn StartContext,
            _inputs: &StructureInputs<'_>,
            chunk: ChunkPos,
            _config: &Arc<Tag>,
            _random: &mut WorldgenRandom<LegacyRandom>,
        ) -> Option<([i32; 3], Vec<Piece>)> {
            let origin = [chunk.x * 16, 64, chunk.z * 16];
            let pieces = self
                .0
                .iter()
                .map(|&offset| {
                    Piece::Legacy(Box::new(SavedPiece {
                        id: "testmod:boxed".to_owned(),
                        min: offset,
                        max: [offset[0] + 1, offset[1] + 1, offset[2] + 1],
                    }))
                })
                .collect();
            Some((origin, pieces))
        }
        fn load_piece(
            &self,
            _registry: &crate::registry::BlockRegistry,
            tag: &Compound,
        ) -> Option<Box<dyn LegacyPiece>> {
            let int_array = |key: &str| -> Option<[i32; 3]> {
                let value = tag.get(&key.into())?;
                let sunrise_core::nbt::Tag::IntArray(a) = value else {
                    return None;
                };
                Some([a[0], a[1], a[2]])
            };
            Some(Box::new(SavedPiece {
                id: "testmod:boxed".to_owned(),
                min: int_array("Min")?,
                max: int_array("Max")?,
            }))
        }
    }

    /// The adapter's saved piece shape: a box that saves and loads back.
    struct SavedPiece {
        id: String,
        min: [i32; 3],
        max: [i32; 3],
    }
    impl LegacyPiece for SavedPiece {
        fn bounding_box(&self) -> BoundingBox {
            BoundingBox::new(self.min, self.max)
        }
        fn type_name(&self) -> &'static str {
            Box::leak(self.id.clone().into_boxed_str())
        }
        fn save(&self) -> Option<Compound> {
            use sunrise_core::nbt::Tag;
            let mut tag = super::super::legacy::piece_tag(&self.id, self.bounding_box(), None, 0);
            tag.insert("Min".into(), Tag::IntArray(self.min.to_vec()));
            tag.insert("Max".into(), Tag::IntArray(self.max.to_vec()));
            Some(tag)
        }
        fn post_process(&self, _: &mut PieceContext<'_>) -> Result<()> {
            Ok(())
        }
    }
    impl std::fmt::Debug for SavedPiece {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("SavedPiece").field("id", &self.id).finish()
        }
    }

    /// A bundle structure whose `"type"` a registered mod type answers, keeps
    /// the structure's JSON config and rebuilds its pieces by it.
    #[test]
    fn registry_resolves_a_mod_type_and_keeps_its_config() {
        use super::super::start::StructureRegistry;
        let types: ModStructureTypes = BTreeMap::from([(
            "testmod:boxed".to_owned(),
            Arc::new(Boxed(vec![[0, 0, 0], [8, 0, 8]])) as Arc<dyn ModStructureType>,
        )]);
        let structures = BTreeMap::from([(
            "testmod:tower".to_owned(),
            json!({"type": "testmod:boxed", "biomes": "#minecraft:is_forest"}).to_string(),
        )]);
        let registry = with_test_types(&types, || {
            StructureRegistry::new(
                &structures,
                &BTreeMap::new(),
                BTreeMap::new(),
                &|_| Ok(BTreeSet::new()),
                &types,
            )
        })
        .unwrap();
        assert_eq!(registry.structures.len(), 1);
        let kind = &registry.structures[0].kind;
        assert!(matches!(
            kind,
            StructureKind::Mod(m) if m.config.get("type") == Some(&Tag::String("testmod:boxed".into()))
        ));
    }

    /// An unknown `"type"` stays `Unsupported`, placed by no one.
    #[test]
    fn unknown_type_stays_unsupported() {
        use super::super::start::StructureRegistry;
        let structures = BTreeMap::from([(
            "testmod:tower".to_owned(),
            json!({"type": "testmod:gone", "biomes": "#minecraft:is_forest"}).to_string(),
        )]);
        let registry = StructureRegistry::new(
            &structures,
            &BTreeMap::new(),
            BTreeMap::new(),
            &|_| Ok(BTreeSet::new()),
            &ModStructureTypes::new(),
        )
        .unwrap();
        assert!(matches!(
            registry.structures[0].kind,
            StructureKind::Unsupported(ref id) if id == "testmod:gone"
        ));
    }

    /// A type that lays its start out from template pools at its own `size`.
    struct Deep;
    impl ModStructureType for Deep {
        fn prepare_config(
            &self,
            config: &serde_json::Value,
        ) -> std::result::Result<Arc<Tag>, String> {
            sunrise_core::json_nbt::json_to_tag(config)
                .map(Arc::new)
                .map_err(|error| error.to_string())
        }
        fn find_generation_point(
            &self,
            context: &mut dyn StartContext,
            inputs: &StructureInputs<'_>,
            chunk: ChunkPos,
            config: &Arc<Tag>,
            random: &mut WorldgenRandom<LegacyRandom>,
        ) -> Option<([i32; 3], Vec<Piece>)> {
            // JsonOps reads a small whole number as a byte.
            let size = match config.get("size")? {
                Tag::Byte(size) => i32::from(*size),
                Tag::Short(size) => i32::from(*size),
                Tag::Int(size) => *size,
                _ => return None,
            };
            let aliases = BTreeMap::new();
            let jigsaw = JigsawConfig {
                start_pool: "test:links",
                start_jigsaw: None,
                max_depth: size,
                use_expansion_hack: false,
                project_to: None,
                max_distance: [16, 400],
                aliases: &aliases,
                padding: [0, 0],
                waterlogging: true,
            };
            inputs.assemble_jigsaw(&jigsaw, context, random, [chunk.x * 16, 0, chunk.z * 16])
        }
        fn load_piece(
            &self,
            _registry: &crate::registry::BlockRegistry,
            _tag: &Compound,
        ) -> Option<Box<dyn LegacyPiece>> {
            None
        }
    }

    /// A flat column from -64 to 320 with every biome valid: a start needs nothing else here.
    struct Flat;
    impl super::super::jigsaw::JigsawContext for Flat {
        fn min_y(&self) -> i32 {
            -64
        }
        fn max_y(&self) -> i32 {
            320
        }
        fn base_height(&mut self, _: i32, _: i32, _: crate::heightmap::HeightmapType) -> i32 {
            64
        }
        fn column_has_valid_biome(&mut self, _: i32, _: i32, _: i32, _: i32) -> bool {
            true
        }
    }
    impl StartContext for Flat {
        fn seed(&self) -> i64 {
            0
        }
        fn sea_level(&self) -> i32 {
            63
        }
        fn base_column(&mut self, _: i32, _: i32, _: crate::heightmap::HeightmapType) -> Vec<bool> {
            unreachable!("a pool chain without projection reads no column")
        }
        fn base_columns_by(
            &mut self,
            _: i32,
            _: i32,
            _: usize,
            _: &dyn Fn(usize) -> crate::heightmap::HeightmapType,
        ) -> Vec<Vec<bool>> {
            unreachable!("a pool chain without projection reads no column")
        }
        fn noise_column_into(
            &mut self,
            _: i32,
            _: i32,
            _: &mut dyn super::super::start::NoiseColumnTarget,
        ) {
            unreachable!("a pool chain reads no noise")
        }
        fn cold_enough_to_snow(&mut self, _: crate::worldgen::biome::BiomeId, _: [i32; 3]) -> bool {
            unreachable!("a pool chain asks no biome")
        }
        fn generation(&self) -> crate::worldgen::anchor::WorldGenerationContext {
            crate::worldgen::anchor::WorldGenerationContext::new(-64, 384, 63)
        }
        fn biome_key(&self, _: crate::worldgen::biome::BiomeId) -> mod_api::BiomeKey {
            unreachable!("a pool chain asks no biome")
        }
        fn noise_biome(&mut self, _: [i32; 3]) -> crate::worldgen::biome::BiomeId {
            unreachable!("a pool chain asks no biome")
        }
        fn noise_biomes(&mut self, _: &[[i32; 3]]) -> Vec<crate::worldgen::biome::BiomeId> {
            unreachable!("a pool chain asks no biome")
        }
        fn set_valid_biomes(&mut self, _: Option<&BTreeSet<crate::worldgen::biome::BiomeId>>) {}
    }

    /// `JigsawStructure`'s codec bounds `size` to 0..=20 for `minecraft:jigsaw` only: a mod's
    /// own structure type assembles from the same pools at its own depth. One link three
    /// blocks tall, a jigsaw at its foot facing down and one at its head facing up, stacks
    /// 46 deep above the start: 47 pieces, each on the one before.
    #[test]
    fn a_mod_type_assembles_past_the_jigsaw_types_depth_bound() {
        use super::super::{
            StateTransforms, StructureResources,
            start::{StructureKind, StructureRegistry},
            template::{BlockInfo, save_nbt},
        };
        use crate::registry::{BlockRegistry, BlockStateId};
        let states = [
            ("minecraft:air", None),
            ("minecraft:stone", None),
            ("minecraft:jigsaw", Some("down_south")),
            ("minecraft:jigsaw", Some("up_north")),
            ("minecraft:cave_air", None),
            ("minecraft:void_air", None),
        ];
        let registry: &'static BlockRegistry = Box::leak(Box::new(
            BlockRegistry::from_nbt(&Tag::compound([
                ("DataVersion", Tag::Int(crate::DATA_VERSION)),
                (
                    "states",
                    Tag::List(
                        states
                            .iter()
                            .map(|(name, orientation)| match orientation {
                                None => Tag::compound([("id", Tag::String((*name).into()))]),
                                Some(o) => Tag::compound([
                                    ("id", Tag::String((*name).into())),
                                    (
                                        "properties",
                                        Tag::compound([("orientation", Tag::String((*o).into()))]),
                                    ),
                                ]),
                            })
                            .collect(),
                    ),
                ),
                (
                    "defaults",
                    Tag::compound([
                        ("minecraft:air", Tag::Int(0)),
                        ("minecraft:stone", Tag::Int(1)),
                        ("minecraft:jigsaw", Tag::Int(2)),
                        ("minecraft:cave_air", Tag::Int(4)),
                        ("minecraft:void_air", Tag::Int(5)),
                    ]),
                ),
            ]))
            .unwrap(),
        ));
        let transforms = || {
            StateTransforms::from_json(
                r#"{"rotate":[[0,0,0,0],[1,1,1,1],[2,2,2,2],[3,3,3,3],[4,4,4,4],[5,5,5,5]],"mirror":[[0,0],[1,1],[2,2],[3,3],[4,4],[5,5]],"full_block":[false,true,false,false,false,false]}"#,
                6,
            )
            .unwrap()
        };
        let jigsaw = |state: u32, pool: &str, name: &str, target: &str, y: i32| BlockInfo {
            pos: [0, y, 0],
            state: BlockStateId(state),
            nbt: Some(Box::new(Compound::from_iter([
                ("id".into(), Tag::String("minecraft:jigsaw".into())),
                ("pool".into(), Tag::String(pool.into())),
                ("name".into(), Tag::String(name.into())),
                ("target".into(), Tag::String(target.into())),
                ("final_state".into(), Tag::String("minecraft:air".into())),
            ]))),
        };
        let link = save_nbt(
            registry,
            &transforms(),
            [1, 3, 1],
            vec![
                jigsaw(2, "minecraft:empty", "test:foot", "minecraft:empty", 0),
                BlockInfo {
                    pos: [0, 1, 0],
                    state: BlockStateId(1),
                    nbt: None,
                },
                jigsaw(3, "test:links", "minecraft:empty", "test:foot", 2),
            ],
            &[],
            crate::DATA_VERSION,
        );
        let raw = sunrise_core::nbt::write_named(
            &sunrise_core::nbt::NamedTag {
                name: "".into(),
                tag: link,
            },
            sunrise_core::nbt::Limits::templates(),
        )
        .unwrap();
        let file = crate::compression::Compression::Gzip.encode(&raw).unwrap();
        let resources = StructureResources::new(
            registry,
            transforms(),
            std::path::Path::new("/nonexistent"),
            BTreeMap::new(),
            &BTreeMap::new(),
        )
        .with_pack_templates([("test:link".to_owned(), file.into())].into());
        let pools = Pools::new(BTreeMap::from([
            (
                "minecraft:empty".to_owned(),
                json!({"fallback": "minecraft:empty", "elements": []}).to_string(),
            ),
            (
                "test:links".to_owned(),
                json!({"fallback": "minecraft:empty", "elements": [{"weight": 1, "element": {
                "element_type": "minecraft:single_pool_element", "location": "test:link",
                "projection": "rigid", "processors": {"processors": []}}}]})
                .to_string(),
            ),
        ]));
        let bindings = FeatureBlockBindings::new(registry, BTreeSet::new()).unwrap();
        let inputs = StructureInputs::new(&resources, &pools, &bindings);

        let config = Arc::new(sunrise_core::json_nbt::json_to_tag(&json!({"size": 46})).unwrap());
        let chunk = ChunkPos::new(0, 0);
        let mut random = WorldgenRandom::new(LegacyRandom::new(0));
        let (_, pieces) = Deep
            .find_generation_point(&mut Flat, &inputs, chunk, &config, &mut random)
            .expect("a start from the pool");
        assert_eq!(pieces.len(), 47);
        let bottoms: Vec<i32> = pieces.iter().map(|p| p.bounding_box().min[1]).collect();
        let first = bottoms[0];
        assert_eq!(bottoms, (0..47).map(|i| first + 3 * i).collect::<Vec<_>>());
        assert!(pieces.iter().all(|p| matches!(p, Piece::Pool(_))));

        // `JigsawPlacement.addPieces` adds no piece at depth 0, the start's included.
        let aliases = BTreeMap::new();
        let mut settings = JigsawConfig {
            start_pool: "test:links",
            start_jigsaw: None,
            max_depth: 0,
            use_expansion_hack: false,
            project_to: None,
            max_distance: [16, 400],
            aliases: &aliases,
            padding: [0, 0],
            waterlogging: true,
        };
        let mut random = WorldgenRandom::new(LegacyRandom::new(0));
        let (_, none) = inputs
            .assemble_jigsaw(&settings, &mut Flat, &mut random, [0, 0, 0])
            .expect("the start fits");
        assert!(none.is_empty());
        // A type's position, reach and padding are its own: Java's int arithmetic wraps at the
        // extremes.
        for (position, max_distance, padding, expansion) in [
            ([0, 0, 0], [i32::MAX, i32::MAX], [0, 0], false),
            ([0, 0, 0], [1, 1], [i32::MIN, i32::MAX], false),
            ([0, 0, 0], [i32::MAX, i32::MAX], [i32::MAX, i32::MIN], false),
            ([i32::MAX; 3], [i32::MAX, i32::MAX], [0, 0], true),
            ([i32::MIN; 3], [i32::MAX, i32::MAX], [0, 0], true),
            ([i32::MAX, i32::MIN, i32::MAX], [16, 400], [0, 0], true),
        ] {
            settings.max_depth = 2;
            settings.max_distance = max_distance;
            settings.padding = padding;
            settings.use_expansion_hack = expansion;
            let mut random = WorldgenRandom::new(LegacyRandom::new(0));
            let _ = inputs.assemble_jigsaw(&settings, &mut Flat, &mut random, position);
        }

        // The same structure as `minecraft:jigsaw` is past its codec's bound and refused.
        let structures = BTreeMap::from([(
            "test:deep".to_owned(),
            json!({"type": "minecraft:jigsaw", "start_pool": "test:links", "size": 46,
                "start_height": {"absolute": 0}, "max_distance_from_center": 16,
                "biomes": "#minecraft:is_forest"})
            .to_string(),
        )]);
        let refused = StructureRegistry::new(
            &structures,
            &BTreeMap::new(),
            BTreeMap::new(),
            &|_| Ok(BTreeSet::new()),
            &ModStructureTypes::new(),
        )
        .unwrap();
        assert!(matches!(
            refused.structures[0].kind,
            StructureKind::Unsupported(ref id) if id == "minecraft:jigsaw"
        ));
    }

    /// A piece a type's loader rebuilds comes back with its box and its
    /// type id, as it was saved.
    #[test]
    fn load_piece_rebuilds_the_saved_piece() {
        use sunrise_core::nbt::Tag;
        let types: ModStructureTypes = BTreeMap::from([(
            "testmod:boxed".to_owned(),
            Arc::new(Boxed(vec![])) as Arc<dyn ModStructureType>,
        )]);
        let ty = &types["testmod:boxed"];
        let mut tag = super::super::legacy::piece_tag(
            "testmod:boxed",
            BoundingBox::new([1, 64, 2], [3, 66, 4]),
            None,
            0,
        );
        tag.insert("Min".into(), Tag::IntArray(vec![1, 64, 2]));
        tag.insert("Max".into(), Tag::IntArray(vec![3, 66, 4]));
        let registry = crate::registry::BlockRegistry::from_facts_json(
            br#"{"states":[["air",true,false,"","minecraft:air"]],"defaults":{"minecraft:air":0}}"#
                .as_slice(),
        )
        .unwrap();
        let piece = ty.load_piece(&registry, &tag).unwrap();
        assert_eq!(
            piece.bounding_box(),
            BoundingBox::new([1, 64, 2], [3, 66, 4])
        );
        assert_eq!(piece.type_name(), "testmod:boxed");
    }
}
