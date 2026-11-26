//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;
use sunrise_core::level_id::valid_level_id;

impl PartRegistrar {
    /// Refuses a dimension whose generator or mod biome source no one provides.
    /// After every mod has registered, as either may come from a later mod's
    /// registration.
    pub(crate) fn refuse_unknown_generators_and_biome_sources(&mut self) {
        for dimension in &self.dimensions {
            let generator = &dimension.settings.generator;
            if generator != mod_vocab::__private::NOISE_GENERATOR
                && !self
                    .chunk_generators
                    .iter()
                    .any(|g| g.id.as_str() == generator.as_str())
            {
                self.refused.push(format!(
                    "{} registered dimension {} with the unknown generator {generator:?}",
                    dimension.mod_id, dimension.id
                ));
            }
            if let Some(clock) = &dimension.settings.clock
                && !VANILLA_CLOCKS.contains(&clock.as_str())
                && !self.world_clocks.iter().any(|c| **c == **clock)
            {
                self.refused.push(format!(
                    "{} registered dimension {} with the unknown world clock {clock:?}",
                    dimension.mod_id, dimension.id
                ));
            }
            if let Some(source) = &dimension.settings.mod_biome_source
                && !self.biome_sources.iter().any(|s| *s.id == **source)
            {
                self.refused.push(format!(
                    "{} registered dimension {} with the unknown biome source {source:?}",
                    dimension.mod_id, dimension.id
                ));
            }
        }
    }
}

/// The world clocks vanilla's registry has (`WorldClocks`).
const VANILLA_CLOCKS: [&str; 2] = ["minecraft:overworld", "minecraft:the_end"];

impl DimensionsRegistrar for PartRegistrar {
    fn add_world_clock(&mut self, name: &str) {
        let id = match self.declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        let refusal = if !valid_level_id(&id) {
            Some(format!(
                "{} registered world clock {id:?}, whose name must be lowercase letters, digits                  and _ . - with / between segments, none empty, `.` or `..`",
                self.mod_id
            ))
        } else if VANILLA_CLOCKS.contains(&id.as_str()) {
            Some(format!(
                "{} registered world clock {id}, which is a vanilla clock",
                self.mod_id
            ))
        } else if self.world_clocks.iter().any(|c| **c == *id) {
            Some(format!("{} registered world clock {id} twice", self.mod_id))
        } else {
            None
        };
        match refusal {
            Some(refusal) => self.refused.push(refusal),
            None => self.world_clocks.push(id.into()),
        }
    }

    fn add_dimension(&mut self, name: &str, settings: crate::DimensionSettings) {
        let id = match self.declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        let refusal = if !valid_level_id(&id) {
            Some(format!(
                "{} registered dimension {id:?}, whose name must be lowercase letters, digits and \
                 _ . - with / between segments, none empty, `.` or `..`: it names a folder of the world",
                self.mod_id
            ))
        } else if VANILLA_DIMENSIONS.contains(&id.as_str()) {
            Some(format!(
                "{} registered dimension {id}, which is a vanilla level",
                self.mod_id
            ))
        } else if self.dimensions.iter().any(|d| *d.id == *id) {
            Some(format!("{} registered dimension {id} twice", self.mod_id))
        } else if !(self.dimension_shape)(&settings) {
            Some(format!(
                "{} registered dimension {id} with a build height the game refuses: min_y {}, \
                 height {}, logical_height {}",
                self.mod_id, settings.min_y, settings.height, settings.logical_height
            ))
        } else if let Some(why) = settings
            .invalid_biome_source()
            .or_else(|| settings.invalid_field())
        {
            Some(format!(
                "{} registered dimension {id} with a dimension type the game refuses: {why}",
                self.mod_id
            ))
        } else {
            None
        };
        if let Some(refusal) = refusal {
            self.refused.push(refusal);
            return;
        }
        self.dimensions.push(DimensionInfo {
            mod_id: self.mod_id,
            id: id.into(),
            settings,
        });
    }
    fn add_chunk_generator(&mut self, name: &str, generator: Arc<dyn crate::ChunkGenerator>) {
        let name = qualified(self.mod_id, name);
        let Some(id) = crate::Identifier::parse_namespaced(&name) else {
            self.refused.push(format!(
                "mod {} declares {}",
                self.mod_id,
                DeclarationError::InvalidId { id: name }
            ));
            return;
        };
        // Identifier already checked the resource location; also require safe folder components.
        let refusal = if id.namespace() == "."
            || id
                .path()
                .split('/')
                .any(|part| matches!(part, "" | "." | ".."))
        {
            Some(format!(
                "{} registered chunk generator {:?}, whose name must be lowercase letters, \
                 digits and _ . - with / between segments, none empty, `.` or `..`",
                self.mod_id,
                id.as_str()
            ))
        } else if VANILLA_CHUNK_GENERATORS.contains(&id.as_str())
            || self.has_builtin_entry("minecraft:worldgen/chunk_generator", id.as_str())
        {
            Some(format!(
                "{} registered chunk generator {id}, which is the game's own",
                self.mod_id
            ))
        } else {
            self.chunk_generators
                .iter()
                .any(|g| g.id == id)
                .then(|| format!("{} registered chunk generator {id} twice", self.mod_id))
        };
        if let Some(refusal) = refusal {
            self.refused.push(refusal);
            return;
        }
        self.chunk_generators.push(ChunkGeneratorInfo {
            id,
            // ChunkGenerator.vanilla_stages, asked once.
            stages: generator.vanilla_stages(),
            generator,
        });
    }
    fn add_biome_source(&mut self, name: &str, source: Arc<dyn crate::BiomeSource>) {
        let id = match self.declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        // BiomeSource.possibleBiomes, asked once.
        let biomes = source.possible_biomes();
        let refusal = if !valid_level_id(&id) {
            Some(format!(
                "{} registered biome source {id:?}, whose name must be lowercase letters, \
                 digits and _ . - with / between segments, none empty, `.` or `..`",
                self.mod_id
            ))
        } else if VANILLA_BIOME_SOURCES.contains(&id.as_str())
            || self.has_builtin_entry("minecraft:worldgen/biome_source", &id)
        {
            Some(format!(
                "{} registered biome source {id}, which is the game's own",
                self.mod_id
            ))
        } else if self.biome_sources.iter().any(|s| *s.id == *id) {
            Some(format!(
                "{} registered biome source {id} twice",
                self.mod_id
            ))
        } else if biomes.is_empty() {
            Some(format!(
                "{} registered biome source {id}, whose possible_biomes lists no biome",
                self.mod_id
            ))
        } else {
            None
        };
        if let Some(refusal) = refusal {
            self.refused.push(refusal);
            return;
        }
        self.biome_sources.push(BiomeSourceInfo {
            id: id.into(),
            biomes,
            source,
            owner: mod_vocab::__private::game::CallbackOwner::for_mod(self.mod_id),
        });
    }
}

/// The levels every world has, which no mod's can be.
const VANILLA_DIMENSIONS: [&str; 3] = [
    "minecraft:overworld",
    "minecraft:the_nether",
    "minecraft:the_end",
];

/// Built-in chunk generators (`BuiltInRegistries.CHUNK_GENERATOR`), which no mod can replace.
const VANILLA_CHUNK_GENERATORS: [&str; 3] = [
    mod_vocab::__private::NOISE_GENERATOR,
    "minecraft:flat",
    "minecraft:debug",
];

/// Built-in biome sources (`BuiltInRegistries.BIOME_SOURCE`), which no mod can replace.
const VANILLA_BIOME_SOURCES: [&str; 4] = [
    "minecraft:fixed",
    "minecraft:multi_noise",
    "minecraft:checkerboard",
    "minecraft:the_end",
];

#[cfg(test)]
mod tests {
    use super::*;

    struct Void;
    impl crate::ChunkGenerator for Void {
        fn build_terrain(&self, _: &mut dyn crate::ChunkAccess) {}
    }

    struct Plains;
    impl crate::BiomeSource for Plains {
        fn possible_biomes(&self) -> Vec<crate::BiomeKey> {
            vec![crate::BiomeKey::parse("minecraft:plains").unwrap()]
        }
        fn noise_biome(&self, _: [i32; 3], _: i64) -> u16 {
            0
        }
    }

    #[test]
    fn dimensions_generators_and_biome_sources_allow_explicit_namespaces_and_refuse_builtins() {
        let data = mod_vocab::__private::RegistrationData::new(
            [
                (
                    "minecraft:worldgen/chunk_generator".into(),
                    vec!["minecraft:noise".into(), "minecraft:snapshot_gen".into()],
                ),
                (
                    "minecraft:worldgen/biome_source".into(),
                    vec!["minecraft:fixed".into(), "minecraft:snapshot_source".into()],
                ),
            ]
            .into(),
            Default::default(),
            crate::InstanceDirectory::from_path(std::path::Path::new("unused")).unwrap(),
            crate::RuntimeEnvironment::new(
                crate::Dist::Client,
                "26.3".into(),
                "test".into(),
                false,
            ),
        );
        crate::load_state::with_test_registration_data(data, || {
            let mut r = PartRegistrar::new("maker");
            r.dimension_shape = |_| true;

            r.add_dimension("pocket", crate::DimensionSettings::default());
            r.add_dimension("other:pocket", crate::DimensionSettings::default());
            r.add_dimension("minecraft:custom_dim", crate::DimensionSettings::default());
            r.add_dimension("minecraft:overworld", crate::DimensionSettings::default());
            r.add_dimension("other:pocket", crate::DimensionSettings::default());

            r.add_chunk_generator("void", Arc::new(Void));
            r.add_chunk_generator("other:void", Arc::new(Void));
            r.add_chunk_generator("minecraft:custom_gen", Arc::new(Void));
            for builtin in [
                "minecraft:noise",
                "minecraft:flat",
                "minecraft:debug",
                "minecraft:snapshot_gen",
            ] {
                r.add_chunk_generator(builtin, Arc::new(Void));
            }
            r.add_chunk_generator("other:void", Arc::new(Void));

            r.add_biome_source("chaos", Arc::new(Plains));
            r.add_biome_source("other:chaos", Arc::new(Plains));
            r.add_biome_source("minecraft:custom_source", Arc::new(Plains));
            for builtin in [
                "minecraft:fixed",
                "minecraft:multi_noise",
                "minecraft:checkerboard",
                "minecraft:the_end",
                "minecraft:snapshot_source",
            ] {
                r.add_biome_source(builtin, Arc::new(Plains));
            }
            r.add_biome_source("other:chaos", Arc::new(Plains));

            assert_eq!(
                r.dimensions
                    .iter()
                    .map(|d| (d.mod_id, &*d.id))
                    .collect::<Vec<_>>(),
                [
                    ("maker", "maker:pocket"),
                    ("maker", "other:pocket"),
                    ("maker", "minecraft:custom_dim"),
                ]
            );
            assert_eq!(
                r.chunk_generators
                    .iter()
                    .map(|g| g.id.as_str())
                    .collect::<Vec<_>>(),
                ["maker:void", "other:void", "minecraft:custom_gen"]
            );
            assert_eq!(
                r.biome_sources.iter().map(|s| &*s.id).collect::<Vec<_>>(),
                ["maker:chaos", "other:chaos", "minecraft:custom_source"]
            );
            assert_eq!(r.refused.len(), 13, "{:?}", r.refused);
            assert!(r.refused.iter().all(|msg| msg.contains("maker")));

            for invalid in ["", ":void", "other:Bad"] {
                r.add_chunk_generator(invalid, Arc::new(Void));
                assert!(r.refused.last().unwrap().contains("invalid identifier"));
            }
            for unsafe_folder in [".:void", "other:a//b", "other:a/./b", "other:a/../b"] {
                r.add_chunk_generator(unsafe_folder, Arc::new(Void));
                assert!(
                    r.refused
                        .last()
                        .unwrap()
                        .contains("with / between segments")
                );
            }
            assert_eq!(r.chunk_generators.len(), 3);
        });
    }
}
