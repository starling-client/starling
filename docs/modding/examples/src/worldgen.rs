//! Load-resolved worldgen values, shared by native and generated worker interfaces.

pub mod terrain {
    // ANCHOR: terrain
    use mod_api::{
        BlockKey, ChunkAccess, ChunkGenerator, DimensionSettings, Hooks, Registrar,
        RegistrationContext,
    };

    struct Floor {
        stone: BlockKey,
    }
    impl ChunkGenerator for Floor {
        fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
            let stone = chunk.state(self.stone);
            let [x, z] = [chunk.pos().x * 16, chunk.pos().z * 16];
            for dx in 0..16 {
                for dz in 0..16 {
                    chunk.set_block([x + dx, chunk.min_y(), z + dz], stone);
                }
            }
        }
    }
    #[derive(Default)]
    pub struct TerrainMod;
    #[mod_api::sunrise_mod]
    impl Hooks for TerrainMod {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            let stone = registrar.block_key("minecraft:stone");
            registrar.register_chunk_generator("floor", Floor { stone });
            registrar.register_dimension(
                "floor",
                DimensionSettings {
                    generator: "mymod:floor".into(),
                    ..Default::default()
                },
            );
        }
    }
    // ANCHOR_END: terrain
}

pub mod biome_source {
    // ANCHOR: biome_source
    use mod_api::{
        BiomeKey, BiomeSource, DimensionSettings, Hooks, Registrar, RegistrationContext,
    };

    /// A random biome in each 5x5 square of quart cells (20 blocks wide).
    struct Chaos;
    impl BiomeSource for Chaos {
        fn possible_biomes(&self) -> Vec<BiomeKey> {
            vec![
                BiomeKey::parse("minecraft:desert").unwrap(),
                BiomeKey::parse("minecraft:jungle").unwrap(),
                BiomeKey::parse("minecraft:snowy_plains").unwrap(),
            ]
        }
        fn noise_biome(&self, [x, _, z]: [i32; 3], seed: i64) -> u16 {
            let cell = (i64::from(x.div_euclid(5)) << 32) ^ i64::from(z.div_euclid(5)) ^ seed;
            (cell.wrapping_mul(6364136223846793005) >> 48) as u16
        }
    }
    #[derive(Default)]
    pub struct ChaosMod;
    #[mod_api::sunrise_mod]
    impl Hooks for ChaosMod {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            registrar.register_biome_source("chaos", Chaos);
            registrar.register_dimension(
                "chaos",
                DimensionSettings {
                    mod_biome_source: Some("mymod:chaos".into()),
                    ..Default::default()
                },
            );
        }
    }
    // ANCHOR_END: biome_source
}

pub mod structures {
    // ANCHOR: structure
    use mod_api::{
        BlockKey, BlockMetadata, BlockState, BoundingBox, Compound, FoundStructure,
        GenerationContext, Hooks, PieceContext, Registrar, RegistrationContext, StateConstraint,
        StructureError, StructurePiece, StructureType, Tag,
    };
    use std::sync::Arc;

    const MATERIAL: &str = "minecraft:oak_log";
    struct Tower {
        material: BlockKey,
    }
    #[derive(Debug)]
    struct Pillar {
        min: [i32; 3],
        max: [i32; 3],
        state: BlockState,
        axis: String,
    }

    impl StructureType for Tower {
        fn find_generation_point(
            &self,
            context: &mut dyn GenerationContext,
        ) -> Option<FoundStructure> {
            let state = BlockState::of(
                context.metadata(),
                self.material,
                &[StateConstraint::new("axis", "y")],
            )
            .ok()?;
            let min = [
                context.chunk().x * 16,
                context.sea_level(),
                context.chunk().z * 16,
            ];
            Some(FoundStructure {
                position: min,
                pool_pieces: Vec::new(),
                pieces: vec![Arc::new(Pillar {
                    min,
                    max: [min[0], min[1] + 4, min[2]],
                    state,
                    axis: "y".into(),
                })],
            })
        }
        fn load_piece(
            &self,
            metadata: &dyn BlockMetadata,
            tag: &Compound,
        ) -> Result<Arc<dyn StructurePiece>, StructureError> {
            let block = saved_text(tag, "block")?;
            if metadata.name(self.material).map_err(StructureError::new)? != block {
                return Err(StructureError::new(std::io::Error::other(
                    "saved block does not match this pillar's material",
                )));
            }
            let axis = saved_text(tag, "axis")?;
            let state = BlockState::of(
                metadata,
                self.material,
                &[StateConstraint::new("axis", &axis)],
            )
            .map_err(StructureError::new)?;
            Ok(Arc::new(Pillar {
                min: saved_position(tag, "min")?,
                max: saved_position(tag, "max")?,
                state,
                axis,
            }))
        }
    }
    impl StructurePiece for Pillar {
        fn bounding_box(&self) -> BoundingBox {
            BoundingBox::new(self.min, self.max)
        }
        fn post_process(&self, context: &mut dyn PieceContext) -> Result<(), StructureError> {
            for y in self.min[1]..=self.max[1] {
                context.set_block([self.min[0], y, self.min[2]], self.state);
            }
            Ok(())
        }
        fn add_additional_save_data(&self, tag: &mut Compound) {
            tag.insert("min".into(), Tag::IntArray(self.min.to_vec()));
            tag.insert("max".into(), Tag::IntArray(self.max.to_vec()));
            tag.insert("block".into(), Tag::String(MATERIAL.into()));
            tag.insert("axis".into(), Tag::String(self.axis.as_str().into()));
        }
    }
    fn saved_text(tag: &Compound, key: &str) -> Result<String, StructureError> {
        match tag.get(&key.into()) {
            Some(Tag::String(value)) => value.to_string().map_err(StructureError::new),
            _ => Err(StructureError::new(std::io::Error::other(format!(
                "missing or invalid saved {key}"
            )))),
        }
    }
    fn saved_position(tag: &Compound, key: &str) -> Result<[i32; 3], StructureError> {
        match tag.get(&key.into()) {
            Some(Tag::IntArray(values)) if values.len() == 3 => {
                Ok([values[0], values[1], values[2]])
            }
            _ => Err(StructureError::new(std::io::Error::other(format!(
                "missing or invalid saved {key}"
            )))),
        }
    }
    #[derive(Default)]
    pub struct StructureMod;
    #[mod_api::sunrise_mod]
    impl Hooks for StructureMod {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            let material = registrar.block_key(MATERIAL);
            registrar.register_structure_type("tower", Tower { material });
        }
    }
    // ANCHOR_END: structure
}

pub mod features {
    // ANCHOR: feature
    use mod_api::{
        BlockKey, BlockState, BlockUpdateFlags, DatapackField, DatapackFieldKind, DatapackValue,
        FeatureContext, FeatureError, FeatureType, Hooks, Registrar, RegistrationContext,
        StateConstraint, TagKey,
    };

    /// A pillar of logs on dirt or grass: `{"type": "mymod:pillar", "height": 4}`.
    struct Pillar {
        log: BlockKey,
        dirt: TagKey,
    }

    impl FeatureType for Pillar {
        fn place(
            &self,
            context: &mut dyn FeatureContext,
            origin: [i32; 3],
        ) -> Result<bool, FeatureError> {
            // Decoded against the schema at load: present, an integer, 1 to 32.
            let Some(DatapackValue::Int(height)) = context.config().get("height").cloned() else {
                return Err(FeatureError::new(std::io::Error::other(
                    "height was not decoded",
                )));
            };
            let below = [origin[0], origin[1] - 1, origin[2]];
            if context.block_in_tag(below, self.dirt) != Some(true) {
                return Ok(false);
            }
            let state = BlockState::of(
                context.metadata(),
                self.log,
                &[StateConstraint::new("axis", "y")],
            )
            .map_err(FeatureError::new)?;
            let flags = BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS;
            let mut placed = false;
            for dy in 0..height as i32 {
                let pos = [origin[0], origin[1] + dy, origin[2]];
                if !context.ensure_can_write(pos)
                    || !context
                        .block(pos)
                        .is_some_and(|block| context.is_air(block))
                {
                    // Outside the writable region or blocked: completed writes stay.
                    return Ok(placed);
                }
                if !context.set_block(pos, state, flags) {
                    return Ok(placed);
                }
                placed = true;
            }
            Ok(placed)
        }
    }

    #[derive(Default)]
    pub struct FeatureMod;
    #[mod_api::sunrise_mod]
    impl Hooks for FeatureMod {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            let log = registrar.block_key("minecraft:oak_log");
            let dirt = registrar.tag_key("minecraft:dirt");
            let config = vec![DatapackField::with_default(
                "height",
                DatapackFieldKind::Int { min: 1, max: 32 },
                DatapackValue::Int(4),
            )];
            registrar.register_feature_type("pillar", config, Pillar { log, dirt });
        }
    }
    // ANCHOR_END: feature
}
